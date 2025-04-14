use crate::service::Service;
use axum::extract::FromRef;
use axum::routing::{get, post};
use axum::Router;
use bcr_wdc_key_client::KeyClient;
use cashu::mint_url::MintUrl;
use cdk::wallet::MintConnector;
use cdk::HttpClient;
use std::str::FromStr;
use std::sync::Arc;
use tower_http::cors::{Any, CorsLayer};
use utoipa::OpenApi;

mod error;
mod service;
mod web;

#[derive(Debug, Clone, serde::Deserialize)]
pub struct KeysClientConfig {
    base_url: bcr_wdc_key_client::Url,
}

#[derive(Clone, Debug, serde::Deserialize)]
pub struct AppConfig {
    keys_client: KeysClientConfig,
    cdk_mint_url: String,
}

#[derive(Clone, FromRef)]
pub struct AppController {
    bff: Service,
}

impl AppController {
    pub async fn new(cfg: AppConfig) -> Self {
        let AppConfig {
            keys_client,
            cdk_mint_url,
        } = cfg;

        let keys_client = KeyClient::new(keys_client.base_url);

        let _mint_url =
            MintUrl::from_str(cdk_mint_url.as_str()).expect("Failed to create mint url");

        let mint_client = HttpClient::new(_mint_url, None);

        let info = mint_client.get_mint_info().await;
        match info {
            Ok(_) => {
                log::info!(
                    "Connected to mint: {}",
                    info.map(|it| it.name)
                        .unwrap()
                        .filter(|s| !s.is_empty())
                        .unwrap_or("(empty)".to_string())
                );
            }
            Err(e) => {
                log::error!("Error on initial info request to mint: {}", e);
            }
        }

        Self {
            bff: Service {
                key_service: keys_client,
                mint_service: Arc::new(mint_client),
            },
        }
    }
}

pub fn routes(app: AppController) -> Router {
    let swagger = utoipa_swagger_ui::SwaggerUi::new("/swagger-ui")
        .url("/api-docs/openapi.json", ApiDoc::openapi());

    Router::new()
        .route("/health", get(web::health))
        .route("/v1/info", get(web::get_mint_info))
        .route("/v1/keys", get(web::get_mint_keys))
        .route("/v1/keysets", get(web::get_mint_keysets))
        .route("/v1/keys/{kid}", get(web::get_mint_keyset))
        .route("/v1/mint/quote/bolt11", post(web::post_mint_quote))
        .route(
            "/v1/mint/quote/bolt11/{quote_id}",
            get(web::get_mint_quote_status),
        )
        .route("/v1/mint/bolt11", post(web::post_mint))
        .route("/v1/melt/quote/bolt11", post(web::post_melt_quote))
        .route(
            "/v1/melt/quote/bolt11/{quote_id}",
            get(web::get_melt_quote_status),
        )
        .route("/v1/melt/bolt11", post(web::post_melt))
        .route("/v1/swap", post(web::post_swap))
        .route("/v1/checkstate", post(web::post_check_state))
        .route("/v1/restore", post(web::post_restore))
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_headers(Any)
                .allow_methods(Any)
                .expose_headers(Any),
        )
        .with_state(app)
        .merge(swagger)
}

#[derive(utoipa::OpenApi)]
#[openapi(paths(
    crate::web::health,
    crate::web::get_mint_keys,
    crate::web::get_mint_keysets,
    crate::web::get_mint_keyset,
    crate::web::post_mint_quote,
    crate::web::get_mint_quote_status,
    crate::web::post_mint,
    crate::web::post_melt_quote,
    crate::web::get_melt_quote_status,
    crate::web::post_melt,
    crate::web::post_swap,
    crate::web::post_check_state,
    crate::web::post_restore,
))]
struct ApiDoc;
