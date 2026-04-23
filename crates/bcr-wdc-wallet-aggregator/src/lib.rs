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
use bcr_wdc_utils::surreal;
// ----- local modules
mod commitment;
mod error;
mod persistence;
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
    commit_repo_cfg: surreal::DBConnConfig,
    #[serde(default = "default_commitment_expiry_secs")]
    commitment_expiry_secs: u64,
}

fn default_commitment_expiry_secs() -> u64 {
    1200
}

#[derive(Clone, FromRef)]
pub struct AppController {
    core_client: bcr_common::client::core::Client,
    treasury_client: bcr_common::client::treasury::Client,
    clwdr_stream_client: Arc<ClowderNatsClient>,
    clwdr_rest_client: Arc<clowder::Client>,
    commit_srv: Arc<commitment::Service>,
    time_started: chrono::DateTime<chrono::Utc>,
}

impl AppController {
    pub async fn new(cfg: AppConfig) -> Self {
        let AppConfig {
            core_client_url,
            treasury_client_url,
            clwdr_nats_url,
            clwdr_rest_url,
            commit_repo_cfg,
            commitment_expiry_secs,
        } = cfg;

        let core_client = bcr_common::client::core::Client::new(core_client_url);
        let treasury_client = bcr_common::client::treasury::Client::new(treasury_client_url);
        let clwdr_stream_client = Arc::new(
            ClowderNatsClient::new(clwdr_nats_url)
                .await
                .expect("failed to init clowder nats client"),
        );
        let clwdr_rest_client = Arc::new(clowder::Client::new(clwdr_rest_url));
        let commit_repo = persistence::surreal::DBCommitments::new(commit_repo_cfg)
            .await
            .expect("failed to init commitment repo");
        let commit_srv = Arc::new(commitment::Service {
            repo: Box::new(commit_repo),
            max_expiry: chrono::Duration::seconds(commitment_expiry_secs as i64),
        });

        Self {
            core_client,
            treasury_client,
            clwdr_stream_client,
            clwdr_rest_client,
            commit_srv,
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
            clowder::web_ep::ONLINE_EXCHANGE_V1,
            post(web::post_online_exchange),
        )
        .route(
            clowder::web_ep::OFFLINE_EXCHANGE_V1,
            post(web::post_offline_exchange),
        )
        .route(clowder::web_ep::LOCAL_COVERAGE_V1, get(web::get_coverage))
        .with_state(app);
    Ok(router)
}
