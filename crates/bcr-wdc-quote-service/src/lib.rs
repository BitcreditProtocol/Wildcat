// ----- standard library imports
use std::sync::Arc;
// ----- extra library imports
use axum::{
    extract::FromRef,
    routing::{delete, get, patch, post},
    Router,
};
use bcr_common::client::{
    core::Client as CoreClient, ebill::Client as EBillClient, quote::Client as QuoteClient,
    Url as ClientUrl,
};
use bcr_wdc_utils::surreal;
// ----- local modules
mod admin;
mod client;
mod error;
mod persistence;
mod quotes;
mod service;
mod web;
// ----- local imports

// ----- end imports

type TStamp = chrono::DateTime<chrono::Utc>;

pub type ProdQuoteRepository = persistence::surreal::DBQuotes;

pub type ProdQuotingService = service::Service;

#[derive(Clone, Debug, serde::Deserialize)]
pub struct AppConfig {
    quotes: surreal::DBConnConfig,
    core_url: ClientUrl,
    ebill_url: ClientUrl,
    clowder_rest_url: reqwest::Url,
}

#[derive(Clone, FromRef)]
pub struct AppController {
    quote: Arc<ProdQuotingService>,
}

impl AppController {
    pub async fn new(cfg: AppConfig) -> Self {
        let AppConfig {
            quotes,
            core_url,
            ebill_url,
            clowder_rest_url,
        } = cfg;
        let quotes_repository = ProdQuoteRepository::new(quotes)
            .await
            .expect("DB connection to quotes failed");

        let clwdr_cl = clwdr_client::ClowderRestClient::new(clowder_rest_url);
        let public_key = clwdr_cl
            .get_info()
            .await
            .expect("Failed to get Clowder ID")
            .node_id;
        let clwdr_client::model::MintUrlResponse { mint_url, .. } = clwdr_cl
            .get_mint_url(*public_key)
            .await
            .expect("Failed to get mint URL");
        let core_cl = CoreClient::new(core_url);
        let ebill_cl = EBillClient::new(ebill_url);
        let wdc_cl = client::WildcatCl {
            core: core_cl,
            ebill: ebill_cl,
        };
        let quoting_service = ProdQuotingService {
            wdc_client: Box::new(wdc_cl),
            quotes: Box::new(quotes_repository),
            mint_url,
        };

        Self {
            quote: Arc::new(quoting_service),
        }
    }
}
pub fn routes<Cntrlr>(ctrl: Cntrlr) -> Router
where
    Arc<service::Service>: FromRef<Cntrlr> + Send + Sync + 'static,
    Cntrlr: Send + Sync + Clone + 'static,
{
    let user_routes = Router::new()
        .route("/health", get(get_health))
        .route(QuoteClient::ENQUIRE_EP_V1, post(web::enquire_quote))
        .route(QuoteClient::LOOKUP_EP_V1, get(web::lookup_quote))
        .route(QuoteClient::RESOLVE_EP_V1, delete(web::cancel))
        .route(QuoteClient::RESOLVE_EP_V1, patch(web::resolve_offer));

    let admin_routes = Router::new()
        .route(QuoteClient::LIST_EP_V1, get(admin::list_quotes))
        .route("/v1/admin/credit/quote/{qid}", get(admin::lookup_quote))
        .route(QuoteClient::UPDATE_EP_V1, patch(admin::update_quote))
        .route(
            QuoteClient::ENABLE_MINTING_EP_V1,
            patch(admin::enable_minting),
        );

    Router::new()
        .merge(user_routes)
        .merge(admin_routes)
        .with_state(ctrl)
}

async fn get_health() -> &'static str {
    "{ \"status\": \"OK\" }"
}
