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
#[cfg(any(feature = "test-utils", test))]
pub mod utils;
mod web;
// ----- local imports

// ----- end imports

type ProdProofRepository = persistence::surreal::ProofDB;
type ProdKeysService = crate::keys::RESTClient;
type ProdSwapService = service::Service<ProdKeysService, ProdProofRepository>;

#[derive(Clone, Debug, serde::Deserialize)]
pub struct AppConfig {
    proof_db: persistence::surreal::ConnectionConfig,
    keys_client: crate::keys::KeysClientConfig,
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

        let keys_client = ProdKeysService::new(keys_client);
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
    type TestSwapService = service::Service<TestKeysService, TestProofRepository>;

    use super::*;
    use crate::error::{Error, Result};
    use cashu::nut00 as cdk00;
    use cashu::nut02 as cdk02;

    #[derive(Debug, Default, Clone)]
    pub struct TestKeysService {
        pub keys: bcr_wdc_key_client::test_utils::KeyClient,
    }
    #[async_trait::async_trait]
    impl service::KeysService for TestKeysService {
        async fn info(&self, id: &cdk02::Id) -> Result<cdk02::KeySetInfo> {
            self.keys.keyset_info(*id).await.map_err(Error::KeysClient)
        }
        async fn sign_blind(&self, blind: &cdk00::BlindedMessage) -> Result<cdk00::BlindSignature> {
            self.keys.sign(blind).await.map_err(Error::KeysClient)
        }
        async fn verify_proof(&self, proof: &cdk00::Proof) -> Result<()> {
            self.keys.verify(proof).await.map_err(Error::KeysClient)?;
            Ok(())
        }
    }

    #[derive(Clone, FromRef)]
    pub struct AppController {
        keys: TestSwapService,
    }

    impl AppController {
        pub fn new(keys: TestKeysService) -> Self {
            let proofs_repo = TestProofRepository::default();
            let srv = TestSwapService {
                keys,
                proofs: proofs_repo,
            };
            Self { keys: srv }
        }
    }

    pub fn build_test_server() -> (axum_test::TestServer, TestKeysService) {
        let cfg = axum_test::TestServerConfig {
            transport: Some(axum_test::Transport::HttpRandomPort),
            ..Default::default()
        };
        let keys = TestKeysService::default();
        let cntrl = AppController::new(keys.clone());
        let srv = axum_test::TestServer::new_with_config(routes(cntrl), cfg)
            .expect("failed to start test server");
        (srv, keys)
    }
}
