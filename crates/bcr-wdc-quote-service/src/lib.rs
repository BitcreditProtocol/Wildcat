// ----- standard library imports
use std::sync::Arc;
// ----- extra library imports
use axum::{
    extract::FromRef,
    routing::{delete, get, patch, post},
    Router,
};
use bcr_common::{
    client::{
        core::Client as CoreClient, ebill::Client as EBillClient, mint::Client as MintClient,
        quote::Client as QuoteClient, treasury::Client as TreasuryClient, Url as ClientUrl,
    },
    clwdr_client,
};
use bcr_wdc_utils::{routine::RoutineHandle, surreal};
// ----- local modules
mod admin;
mod client;
mod error;
mod monitor;
mod persistence;
mod quotes;
mod service;
mod web;
// ----- local imports

// ----- end imports

type TStamp = chrono::DateTime<chrono::Utc>;

pub const MINIMUM_MONITOR_INTERVAL_SECONDS: u64 = 5;
pub type ProdQuoteRepository = persistence::surreal::DBQuotes;
pub type ProdQuotingService = service::Service;

#[derive(Clone, Debug, serde::Deserialize)]
pub struct AppConfig {
    quotes: surreal::DBConnConfig,
    core_url: ClientUrl,
    treasury_url: ClientUrl,
    ebill_url: ClientUrl,
    clowder_rest_url: reqwest::Url,
    monitor_interval_seconds: u64,
}

#[derive(Clone, FromRef)]
pub struct AppController {
    quote: Arc<ProdQuotingService>,
}

pub async fn init_app(cfg: AppConfig) -> (AppController, RoutineHandle) {
    let AppConfig {
        quotes,
        core_url,
        treasury_url,
        ebill_url,
        clowder_rest_url,
        monitor_interval_seconds,
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
    let core = CoreClient::new(core_url);
    let treasury_cl = TreasuryClient::new(treasury_url);
    let ebill = EBillClient::new(ebill_url);
    let wdc_cl = client::WildcatCl {
        core,
        treasury: treasury_cl,
        ebill,
    };
    let quoting_service = ProdQuotingService {
        wdc_client: Box::new(wdc_cl),
        quotes: Box::new(quotes_repository),
        mint_url,
    };
    let quote = Arc::new(quoting_service);
    let monitor = monitor::EbillMonitor {
        srvc: quote.clone(),
    };
    let interval = std::time::Duration::from_secs(std::cmp::max(
        monitor_interval_seconds,
        MINIMUM_MONITOR_INTERVAL_SECONDS,
    ));
    let routine_handle = RoutineHandle::new(monitor, interval);
    (AppController { quote }, routine_handle)
}

pub fn routes<Cntrlr>(ctrl: Cntrlr) -> Router
where
    Arc<service::Service>: FromRef<Cntrlr> + Send + Sync + 'static,
    Cntrlr: Send + Sync + Clone + 'static,
{
    let web = Router::new()
        .route("/health", get(get_health))
        .route(MintClient::ENQUIRE_EP_V1, post(web::enquire_quote))
        .route(MintClient::LOOKUP_EP_V1, get(web::lookup_quote))
        .route(MintClient::RESOLVE_EP_V1, delete(web::cancel))
        .route(MintClient::RESOLVE_EP_V1, patch(web::resolve_offer));

    let admin = Router::new()
        .route(QuoteClient::LIST_EP_V1, get(admin::list_quotes))
        .route(QuoteClient::ADMIN_LOOKUP_EP_V1, get(admin::lookup_quote))
        .route(QuoteClient::UPDATE_EP_V1, patch(admin::update_quote));

    Router::new().merge(web).merge(admin).with_state(ctrl)
}

async fn get_health() -> &'static str {
    "{ \"status\": \"OK\" }"
}
