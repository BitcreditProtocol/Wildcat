// ----- standard library imports
use std::sync::Arc;
// ----- extra library imports
use axum::{
    extract::FromRef,
    routing::{get, post},
    Router,
};
use bcr_common::{
    client::{
        admin::{clowder, core},
        Url as ClientUrl,
    },
    clwdr_client::ClowderNatsClient,
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
    clwdr_nats_url: ClientUrl,
    clwdr_rest_url: ClientUrl,
}

#[derive(Clone, FromRef)]
pub struct AppController {
    core_client: bcr_common::client::core::Client,
    treasury_client: bcr_common::client::treasury::Client,
    clwdr_stream_client: Arc<ClowderNatsClient>,
    clwdr_rest_client: Arc<clowder::Client>,
    time_started: chrono::DateTime<chrono::Utc>,
}

impl AppController {
    pub async fn new(cfg: AppConfig) -> Self {
        let AppConfig {
            core_client_url,
            treasury_client_url,
            clwdr_nats_url,
            clwdr_rest_url,
        } = cfg;

        let core_client = bcr_common::client::core::Client::new(core_client_url);
        let treasury_client = bcr_common::client::treasury::Client::new(treasury_client_url);
        let clwdr_stream_client = Arc::new(
            ClowderNatsClient::new(clwdr_nats_url)
                .await
                .expect("failed to init clowder nats client"),
        );
        let clwdr_rest_client = Arc::new(clowder::Client::new(clwdr_rest_url));

        Self {
            core_client,
            treasury_client,
            clwdr_stream_client,
            clwdr_rest_client,
            time_started: chrono::Utc::now(),
        }
    }
}

pub async fn routes(app: AppController) -> Result<Router> {
    let router = Router::new()
        .route("/health", get(web::health))
        // Cashu Endpoints
        .route("/v1/info", get(web::get_mint_info))
        .route("/v1/wildcat", get(web::get_wildcat_info))
        .route(core::web_ep::SWAP_V1, post(web::post_swap))
        // Clowder Endpoints
        .route(
            clowder::web_ep::OFFLINE_EXCHANGE_V1,
            post(web::post_offline_exchange),
        )
        .route(clowder::web_ep::LOCAL_COVERAGE_V1, get(web::get_coverage))
        .with_state(app);
    Ok(router)
}
