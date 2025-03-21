// ----- standard library imports
// ----- extra library imports
use axum::extract::FromRef;
use axum::routing::post;
use axum::Router;
// ----- local modules
mod admin;
mod error;
mod keys;
mod persistence;
mod service;
#[cfg(test)]
mod utils;
mod web;
// ----- local imports

type ProdProofRepository = persistence::surreal::ProofDB;
type ProdKeysService = crate::keys::RESTClient;
type ProdSwapService = service::Service<ProdKeysService, ProdProofRepository>;

#[derive(Clone, Debug, serde::Deserialize)]
pub struct AppConfig {
    proof_db: persistence::surreal::ConnectionConfig,
    keys_cl: crate::keys::KeysClientConfig,
}

#[derive(Clone, FromRef)]
pub struct AppController {
    swap: ProdSwapService,
}

impl AppController {
    pub async fn new(cfg: AppConfig) -> Self {
        let AppConfig {
            proof_db,
            keys_client,
        } = cfg;

        let keys_client = ProdKeysService::new(keys_cl)
            .await
            .expect("Failed to create keys client");
        let proofs_repo = ProdProofRepository::new(proof_db)
            .await
            .expect("Failed to create proofs repository");

        let srv = ProdSwapService {
            keys: keys_client,
            proofs: proofs_repo,
        };
        Self { swap: srv }
    }
}

pub fn routes<Cntrlr, KeysSrvc, ProofRepo>(ctrl: Cntrlr) -> Router
where
    KeysSrvc: service::KeysService + Send + Sync + 'static,
    ProofRepo: service::ProofRepository + Send + Sync + 'static,
    service::Service<KeysSrvc, ProofRepo>: FromRef<Cntrlr>,
    Cntrlr: Send + Sync + Clone + 'static,
{
    let web = Router::new()
        .route("/v1/swap", post(crate::web::swap_tokens))
        .route("/v1/burn", post(crate::web::burn_tokens));
    // separate admin as it will likely have different auth requirements
    let admin = Router::new().route("/v1/recover", post(crate::admin::recover_tokens));

    Router::new().merge(web).merge(admin).with_state(ctrl)
}

#[cfg(feature = "test-utils")]
pub mod test_utils {
    type TestProofRepository = persistence::inmemory::ProofMap;
    type TestSwapService = service::Service<ProdKeysService, TestProofRepository>;
    use super::*;

    #[derive(Clone, FromRef)]
    pub struct AppController {
        keys: TestSwapService,
    }

    impl AppController {
        pub fn new() -> Self {
            let keys_cfg = crate::keys::KeysClientConfig {
                base_url: "http://localhost:8080".parse().expect("valid url"),
            };
            let keys_client = ProdKeysService::new(keys_cfg)
                .await
                .expect("Failed to create keys client");
            let proofs_repo = TestProofRepository::default();
            let srv = TestSwapService {
                keys: keys_client,
                proofs: proofs_repo,
            };
            Self { keys: srv }
        }
    }

    pub fn build_test_server() -> axum_test::TestServer {
        let cfg = axum_test::TestServerConfig {
            transport: Some(axum_test::Transport::HttpRandomPort),
            ..Default::default()
        };
        let cntrl = AppController::new();
        axum_test::TestServer::new_with_config(routes(cntrl), cfg)
            .expect("failed to start test server")
    }
}
