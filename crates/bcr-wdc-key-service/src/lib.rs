// ----- standard library imports
// ----- extra library imports
use axum::extract::FromRef;
use axum::routing::{get, post};
use axum::Router;
use bitcoin::bip32 as btc32;
use cashu::{nut00 as cdk00, nut01 as cdk01, nut02 as cdk02, nut04 as cdk04, nut09 as cdk09};
use utoipa::OpenApi;
// ----- local modules
mod admin;
mod error;
mod factory;
mod persistence;
mod service;
mod web;

// ----- end imports

type TStamp = chrono::DateTime<chrono::Utc>;

pub type ProdQuoteKeysRepository = persistence::surreal::DBQuoteKeys;
pub type ProdKeysRepository = persistence::surreal::DBKeys;
pub type ProdSignaturesRepository = persistence::surreal::DBSignatures;
pub type ProdKeysService =
    service::Service<ProdQuoteKeysRepository, ProdKeysRepository, ProdSignaturesRepository>;

#[derive(Clone, Debug, Default, serde::Deserialize)]
pub struct AppConfig {
    keys: persistence::surreal::ConnectionConfig,
    quotekeys: persistence::surreal::ConnectionConfig,
    signatures: persistence::surreal::ConnectionConfig,
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
            signatures,
            starting_derivation_path,
        } = cfg;

        let keys_repo = ProdKeysRepository::new(keys)
            .await
            .expect("DB connection to keys failed");
        let quotekeys_repo = ProdQuoteKeysRepository::new(quotekeys)
            .await
            .expect("DB connection to quotekeys failed");
        let signatures_repo = ProdSignaturesRepository::new(signatures)
            .await
            .expect("DB connection to signatures failed");
        let keygen = factory::Factory::new(seed, starting_derivation_path);
        let srv = ProdKeysService {
            keys: keys_repo,
            quote_keys: quotekeys_repo,
            signatures: signatures_repo,
            keygen,
        };
        Self { keys: srv }
    }
}

pub fn routes<Cntrlr, QuoteKeysRepo, KeysRepo, SignsRepo>(ctrl: Cntrlr) -> Router
where
    SignsRepo: service::SignaturesRepository + Send + Sync + 'static,
    KeysRepo: service::KeysRepository + Send + Sync + 'static,
    QuoteKeysRepo: service::QuoteKeysRepository + Send + Sync + 'static,
    service::Service<QuoteKeysRepo, KeysRepo, SignsRepo>: FromRef<Cntrlr> + Send + Sync + 'static,
    Cntrlr: Send + Sync + Clone + 'static,
{
    let swagger = utoipa_swagger_ui::SwaggerUi::new("/swagger-ui")
        .url("/api-docs/openapi.json", ApiDoc::openapi());

    let web = Router::new()
        .route("/v1/keysets/{kid}", get(web::lookup_keyset))
        .route("/v1/keysets", get(web::list_keysets))
        .route("/v1/keys/{kid}", get(web::lookup_keys))
        .route("/v1/keys", get(web::list_keys))
        .route("/v1/mint/ebill", post(web::mint_ebill))
        .route("/v1/restore", post(web::restore));
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
        cdk09::RestoreRequest,
        cdk09::RestoreResponse,
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
        web::lookup_keyset,
        web::mint_ebill,
        web::restore,
    )
)]
struct ApiDoc;

#[cfg(feature = "test-utils")]
pub use crate::service::MintCondition;
#[cfg(feature = "test-utils")]
pub mod test_utils {
    use super::*;
    use bcr_wdc_utils::{keys::test_utils as keys_test, KeysetEntry};
    use cashu::Amount;

    pub type InMemoryRepository = persistence::inmemory::InMemoryKeyMap;
    pub type TestQuoteKeysRepository = persistence::inmemory::InMemoryQuoteKeyMap;
    pub type TestKeysRepository = persistence::inmemory::InMemoryKeyMap;
    pub type TestSignaturesRepository = persistence::inmemory::InMemorySignatureMap;
    pub type TestKeysService =
        service::Service<TestQuoteKeysRepository, TestKeysRepository, TestSignaturesRepository>;

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
            let signatures_repo = TestSignaturesRepository::default();
            let keygen = factory::Factory::new(&seed, derivation_path);
            let srv = TestKeysService {
                keys: keys_repo,
                quote_keys: quotekeys_repo,
                signatures: signatures_repo,
                keygen,
            };
            Self { keys: srv }
        }
    }

    pub fn build_test_server(keyset: Option<KeysetEntry>) -> axum_test::TestServer {
        let cfg = axum_test::TestServerConfig {
            transport: Some(axum_test::Transport::HttpRandomPort),
            ..Default::default()
        };
        let cntrl = AppController::default();
        if let Some(entry) = keyset {
            let condition = MintCondition {
                is_minted: true,
                pub_key: keys_test::publics()[0],
                target: Amount::ZERO,
            };
            cntrl
                .keys
                .keys
                .store(entry, condition)
                .expect("store keyset");
        }
        axum_test::TestServer::new_with_config(routes(cntrl), cfg)
            .expect("failed to start test server")
    }
}
