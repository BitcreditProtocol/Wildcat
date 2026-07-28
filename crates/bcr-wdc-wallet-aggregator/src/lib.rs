// ----- standard library imports
use std::sync::Arc;
// ----- extra library imports
use axum::{
    extract::FromRef,
    routing::{get, post},
    Router,
};
use bcr_common::client::{
    admin::{clowder, core},
    Url as ClientUrl,
};
// ----- local modules
mod error;
mod web;
// ----- local imports
use error::Result;

// ----- end imports

pub type TStamp = chrono::DateTime<chrono::Utc>;

#[derive(Clone, Debug, serde::Deserialize)]
pub struct AppConfig {
    core_client_url: bcr_common::client::Url,
    treasury_client_url: bcr_common::client::Url,
    clwdr_rest_url: ClientUrl,
}

#[derive(Clone, FromRef)]
pub struct AppController {
    core_client: bcr_common::client::core::Client,
    treasury_client: bcr_common::client::treasury::Client,
    clwdr_rest_client: Arc<clowder::Client>,
    time_started: chrono::DateTime<chrono::Utc>,
}

impl AppController {
    pub async fn new(cfg: AppConfig) -> Self {
        let AppConfig {
            core_client_url,
            treasury_client_url,
            clwdr_rest_url,
        } = cfg;

        let core_client = bcr_common::client::core::Client::new(core_client_url);
        let treasury_client = bcr_common::client::treasury::Client::new(treasury_client_url);
        let clwdr_rest_client = Arc::new(clowder::Client::new(clwdr_rest_url));

        Self {
            core_client,
            treasury_client,
            clwdr_rest_client,
            time_started: chrono::Utc::now(),
        }
    }
}

pub async fn routes(app: AppController) -> Result<Router> {
    let router = Router::new()
        .route("/health", get(web::health))
        .route("/v1/info", get(web::get_mint_info))
        .route(core::web_ep::SWAP_V1, post(web::post_swap))
        .with_state(app);
    Ok(router)
}
