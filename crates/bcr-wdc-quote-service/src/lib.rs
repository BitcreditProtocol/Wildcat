// ----- standard library imports
use std::sync::Arc;
// ----- extra library imports
use axum::extract::FromRef;
use axum::routing::{delete, get, post};
use axum::Router;
use cashu::{nut00 as cdk00, nut02 as cdk02, nut11 as cdk11, nut12 as cdk12, nut14 as cdk14};
use utoipa::OpenApi;
// ----- local modules
mod admin;
mod ebill;
mod error;
mod keys;
mod persistence;
mod quotes;
mod service;
mod wallet;
mod web;
// ----- local imports

// ----- end imports

type TStamp = chrono::DateTime<chrono::Utc>;

pub type ProdQuoteRepository = persistence::surreal::DBQuotes;

pub type ProdKeysHandler = keys::KeysRestHandler;
pub type ProdWallet = wallet::Client;
pub type ProdQuotingService = service::Service;

#[derive(Clone, Debug, serde::Deserialize)]
pub struct AppConfig {
    quotes: persistence::surreal::ConnectionConfig,
    keys: keys::KeysRestConfig,
    wallet: wallet::WalletConfig,
    ebill_client: ebill::EBillClientConfig,
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
            wallet,
            ebill_client,
        } = cfg;
        let quotes_repository = ProdQuoteRepository::new(quotes)
            .await
            .expect("DB connection to quotes failed");

        let keys_hndlr = ProdKeysHandler::new(keys);
        let wallet = ProdWallet::new(wallet);
        let ebill = ebill::EBillClient::new(ebill_client);
        let quoting_service = ProdQuotingService {
            keys_hndlr: Arc::new(keys_hndlr),
            wallet: Arc::new(wallet),
            quotes: Arc::new(quotes_repository),
            ebill: Arc::new(ebill),
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
    let swagger = utoipa_swagger_ui::SwaggerUi::new("/swagger-ui")
        .url("/api-docs/openapi.json", ApiDoc::openapi());

    let user_routes = Router::new()
        .route("/v1/mint/quote/credit", post(web::enquire_quote))
        .route("/v1/mint/quote/credit/{id}", get(web::lookup_quote))
        .route("/v1/mint/quote/credit/{id}", delete(web::cancel))
        .route("/v1/mint/quote/credit/{id}", post(web::resolve_offer));

    let admin_routes = Router::new()
        .route(
            "/v1/admin/credit/quote/pending",
            get(admin::list_pending_quotes),
        )
        .route("/v1/admin/credit/quote", get(admin::list_quotes))
        .route("/v1/admin/credit/quote/{id}", get(admin::lookup_quote))
        .route("/v1/admin/credit/quote/{id}", post(admin::update_quote))
        .route(
            "/v1/admin/credit/quote/enable_mint/{id}",
            post(admin::enable_minting),
        );

    Router::new()
        .merge(user_routes)
        .merge(admin_routes)
        .with_state(ctrl)
        .merge(swagger)
}

#[derive(utoipa::OpenApi)]
#[openapi(
    components(schemas(
        bcr_common::wire::bill::BillIdentParticipant,
        bcr_common::wire::bill::BillParticipant,
        bcr_common::wire::contact::ContactType,
        bcr_common::wire::identity::PostalAddress,
        bcr_common::wire::quotes::BillInfo,
        bcr_common::wire::quotes::EnableMintingRequest,
        bcr_common::wire::quotes::EnableMintingResponse,
        bcr_common::wire::quotes::EnquireReply,
        bcr_common::wire::quotes::InfoReply,
        bcr_common::wire::quotes::LightInfo,
        bcr_common::wire::quotes::ListReply,
        bcr_common::wire::quotes::ListReplyLight,
        bcr_common::wire::quotes::ListSort,
        bcr_common::wire::quotes::ResolveOffer,
        bcr_common::wire::quotes::SignedEnquireRequest,
        bcr_common::wire::quotes::StatusReply,
        bcr_common::wire::quotes::StatusReplyDiscriminants,
        bcr_common::wire::quotes::UpdateQuoteRequest,
        bcr_common::wire::quotes::UpdateQuoteResponse,
        cashu::Amount,
        cdk00::BlindSignature,
        cdk00::BlindedMessage,
        cdk00::Witness,
        cdk02::Id,
        cdk11::P2PKWitness,
        cdk12::BlindSignatureDleq,
        cdk14::HTLCWitness,
    ),),
    paths(
        crate::admin::enable_minting,
        crate::admin::list_pending_quotes,
        crate::admin::list_quotes,
        crate::admin::lookup_quote,
        crate::admin::update_quote,
        crate::web::cancel,
        crate::web::enquire_quote,
        crate::web::lookup_quote,
        crate::web::resolve_offer,
    )
)]
pub struct ApiDoc;

impl ApiDoc {
    pub fn generate_yml() -> Option<String> {
        ApiDoc::openapi().to_yaml().ok()
    }
    pub fn generate_json() -> Option<String> {
        ApiDoc::openapi().to_pretty_json().ok()
    }
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
#[test]
fn it_should_successfully_generate_openapi_docs() {
    let yml = ApiDoc::generate_yml();
    assert!(yml.is_some());

    let json = ApiDoc::generate_json();
    assert!(json.is_some());
}
