use std::str::FromStr;
// ----- standard library imports
// ----- extra library imports
use axum::Router;
use axum::extract::FromRef;
use axum::routing::get;
use cashu::mint_url::MintUrl;
use cdk::HttpClient;
use cdk::wallet::client::MintConnector;

mod keys;

type ProdKeysService = crate::keys::RESTClient;

#[derive(Clone, Debug, serde::Deserialize)]
pub struct AppConfig {
    keys_client: crate::keys::KeysClientConfig,
    cdk_mint_url: String,
}

#[derive(Clone, FromRef)]
pub struct AppController {}

impl AppController {
    pub async fn new(cfg: AppConfig) -> Self {
        let AppConfig {
            keys_client,
            cdk_mint_url,
        } = cfg;

        let _keys_repo = ProdKeysService::new(keys_client)
            .await
            .expect("Failed to create keys client");

        let _mint_url =
            MintUrl::from_str(cdk_mint_url.as_str()).expect("Failed to create mint url");

        let _mint_client = HttpClient::new(_mint_url);

        let info = _mint_client.get_mint_info().await;
        log::info!(
            "Connected to mint: {}",
            info.map(|it| it.version)
                .unwrap()
                .or(None)
                .unwrap()
                .to_string()
        );

        Self {}
    }
}

pub fn routes(app: AppController) -> Router {
    Router::new().route("/health", get(health)).with_state(app)
}

async fn health() -> &'static str {
    "{ \"status\": \"OK\" }"
}
