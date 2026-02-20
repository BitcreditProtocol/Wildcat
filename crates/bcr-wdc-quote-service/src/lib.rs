// ----- standard library imports
use std::sync::Arc;
// ----- extra library imports
use axum::{
    extract::FromRef,
    routing::{delete, get, post},
    Router,
};
use bcr_common::client::quote::Client as QuoteClient;
use bcr_wdc_utils::surreal;
// ----- local modules
mod admin;
mod ebill;
mod error;
mod keys;
mod persistence;
mod quotes;
mod service;
mod web;
// ----- local imports

// ----- end imports

type TStamp = chrono::DateTime<chrono::Utc>;

pub type ProdQuoteRepository = persistence::surreal::DBQuotes;

pub type ProdKeysHandler = keys::KeysRestHandler;
pub type ProdQuotingService = service::Service;

#[derive(Clone, Debug, serde::Deserialize)]
pub struct AppConfig {
    quotes: surreal::DBConnConfig,
    keys: keys::KeysRestConfig,
    ebill_client: ebill::EBillClientConfig,
    clowder_rest_url: reqwest::Url,
}

#[derive(Clone, FromRef)]
pub struct AppController {
    quote: ProdQuotingService,
}

impl AppController {
    pub async fn new(cfg: AppConfig) -> Self {
        let AppConfig {
            quotes,
            keys,
            ebill_client,
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
        let keys_hndlr = ProdKeysHandler::new(keys);
        let ebill = ebill::EBillClient::new(ebill_client);
        let quoting_service = ProdQuotingService {
            keys_hndlr: Arc::new(keys_hndlr),
            quotes: Arc::new(quotes_repository),
            ebill: Arc::new(ebill),
            mint_url,
        };

        Self {
            quote: quoting_service,
        }
    }
}
pub fn routes<Cntrlr>(ctrl: Cntrlr) -> Router
where
    service::Service: FromRef<Cntrlr> + Send + Sync + 'static,
    Cntrlr: Send + Sync + Clone + 'static,
{
    let user_routes = Router::new()
        .route("/health", get(get_health))
        .route(QuoteClient::ENQUIRE_EP_V1, post(web::enquire_quote))
        .route(QuoteClient::LOOKUP_EP_V1, get(web::lookup_quote))
        .route(QuoteClient::RESOLVE_EP_V1, delete(web::cancel))
        .route(QuoteClient::RESOLVE_EP_V1, post(web::resolve_offer));

    let admin_routes = Router::new()
        .route(QuoteClient::LIST_EP_V1, get(admin::list_quotes))
        .route("/v1/admin/credit/quote/{qid}", get(admin::lookup_quote))
        .route(QuoteClient::UPDATE_EP_V1, post(admin::update_quote))
        .route(
            "/v1/admin/credit/quote/enable_mint/{id}",
            post(admin::enable_minting),
        );

    Router::new()
        .merge(user_routes)
        .merge(admin_routes)
        .with_state(ctrl)
}

async fn get_health() -> &'static str {
    "{ \"status\": \"OK\" }"
}

#[cfg(feature = "test-utils")]
pub mod test_utils {
    use super::*;

    type TestKeysHandler = keys::test_utils::DummyKeysHandler;
    type TestQuoteRepository = persistence::inmemory::QuotesIDMap;
    type TestWallet = wallet::test_utils::DummyWallet;
    type TestEBillNode = ebill::test_utils::DummyEbillNode;
    type TestQuotingService = service::Service;

    #[derive(Clone, FromRef)]
    pub struct AppController {
        quotes: TestQuotingService,
    }

    impl std::default::Default for AppController {
        fn default() -> Self {
            AppController {
                quotes: TestQuotingService {
                    keys_hndlr: Arc::new(TestKeysHandler::default()),
                    wallet: Arc::new(TestWallet::default()),
                    quotes: Arc::new(TestQuoteRepository::default()),
                    ebill: Arc::new(TestEBillNode::default()),
                },
            }
        }
    }

    pub fn build_test_server() -> axum_test::TestServer {
        let cfg = axum_test::TestServerConfig {
            transport: Some(axum_test::Transport::HttpRandomPort),
            ..Default::default()
        };
        let cntrl = AppController::default();
        axum_test::TestServer::new_with_config(routes(cntrl), cfg)
            .expect("failed to start test server")
    }
}
