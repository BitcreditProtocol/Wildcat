// ----- standard library imports
use std::sync::Arc;
// ----- extra library imports
use axum::{
    extract::FromRef,
    routing::{get, post},
    Router,
};
use bcr_common::{client::keys::Client as KeysClient, wire::keys as wire_keys};
use bitcoin::bip32 as btc32;
use utoipa::OpenApi;
// ----- local modules
mod admin;
#[cfg(feature = "test-utils")]
pub mod client;
mod clowder;
mod error;
mod factory;
mod persistence;
mod service;
mod web;

// ----- end imports

type TStamp = chrono::DateTime<chrono::Utc>;
pub use service::KeysRepository;

pub type ProdKeysRepository = persistence::surreal::DBKeys;
pub type ProdSignaturesRepository = persistence::surreal::DBSignatures;
pub type ProdKeysService = service::Service;

#[derive(Clone, Debug, Default, serde::Deserialize)]
pub struct AppConfig {
    keys: persistence::surreal::DBKeysConnectionConfig,
    signatures: persistence::surreal::DBSignaturesConnectionConfig,
    clowder: clowder::ClowderClientConfig,
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
            clowder,
            starting_derivation_path,
        } = cfg;

        let keys_repo = ProdKeysRepository::new(keys)
            .await
            .expect("DB connection to keys failed");
        let signatures_repo = ProdSignaturesRepository::new(signatures)
            .await
            .expect("DB connection to signatures failed");
        let keygen = factory::Factory::new(seed, starting_derivation_path);
        let clowder_cl = clowder::build_clowder_client(clowder)
            .await
            .expect("clowder client");
        let srv = ProdKeysService {
            keys: Arc::new(keys_repo),
            signatures: Arc::new(signatures_repo),
            clowder: Arc::from(clowder_cl),
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
        .route(KeysClient::KEYSETINFO_EP_V1, get(web::lookup_keyset))
        .route(KeysClient::LISTKEYSETINFO_EP_V1, get(web::list_keysets))
        .route(KeysClient::KEYS_EP_V1, get(web::lookup_keys))
        .route(KeysClient::LISTKEYS_EP_V1, get(web::list_keys))
        .route(KeysClient::MINT_EP_V1, post(web::mint_ebill))
        .route(KeysClient::RESTORE_EP_V1, post(web::restore));
    // separate admin as it will likely have different auth requirements
    let admin = Router::new()
        .route(
            KeysClient::KEYSFOREXPIRATION_EP_V1,
            get(admin::get_keyset_for_date),
        )
        .route(KeysClient::SIGN_EP_V1, post(admin::sign_blind))
        .route(KeysClient::NEWMINTOP_EP_V1, post(admin::new_mintop))
        .route(KeysClient::MINTOPSTATUS_EP_V1, get(admin::mintop_status))
        .route(KeysClient::LISTMINTOPS_EP_V1, get(admin::list_mintops))
        .route(KeysClient::VERIFY_PROOF_EP_V1, post(admin::verify_proof))
        .route(
            KeysClient::VERIFY_FINGERPRINT_EP_V1,
            post(admin::verify_fingerprint),
        )
        .route(KeysClient::DEACTIVATEKEYSET_EP_V1, post(admin::deactivate));

    Router::new()
        .merge(web)
        .merge(admin)
        .with_state(ctrl)
        .merge(swagger)
}

#[derive(utoipa::OpenApi)]
#[openapi(
    components(schemas(
        wire_keys::DeactivateKeysetRequest,
        wire_keys::DeactivateKeysetResponse,
        wire_keys::KeysetMintCondition,
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
        admin::list_mintops,
        admin::mintop_status,
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
                clowder: Arc::new(clowder::DummyClowderClient),
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
