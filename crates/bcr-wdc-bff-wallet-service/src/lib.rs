// ----- standard library imports
// ----- extra library imports
use axum::Router;
use axum::extract::FromRef;
use axum::routing::{get};

mod keys;

type ProdKeysService = crate::keys::RESTClient;

#[derive(Clone, Debug, serde::Deserialize)]
pub struct AppConfig {
    keys_client: crate::keys::KeysClientConfig,
}

#[derive(Clone, FromRef)]
pub struct AppController {}

impl AppController {
    pub async fn new(cfg: AppConfig) -> Self {
        let AppConfig { keys_client } = cfg;

        let _keys_repo = ProdKeysService::new(keys_client)
            .await
            .expect("Failed to create keys client");

        Self {}
    }
}

pub fn routes(app: AppController) -> Router {
    Router::new().route("/health", get(health)).with_state(app)
}

async fn health() -> &'static str {
    "{ \"status\": \"OK\" }"
}
