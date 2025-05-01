// ----- standard library imports
// ----- extra library imports
use axum::extract::FromRef;
use axum::routing::{get, post};
use axum::Router;
use bitcoin::bip32 as btc32;
use cashu::{nut00 as cdk00, nut01 as cdk01, nut02 as cdk02, nut04 as cdk04};
use utoipa::OpenApi;
// ----- local modules
mod admin;
mod error;
mod factory;
mod persistence;
mod service;
mod web;

// ----- end imports
#[cfg(feature = "test-utils")]
pub use crate::service::MintCondition;

type TStamp = chrono::DateTime<chrono::Utc>;

pub type ProdQuoteKeysRepository = persistence::surreal::DBQuoteKeys;
pub type ProdKeysRepository = persistence::surreal::DBKeys;
pub type ProdKeysService = service::Service<ProdQuoteKeysRepository, ProdKeysRepository>;

#[derive(Clone, Debug, Default, serde::Deserialize)]
pub struct AppConfig {
    keys: persistence::surreal::ConnectionConfig,
    quotekeys: persistence::surreal::ConnectionConfig,
    starting_derivation_path: btc32::DerivationPath,
}

#[derive(Clone, FromRef)]
pub struct AppController {
    keys: ProdKeysService,
}

impl AppController {
    pub async fn new(seed: &[u8], cfg: AppConfig) -> Self {
        let AppConfig {
            keys,
            quotekeys,
            starting_derivation_path,
        } = cfg;

        let keys_repo = ProdKeysRepository::new(keys)
            .await
            .expect("DB connection to keys failed");
        let quotekeys_repo = ProdQuoteKeysRepository::new(quotekeys)
            .await
            .expect("DB connection to quotekeys failed");
        let keygen = factory::Factory::new(seed, starting_derivation_path);
        let srv = ProdKeysService {
            keys: keys_repo,
            quote_keys: quotekeys_repo,
            keygen,
        };
        Self { keys: srv }
    }
}

pub fn routes<Cntrlr, Q, K>(ctrl: Cntrlr) -> Router
where
    Q: service::QuoteKeysRepository + Send + Sync + 'static,
    K: service::KeysRepository + Send + Sync + 'static,
    service::Service<Q, K>: FromRef<Cntrlr>,
    Cntrlr: Send + Sync + Clone + 'static,
{
    let swagger = utoipa_swagger_ui::SwaggerUi::new("/swagger-ui")
        .url("/api-docs/openapi.json", ApiDoc::openapi());

    let web = Router::new()
        .route("/v1/keysets/{kid}", get(web::lookup_keysets))
        .route("/v1/keysets", get(web::list_keysets))
        .route("/v1/keys/{kid}", get(web::lookup_keys))
        .route("/v1/keys", get(web::list_keys))
        .route("/v1/mint/ebill", post(web::mint_ebill));
    // separate admin as it will likely have different auth requirements
    let admin = Router::new()
        .route("/v1/admin/keys/sign", post(admin::sign_blind))
        .route("/v1/admin/keys/pre_sign", post(admin::pre_sign))
        .route("/v1/admin/keys/generate", post(admin::generate))
        .route("/v1/admin/keys/verify", post(admin::verify_proof))
        .route("/v1/admin/keys/activate", post(admin::activate));

    Router::new()
        .merge(web)
        .merge(admin)
        .with_state(ctrl)
        .merge(swagger)
}

#[derive(utoipa::OpenApi)]
#[openapi(
    components(schemas(
        bcr_wdc_webapi::keys::ActivateKeysetRequest,
        bcr_wdc_webapi::keys::GenerateKeysetRequest,
        bcr_wdc_webapi::keys::KeysetMintCondition,
        bcr_wdc_webapi::keys::PreSignRequest,
        cdk00::BlindSignature,
        cdk00::BlindedMessage,
        cdk00::Proof,
        cdk01::KeysResponse,
        cdk02::Id,
        cdk02::KeySet,
        cdk02::KeySetInfo,
        cdk02::KeysetResponse,
        cdk04::MintBolt11Request<String>,
        cdk04::MintBolt11Response,
    ),),
    paths(
        admin::activate,
        admin::generate,
        admin::pre_sign,
        admin::sign_blind,
        admin::verify_proof,
        web::list_keys,
        web::list_keysets,
        web::lookup_keys,
        web::lookup_keysets,
        web::mint_ebill,
    )
)]
struct ApiDoc;

#[cfg(feature = "test-utils")]
pub mod test_utils {
    use super::*;

    pub type InMemoryRepository = persistence::inmemory::InMemoryMap;
    pub type TestQuoteKeysRepository = persistence::inmemory::InMemoryQuoteKeyMap;
    pub type TestKeysRepository = persistence::inmemory::InMemoryMap;
    pub type TestKeysService = service::Service<TestQuoteKeysRepository, TestKeysRepository>;

    #[derive(Clone, FromRef)]
    pub struct AppController {
        keys: TestKeysService,
    }

    impl std::default::Default for AppController {
        fn default() -> Self {
            let seed = [0u8; 32];
            let derivation_path = btc32::DerivationPath::default();
            let keys_repo = TestKeysRepository::default();
            let quotekeys_repo = TestQuoteKeysRepository::default();
            let keygen = factory::Factory::new(&seed, derivation_path);
            let srv = TestKeysService {
                keys: keys_repo,
                quote_keys: quotekeys_repo,
                keygen,
            };
            Self { keys: srv }
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
