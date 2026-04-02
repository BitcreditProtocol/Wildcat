// ----- standard library imports
use std::sync::Arc;
// ----- extra library imports
use axum::{
    extract::FromRef,
    routing::{get, post},
    Router,
};
use bcr_common::{
    cashu,
    client::clowder::Client as ClowderClient,
    wire::{
        clowder::{self as wire_clowder},
        exchange as wire_exchange,
    },
};
use bcr_wdc_utils::surreal;
use utoipa::OpenApi;
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
    clwdr_nats_url: clwdr_client::Url,
    clwdr_rest_url: clwdr_client::Url,
    signer_url: clwdr_client::Url,
    commit_repo_cfg: surreal::DBConnConfig,
}

#[derive(Clone, FromRef)]
pub struct AppController {
    core_client: bcr_common::client::core::Client,
    treasury_client: bcr_common::client::treasury::Client,
    clwdr_stream_client: Arc<clwdr_client::ClowderNatsClient>,
    clwdr_rest_client: Arc<clwdr_client::ClowderRestClient>,
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
            signer_url: _,
            commit_repo_cfg,
        } = cfg;

        let core_client = bcr_common::client::core::Client::new(core_client_url);
        let treasury_client = bcr_common::client::treasury::Client::new(treasury_client_url);
        let clwdr_stream_client = Arc::new(
            clwdr_client::ClowderNatsClient::new(clwdr_nats_url)
                .await
                .expect("failed to init clowder nats client"),
        );
        let clwdr_rest_client = Arc::new(clwdr_client::ClowderRestClient::new(clwdr_rest_url));
        let commit_repo = persistence::surreal::DBCommitments::new(commit_repo_cfg)
            .await
            .expect("failed to init commitment repo");
        let commit_srv = Arc::new(commitment::Service {
            repo: Box::new(commit_repo),
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
    let swagger = utoipa_swagger_ui::SwaggerUi::new("/swagger-ui")
        .url("/api-docs/openapi.json", ApiDoc::openapi());

    let router = Router::new()
        .route("/health", get(web::health))
        // Cashu Endpoints
        .route("/v1/info", get(web::get_mint_info))
        .route("/v1/wildcat", get(web::get_wildcat_info))
        .route("/v1/keys", get(web::get_mint_keys))
        .route("/v1/keysets", get(web::get_mint_keysets))
        .route("/v1/keysets/{kid}", get(web::get_keyset_info))
        .route("/v1/keys/{kid}", get(web::get_mint_keyset))
        .route("/v1/swap", post(web::post_swap))
        .route("/v1/checkstate", post(web::post_check_state))
        .route("/v1/restore", post(web::post_restore))
        .route("/v1/swap/commitment", post(web::post_commit))
        // Clowder Endpoints
        .route(ClowderClient::LOCAL_INFO_EP_V1, get(web::get_clowder_info))
        .route(
            ClowderClient::LOCAL_PATH_EP_V1,
            post(web::post_clowder_path),
        )
        .route(
            ClowderClient::ONLINE_EXCHANGE_EP_V1,
            post(web::post_online_exchange),
        )
        .route(
            ClowderClient::OFFLINE_EXCHANGE_EP_V1,
            post(web::post_offline_exchange),
        )
        .route(
            ClowderClient::LOCAL_BETAS_EP_V1,
            get(web::get_clowder_betas),
        )
        .route(
            ClowderClient::FOREIGN_OFFLINE_EP_V1,
            get(web::get_foreign_offline),
        )
        .route(
            ClowderClient::FOREIGN_STATUS_EP_V1,
            get(web::get_foreign_status),
        )
        .route(
            ClowderClient::FOREIGN_SUBSTITUTE_EP_V1,
            get(web::get_foreign_substitute),
        )
        .route(
            ClowderClient::FOREIGN_KEYSETS_EP_V1,
            get(web::get_foreign_keysets),
        )
        .route(ClowderClient::LOCAL_COVERAGE_EP_V1, get(web::get_coverage))
        .with_state(app)
        .merge(swagger);
    Ok(router)
}

#[derive(utoipa::OpenApi)]
#[openapi(
    components(schemas(
        // clowder service
        wire_clowder::OfflineResponse,
        wire_clowder::AlphaStateResponse,
        wire_clowder::ConnectedMintResponse,
        wire_clowder::ConnectedMintsResponse,
        wire_clowder::PathRequest,
        wire_clowder::ClowderNodeInfo,
        wire_clowder::Coverage,
        bcr_common::wire::info::WildcatInfo,
        bcr_common::wire::info::VersionInfo,
        // exchange service
        wire_exchange::OnlineExchangeRequest,
        wire_exchange::OnlineExchangeResponse,
        wire_exchange::OfflineExchangeRequest,
        wire_exchange::OfflineExchangeResponse,
        // cashu types
        cashu::KeysResponse,
    )),
    paths(
        crate::web::health,
        crate::web::get_wildcat_info,
        crate::web::get_mint_keys,
        crate::web::get_mint_keysets,
        crate::web::get_mint_keyset,
        crate::web::post_swap,
        crate::web::post_check_state,
        crate::web::post_restore,
        crate::web::get_clowder_info,
        crate::web::post_clowder_path,
        crate::web::get_clowder_betas,
        crate::web::get_foreign_offline,
        crate::web::get_foreign_status,
        crate::web::get_foreign_substitute,
        crate::web::get_foreign_keysets,
        crate::web::post_online_exchange,
        crate::web::post_offline_exchange,
        crate::web::get_coverage,
    )
)]
struct ApiDoc;
