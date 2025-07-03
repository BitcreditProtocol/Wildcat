// ----- standard library imports
// ----- extra library imports
use axum::extract::FromRef;
use axum::routing::{delete, get, post};
use axum::Router;
use cashu::nuts::nut00 as cdk00;
use cashu::nuts::nut02 as cdk02;
use cashu::nuts::nut11 as cdk11;
use cashu::nuts::nut12 as cdk12;
use cashu::nuts::nut14 as cdk14;
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

type TStamp = chrono::DateTime<chrono::Utc>;

pub type ProdQuoteRepository = persistence::surreal::DBQuotes;

pub type ProdKeysHandler = keys::KeysRestHandler;
pub type ProdWallet = wallet::Client;
pub type ProdQuotingService =
    service::Service<ProdKeysHandler, ProdWallet, ProdQuoteRepository, ebill::EBillClient>;

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
            keys_hndlr,
            wallet,
            quotes: quotes_repository,
            ebill,
        };

        Self {
            quote: quoting_service,
        }
    }
}
pub fn routes<Cntrlr, KeysHndlr, Wlt, QuotesRepo, EBillCl>(ctrl: Cntrlr) -> Router
where
    QuotesRepo: service::Repository + Send + Sync + 'static,
    Wlt: service::Wallet + Send + Sync + 'static,
    KeysHndlr: service::KeysHandler + Send + Sync + 'static,
    EBillCl: service::EBillNode + Send + Sync + 'static,
    service::Service<KeysHndlr, Wlt, QuotesRepo, EBillCl>: FromRef<Cntrlr> + Send + Sync + 'static,
    Cntrlr: Send + Sync + Clone + 'static,
{
    let swagger = utoipa_swagger_ui::SwaggerUi::new("/v1/admin/credit/swagger-ui")
        .url("/v1/admin/credit/api-docs/openapi.json", ApiDoc::openapi());

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
        .route(
            "/v1/admin/credit/quote/{id}",
            get(admin::admin_lookup_quote),
        )
        .route(
            "/v1/admin/credit/quote/{id}",
            post(admin::admin_update_quote),
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
        bcr_wdc_webapi::contact::ContactType,
        bcr_wdc_webapi::bill::BillIdentParticipant,
        bcr_wdc_webapi::bill::BillParticipant,
        bcr_wdc_webapi::identity::PostalAddress,
        bcr_wdc_webapi::quotes::BillInfo,
        bcr_wdc_webapi::quotes::EnquireReply,
        bcr_wdc_webapi::quotes::SignedEnquireRequest,
        bcr_wdc_webapi::quotes::InfoReply,
        bcr_wdc_webapi::quotes::LightInfo,
        bcr_wdc_webapi::quotes::ListReply,
        bcr_wdc_webapi::quotes::ListReplyLight,
        bcr_wdc_webapi::quotes::ListSort,
        bcr_wdc_webapi::quotes::ResolveOffer,
        bcr_wdc_webapi::quotes::StatusReply,
        bcr_wdc_webapi::quotes::StatusReplyDiscriminants,
        bcr_wdc_webapi::quotes::UpdateQuoteRequest,
        bcr_wdc_webapi::quotes::UpdateQuoteResponse,
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
        crate::admin::admin_lookup_quote,
        crate::admin::admin_update_quote,
        crate::admin::list_pending_quotes,
        crate::admin::list_quotes,
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
    type TestQuotingService =
        service::Service<TestKeysHandler, TestWallet, TestQuoteRepository, TestEBillNode>;

    #[derive(Clone, FromRef)]
    pub struct AppController {
        quotes: TestQuotingService,
    }

    impl std::default::Default for AppController {
        fn default() -> Self {
            AppController {
                quotes: TestQuotingService {
                    keys_hndlr: TestKeysHandler::default(),
                    wallet: TestWallet::default(),
                    quotes: TestQuoteRepository::default(),
                    ebill: TestEBillNode::default(),
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
