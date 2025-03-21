// ----- standard library imports
// ----- extra library imports
use axum::extract::FromRef;
use axum::routing::post;
use axum::Router;
// ----- local modules
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
type ProdService = service::Service<ProdKeysService, ProdProofRepository>;

#[derive(Clone, Debug, serde::Deserialize)]
pub struct AppConfig {
    proof_db: persistence::surreal::ConnectionConfig,
    keys_client: crate::keys::KeysClientConfig,
}

#[derive(Clone, FromRef)]
pub struct AppController {
    swap: ProdService,
}

impl AppController {
    pub async fn new(cfg: AppConfig) -> Self {
        let AppConfig { proof_db, keys_client } = cfg;

        let keys_repo = ProdKeysService::new(keys_client)
            .await
            .expect("Failed to create keys client");
        let proofs_repo = ProdProofRepository::new(proof_db)
            .await
            .expect("Failed to create proofs repository");

        let srv = ProdService {
            keys: keys_repo,
            proofs: proofs_repo,
        };
        Self { swap: srv }
    }
}

pub fn routes(app: AppController) -> Router {
    Router::new()
        .route("/v1/swap", post(crate::web::swap_tokens))
        .with_state(app)
}
