// ----- standard library imports
use std::sync::Arc;
// ----- extra library imports
use axum::extract::FromRef;
use axum::routing::{get, post};
use axum::Router;
use bitcoin::bip32 as btc32;
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

pub type ProdKeysRepository = persistence::surreal::DBKeys;
pub type ProdSignaturesRepository = persistence::surreal::DBSignatures;
pub type ProdKeysService = service::Service;

#[derive(Clone, Debug, Default, serde::Deserialize)]
pub struct AppConfig {
    keys: persistence::surreal::DBKeysConnectionConfig,
    signatures: persistence::surreal::DBSignaturesConnectionConfig,
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
            signatures,
            starting_derivation_path,
        } = cfg;

        let keys_repo = ProdKeysRepository::new(keys)
            .await
            .expect("DB connection to keys failed");
        let signatures_repo = ProdSignaturesRepository::new(signatures)
            .await
            .expect("DB connection to signatures failed");
        let keygen = factory::Factory::new(seed, starting_derivation_path);
        let srv = ProdKeysService {
            keys: Arc::new(keys_repo),
            signatures: Arc::new(signatures_repo),
            keygen,
        };
        Self { keys: srv }
    }
}

pub fn routes<Cntrlr>(ctrl: Cntrlr) -> Router
where
    Cntrlr: Send + Sync + Clone + 'static,
    service::Service: FromRef<Cntrlr>,
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
        .route("/v1/admin/keys/{date}", get(admin::get_keyset_for_date))
        .route("/v1/admin/keys/sign", post(admin::sign_blind))
        .route("/v1/admin/keys/verify", post(admin::verify_proof))
        .route("/v1/admin/keys/deactivate", post(admin::deactivate));

    Router::new()
        .merge(web)
        .merge(admin)
        .with_state(ctrl)
        .merge(swagger)
}

#[derive(utoipa::OpenApi)]
#[openapi(
    components(schemas(
        bcr_wdc_webapi::keys::DeactivateKeysetRequest,
        bcr_wdc_webapi::keys::DeactivateKeysetResponse,
        bcr_wdc_webapi::keys::KeysetMintCondition,
        cashu::BlindSignature,
        cashu::BlindedMessage,
        cashu::Proof,
        cashu::KeysResponse,
        cashu::Id,
        cashu::KeySet,
        cashu::KeySetInfo,
        cashu::KeysetResponse,
        cashu::MintRequest<String>,
        cashu::MintResponse,
        cashu::RestoreRequest,
        cashu::RestoreResponse,
    ),),
    paths(
        admin::deactivate,
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
pub use crate::service::MintOperation;
#[cfg(feature = "test-utils")]
pub mod test_utils {
    use super::*;
    use bcr_wdc_utils::KeysetEntry;

    pub type InMemoryRepository = persistence::inmemory::InMemoryKeyMap;
    pub type TestKeysRepository = persistence::inmemory::InMemoryKeyMap;
    pub type TestSignaturesRepository = persistence::inmemory::InMemorySignatureMap;
    pub type TestKeysService = service::Service;

    #[derive(Clone, FromRef)]
    pub struct AppController {
        keys: TestKeysService,
    }

    impl std::default::Default for AppController {
        fn default() -> Self {
            let seed = [0u8; 32];
            let derivation_path = btc32::DerivationPath::default();
            let keys_repo = TestKeysRepository::default();
            let signatures_repo = TestSignaturesRepository::default();
            let keygen = factory::Factory::new(&seed, derivation_path);
            let srv = TestKeysService {
                keys: Arc::new(keys_repo),
                signatures: Arc::new(signatures_repo),
                keygen,
            };
            Self { keys: srv }
        }
    }

    pub async fn build_test_server(keyset: Option<KeysetEntry>) -> axum_test::TestServer {
        let cfg = axum_test::TestServerConfig {
            transport: Some(axum_test::Transport::HttpRandomPort),
            ..Default::default()
        };
        let cntrl = AppController::default();
        if let Some(entry) = keyset {
            cntrl.keys.keys.store(entry).await.expect("store keyset");
        }
        axum_test::TestServer::new_with_config(routes(cntrl), cfg)
            .expect("failed to start test server")
    }
}
