// ----- standard library imports
// ----- extra library imports
use axum::{
    extract::FromRef,
    routing::{get, post},
    Router,
};
use cashu::mint_url::MintUrl;
use cdk::HttpClient;
use tower_http::cors::{Any, CorsLayer};
use utoipa::OpenApi;
// ----- local modules
mod error;
mod web;
// ----- local imports

// ----- end imports

#[derive(Clone, Debug, serde::Deserialize)]
pub struct AppConfig {
    cdk_mint_url: MintUrl,
    keys_client_url: bcr_wdc_key_client::Url,
    swap_client_url: bcr_wdc_swap_client::Url,
    treasury_client_url: bcr_wdc_treasury_client::Url,
}

#[derive(Clone, FromRef)]
pub struct AppController {
    cdk_client: cdk::wallet::HttpClient,
    keys_client: bcr_wdc_key_client::KeyClient,
    swap_client: bcr_wdc_swap_client::SwapClient,
    treasury_client: bcr_wdc_treasury_client::TreasuryClient,
}

impl AppController {
    pub fn new(cfg: AppConfig) -> Self {
        let AppConfig {
            cdk_mint_url,
            keys_client_url,
            swap_client_url,
            treasury_client_url,
        } = cfg;

        let cdk_client = HttpClient::new(cdk_mint_url, None);
        let keys_client = bcr_wdc_key_client::KeyClient::new(keys_client_url);
        let swap_client = bcr_wdc_swap_client::SwapClient::new(swap_client_url);
        let treasury_client = bcr_wdc_treasury_client::TreasuryClient::new(treasury_client_url);

        Self {
            cdk_client,
            keys_client,
            swap_client,
            treasury_client,
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
