use std::str::FromStr;
// ----- standard library imports
// ----- extra library imports
use crate::service::{MintService, Service};
use axum::Router;
use axum::extract::FromRef;
use axum::routing::get;
use cashu::mint_url::MintUrl;
use cdk::HttpClient;
use cdk::wallet::client::MintConnector;
use serde::de::Unexpected::Option;

mod error;
mod keys;
mod mint_client;
mod service;

#[derive(Clone, Debug, serde::Deserialize)]
pub struct AppConfig {
    keys_client: keys::KeysClientConfig,
    cdk_mint_url: String,
}

#[derive(Clone, FromRef)]
pub struct AppController {
    bff: service::Service<mint_client::MintClient, keys::RESTClient>,
}

impl AppController {
    pub async fn new(cfg: AppConfig) -> Self {
        let AppConfig {
            keys_client,
            cdk_mint_url,
        } = cfg;

        let keys_client = keys::RESTClient::new(keys_client)
            .await
            .expect("Failed to create keys client");

        let _mint_url =
            MintUrl::from_str(cdk_mint_url.as_str()).expect("Failed to create mint url");

        let mint_client = mint_client::MintClient::new(_mint_url)
            .await
            .expect("Failed to create mint client");

        let info = mint_client.info().await;
        match info {
            Ok(_) => {
                log::info!(
                    "Connected to mint: {}",
                    info.map(|it| it.name)
                        .unwrap()
                        .filter(|s| !s.is_empty())
                        .or(Some("(empty)".to_string()))
                        .unwrap()
                        .to_string()
                );
            }
            Err(e) => {
                log::error!("Error on initial info request to mint: {}", e);
            }
        }

        Self {
            bff: Service {
                key_service: keys_client,
                mint_service: mint_client,
            },
        }
    }
}

pub fn routes(app: AppController) -> Router {
    Router::new().route("/health", get(health)).with_state(app)
}

async fn health() -> &'static str {
    "{ \"status\": \"OK\" }"
}
