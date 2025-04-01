// ----- standard library imports
use std::sync::Arc;
// ----- extra library imports
use anyhow::Result as AnyResult;
use axum::{
    extract::FromRef,
    routing::{get, Router},
};
use bcr_wdc_webapi::wallet::Balance;
use cdk_payment_processor::PaymentProcessorServer;
use utoipa::OpenApi;
// ----- local modules
mod bip39;
mod error;
mod service;
mod web;

// ----- end imports

pub type ProdBip39Wallet = bip39::Wallet;
pub type ProdService = service::Service<ProdBip39Wallet>;

#[derive(Clone, Debug, serde::Deserialize)]
pub struct AppConfig {
    grpc_address: std::net::SocketAddr,
    onchain: bip39::WalletConfig,
}

#[derive(Clone, FromRef)]
pub struct AppController {
    srvc: Arc<ProdService>,
    grpc_address: std::net::SocketAddr,
}

impl AppController {
    pub async fn new(cfg: AppConfig) -> Self {
        let onchain_wallet = ProdBip39Wallet::new(cfg.onchain).expect("onchain wallet");
        let processor = ProdService::new(onchain_wallet).await;

        Self {
            srvc: Arc::new(processor),
            grpc_address: cfg.grpc_address,
        }
    }

    pub async fn new_grpc_server(&self) -> AnyResult<PaymentProcessorServer> {
        let ip = self.grpc_address.ip().to_string();
        let port = self.grpc_address.port();
        PaymentProcessorServer::new(self.srvc.clone(), &ip, port)
    }
}

pub fn routes(ctrl: AppController) -> Router {
    let swagger = utoipa_swagger_ui::SwaggerUi::new("/swagger-ui")
        .url("/api-docs/openapi.json", ApiDoc::openapi());
    let web = Router::new().route("/v1/onchain/balance", get(web::balance));
    Router::new().merge(web).with_state(ctrl).merge(swagger)
}

#[derive(utoipa::OpenApi)]
#[openapi(components(schemas(Balance),), paths(web::balance))]
struct ApiDoc;
