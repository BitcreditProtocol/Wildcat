// ----- standard library imports
use std::sync::Arc;
// ----- extra library imports
use axum::{
    extract::FromRef,
    routing::{get, post},
    Router,
};
use cashu::mint_url::MintUrl;
use cdk::{wallet::MintConnector, HttpClient};
use utoipa::OpenApi;
// ----- local modules
pub mod built_info {
    // The file has been placed there by the build script.
    include!(concat!(env!("OUT_DIR"), "/built.rs"));
}
mod commitment;
mod error;
mod persistence;
mod signer;
mod web;
// ----- local imports
use error::Result;

// ----- end imports

pub type TStamp = chrono::DateTime<chrono::Utc>;

#[derive(Clone, Debug, serde::Deserialize)]
pub struct AppConfig {
    cdk_mint_url: MintUrl,
    keys_client_url: reqwest::Url,
    swap_client_url: reqwest::Url,
    treasury_client_url: bcr_wdc_treasury_client::Url,
    ebpp_client_url: bcr_wdc_ebpp_client::Url,
    clwdr_nats_url: Option<reqwest::Url>,
    clwdr_rest_url: Option<reqwest::Url>,
    signer_url: reqwest::Url,
    commit_repo_cfg: persistence::surreal::DBCommitmentsConnectionConfig,
}

#[derive(Clone, FromRef)]
pub struct AppController {
    cdk_client: cdk::wallet::HttpClient,
    keys_client: bcr_common::client::keys::Client,
    swap_client: bcr_common::client::swap::Client,
    treasury_client: bcr_wdc_treasury_client::TreasuryClient,
    ebpp_client: bcr_wdc_ebpp_client::EBPPClient,
    clwdr_stream_client: Option<Arc<clwdr_client::ClowderNatsClient>>,
    clwdr_rest_client: Option<Arc<clwdr_client::ClowderRestClient>>,
    commit_srv: Arc<commitment::Service>,
}

impl AppController {
    pub async fn new(cfg: AppConfig) -> error::Result<Self> {
        let AppConfig {
            cdk_mint_url,
            keys_client_url,
            swap_client_url,
            treasury_client_url,
            ebpp_client_url,
            clwdr_nats_url,
            clwdr_rest_url,
            signer_url,
            commit_repo_cfg,
        } = cfg;

        let cdk_client = HttpClient::new(cdk_mint_url);
        let keys_client = bcr_common::client::keys::Client::new(keys_client_url);
        let swap_client = bcr_common::client::swap::Client::new(swap_client_url);
        let treasury_client = bcr_wdc_treasury_client::TreasuryClient::new(treasury_client_url);
        let ebpp_client = bcr_wdc_ebpp_client::EBPPClient::new(ebpp_client_url);

        let clwdr_stream_client = if let Some(url) = clwdr_nats_url {
            Some(Arc::new(
                clwdr_client::ClowderNatsClient::new(url, false).await?,
            ))
        } else {
            None
        };

        let clwdr_rest_client =
            clwdr_rest_url.map(|url| Arc::new(clwdr_client::ClowderRestClient::new(url)));

        let commit_repo = persistence::surreal::DBCommitments::new(commit_repo_cfg)
            .await
            .expect("failed to init commitment repo");
        let signer = signer::ClowderSigner::new(signer_url)
            .await
            .expect("failed to init signer");
        let commit_srv = Arc::new(commitment::Service {
            repo: Box::new(commit_repo),
            signer: Box::new(signer),
        });

        Ok(Self {
            cdk_client,
            keys_client,
            swap_client,
            treasury_client,
            ebpp_client,
            clwdr_stream_client,
            clwdr_rest_client,
            commit_srv,
        })
    }
}

pub async fn routes(app: AppController) -> Result<Router> {
    let swagger = utoipa_swagger_ui::SwaggerUi::new("/swagger-ui")
        .url("/api-docs/openapi.json", ApiDoc::openapi());

    // WARNING: big hack: send active keyset in cdk-mint to clowder
    const ATTEMPTS: usize = 5;
    for _ in 0..ATTEMPTS {
        let res = app.cdk_client.get_mint_info().await;
        if res.is_ok() {
            tracing::debug!("cdk-mint is up");
            break;
        }
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    }
    // hopefully cdk-mint is up by now
    if let Some(clwdr) = &app.clwdr_stream_client {
        let keyset_lists = app.cdk_client.get_mint_keysets().await?;
        for info in keyset_lists.keysets {
            if info.active {
                let keyset = app.cdk_client.get_mint_keyset(info.id).await?;
                tracing::debug!("posting active keyset to clowder {}", info.id);
                clwdr.post_keyset(keyset).await?;
            }
        }
    }

    let router = Router::new()
        .route("/health", get(web::health))
        // Cashu Endpoints
        .route("/v1/info", get(web::get_mint_info))
        .route("/v1/keys", get(web::get_mint_keys))
        .route("/v1/keysets", get(web::get_mint_keysets))
        .route("/v1/keysets/{kid}", get(web::get_keyset_info))
        .route("/v1/keys/{kid}", get(web::get_mint_keyset))
        .route("/v1/swap", post(web::post_swap))
        .route("/v1/checkstate", post(web::post_check_state))
        .route("/v1/restore", post(web::post_restore))
        .route("/v1/commitment", post(web::post_commit))
        // Clowder Endpoints
        .route("/v1/id", get(web::get_clowder_id))
        .route("/v1/path", post(web::post_clowder_path))
        .route("/v1/exchange/online", post(web::post_online_exchange))
        .route("/v1/exchange/offline", post(web::post_offline_exchange))
        .route("/v1/betas", get(web::get_clowder_betas))
        .with_state(app)
        .merge(swagger);
    Ok(router)
}

#[derive(utoipa::OpenApi)]
#[openapi(paths(
    crate::web::health,
    crate::web::get_mint_keys,
    crate::web::get_mint_keysets,
    crate::web::get_mint_keyset,
    crate::web::post_swap,
    crate::web::post_check_state,
    crate::web::post_restore,
))]
struct ApiDoc;
