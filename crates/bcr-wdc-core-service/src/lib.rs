// ----- standard library imports
use std::sync::Arc;
// ----- extra library imports
use axum::{
    extract::FromRef,
    routing::{get, post},
    Router,
};
use bcr_common::client::core::Client as CoreClient;
use bcr_wdc_utils::surreal;
use bitcoin::bip32 as btc32;
// ----- local modules
mod admin;
pub mod error;
pub mod keys;
pub mod persistence;
pub mod swap;
mod web;

// ----- end imports

type TStamp = chrono::DateTime<chrono::Utc>;

#[derive(Clone, Debug, serde::Deserialize)]
pub struct AppConfig {
    keys: surreal::DBConnConfig,
    proofs: surreal::DBConnConfig,
    signatures: surreal::DBConnConfig,
    clowder: keys::clowder::ClowderClientConfig,
    starting_derivation_path: btc32::DerivationPath,
}

#[derive(Clone, FromRef)]
pub struct AppController {
    pub keys: Arc<keys::service::Service>,
    pub swap: Arc<swap::service::Service>,
}

impl AppController {
    pub async fn new(seed: &[u8], cfg: AppConfig) -> Self {
        let AppConfig {
            keys,
            signatures,
            proofs,
            clowder,
            starting_derivation_path,
        } = cfg;

        let keys_repo = persistence::surreal::DBKeys::new(keys)
            .await
            .expect("DB connection to keys failed");
        let signatures_repo = persistence::surreal::DBSignatures::new(signatures)
            .await
            .expect("DB connection to signatures failed");
        let proofs_repo = persistence::surreal::DBProofs::new(proofs)
            .await
            .expect("Failed to create proofs repository");
        let keygen = keys::factory::Factory::new(seed, starting_derivation_path);
        let clowder_cl = keys::clowder::build_clowder_client(clowder)
            .await
            .expect("clowder client");
        let keys_service = keys::service::Service {
            keys: Box::new(keys_repo),
            signatures: Box::new(signatures_repo),
            clowder: clowder_cl,
            keygen,
        };
        let swap_service = swap::service::Service {
            proofs: Box::new(proofs_repo),
        };

        Self {
            keys: Arc::new(keys_service),
            swap: Arc::new(swap_service),
        }
    }
}

pub fn routes<Cntrlr>(ctrl: Cntrlr) -> Router
where
    Cntrlr: Send + Sync + Clone + 'static,
    Arc<keys::service::Service>: FromRef<Cntrlr>,
    Arc<swap::service::Service>: FromRef<Cntrlr>,
{
    let web = Router::new()
        .route("/health", get(get_health))
        .route(CoreClient::KEYSETINFO_EP_V1, get(web::lookup_keyset))
        .route(CoreClient::LISTKEYSETINFO_EP_V1, get(web::list_keysets))
        .route(CoreClient::KEYS_EP_V1, get(web::lookup_keys))
        .route(CoreClient::LISTKEYS_EP_V1, get(web::list_keys))
        .route(CoreClient::RESTORE_EP_V1, post(web::restore))
        .route(CoreClient::SWAP_EP_V1, post(web::swap_tokens))
        .route(CoreClient::BURN_EP_V1, post(web::burn_tokens))
        .route(CoreClient::CHECKSTATE_EP_V1, post(web::check_state));
    // separate admin as it will likely have different auth requirements
    let admin = Router::new()
        .route(CoreClient::NEW_KEYSET_EP_V1, post(admin::new_keyset))
        .route(CoreClient::SIGN_EP_V1, post(admin::sign_blind))
        .route(CoreClient::VERIFY_PROOF_EP_V1, post(admin::verify_proof))
        .route(
            CoreClient::VERIFY_FINGERPRINT_EP_V1,
            post(admin::verify_fingerprint),
        )
        .route(CoreClient::DEACTIVATEKEYSET_EP_V1, post(admin::deactivate))
        .route(CoreClient::RECOVER_EP_V1, post(admin::recover_tokens));

    Router::new().merge(web).merge(admin).with_state(ctrl)
}

async fn get_health() -> &'static str {
    "{ \"status\": \"OK\" }"
}

#[cfg(feature = "test-utils")]
pub mod test_utils {
    use super::*;
    use bcr_wdc_utils::KeysetEntry;

    fn test_controller() -> AppController {
        let seed = [0u8; 32];
        let derivation_path = btc32::DerivationPath::default();
        let keys_repo = persistence::inmemory::KeyMap::default();
        let signatures_repo = persistence::inmemory::SignatureMap::default();
        let keygen = keys::factory::Factory::new(&seed, derivation_path);
        let keysrv = keys::service::Service {
            keys: Box::new(keys_repo),
            signatures: Box::new(signatures_repo),
            keygen,
            clowder: Box::new(keys::clowder::DummyClowderClient),
        };
        let proofs_repo = persistence::inmemory::ProofMap::default();
        let swprv = swap::service::Service {
            proofs: Box::new(proofs_repo),
        };
        AppController {
            keys: Arc::new(keysrv),
            swap: Arc::new(swprv),
        }
    }

    pub async fn build_test_server(
        keyset: Option<KeysetEntry>,
    ) -> (axum_test::TestServer, AppController) {
        let cfg = axum_test::TestServerConfig {
            transport: Some(axum_test::Transport::HttpRandomPort),
            ..Default::default()
        };
        let cntrl = test_controller();
        if let Some(entry) = keyset {
            cntrl.keys.keys.store(entry).await.expect("store keyset");
        }
        let server = axum_test::TestServer::new_with_config(routes(cntrl.clone()), cfg)
            .expect("failed to start test server");
        (server, cntrl)
    }
}
