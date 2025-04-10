// ----- standard library imports
use std::sync::Arc;
// ----- extra library imports
use anyhow::Result as AnyResult;
use axum::{
    extract::FromRef,
    routing::{get, Router},
};
use bcr_wdc_webapi::wallet::Balance;
use bdk_esplora::esplora_client::AsyncClient;
use cdk_payment_processor::PaymentProcessorServer;
use serde_with::serde_as;
use utoipa::OpenApi;
// ----- local modules
mod ebill;
mod error;
mod onchain;
mod payment;
mod persistence;
mod service;
mod web;

// ----- end imports

pub type ProdPrivateKeysRepository = persistence::surreal::DBPrivateKeys;
pub type ProdPaymentRepository = persistence::surreal::DBPayments;
pub type ProdOnChainSyncer = AsyncClient;
pub type ProdOnChainWallet = onchain::Wallet<ProdPrivateKeysRepository, ProdOnChainSyncer>;
pub type ProdService =
    service::Service<ProdOnChainWallet, ProdPaymentRepository, ebill::DummyEbillNode>;

#[serde_as]
#[derive(Clone, Debug, serde::Deserialize)]
pub struct AppConfig {
    grpc_address: std::net::SocketAddr,
    onchain: onchain::WalletConfig,
    private_keys: persistence::surreal::ConnectionConfig,
    payments: persistence::surreal::ConnectionConfig,
    esplora_url: String,
    #[serde_as(as = "serde_with::DurationSeconds<i64>")]
    refresh_interval: chrono::Duration,
}

#[derive(Clone, FromRef)]
pub struct AppController {
    srvc: Arc<ProdService>,
    grpc_address: std::net::SocketAddr,
}

impl AppController {
    pub async fn new(cfg: AppConfig) -> Self {
        let AppConfig {
            grpc_address,
            onchain,
            private_keys,
            payments,
            esplora_url,
            refresh_interval,
        } = cfg;

        let key_repo = ProdPrivateKeysRepository::new(private_keys)
            .await
            .expect("private keys repo");
        let client = reqwest::Client::new();
        let esplora_client: AsyncClient =
            bdk_esplora::esplora_client::AsyncClient::from_client(esplora_url, client);
        let onchain_wallet = ProdOnChainWallet::new(onchain, key_repo, esplora_client)
            .await
            .expect("onchain wallet");

        let payrepo = ProdPaymentRepository::new(payments, onchain_wallet.network())
            .await
            .expect("payment repo");

        let ebillnode = ebill::DummyEbillNode {};

        let refresh_interval = refresh_interval
            .to_std()
            .expect("refresh_interval conversion");

        let processor =
            ProdService::new(onchain_wallet, payrepo, ebillnode, refresh_interval).await;

        Self {
            srvc: Arc::new(processor),
            grpc_address,
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
