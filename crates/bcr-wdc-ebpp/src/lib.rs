// ----- standard library imports
use std::sync::Arc;
// ----- extra library imports
use anyhow::Result as AnyResult;
use axum::{
    extract::FromRef,
    routing::{get, Router},
};
use bcr_wdc_webapi::wallet::Balance;
use bdk_wallet::{bitcoin as btc, miniscript::ToPublicKey};
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
pub type ProdOnChainElectrumApi = bdk_electrum::electrum_client::Client;
pub type ProdOnChainWallet = onchain::Wallet<ProdPrivateKeysRepository, ProdOnChainElectrumApi>;
pub type ProdService =
    service::Service<ProdOnChainWallet, ProdPaymentRepository, ebill::EBillClient>;

#[serde_as]
#[derive(Clone, Debug, serde::Deserialize)]
pub struct AppConfig {
    grpc_address: std::net::SocketAddr,
    onchain: onchain::WalletConfig,
    private_keys: persistence::surreal::ConnectionConfig,
    payments: persistence::surreal::PaymentConnectionConfig,
    ebill_client: ebill::EBillClientConfig,
    electrum_url: String,
    #[serde_as(as = "serde_with::DurationSeconds<i64>")]
    refresh_interval_secs: chrono::Duration,
    treasury_service_public_key: btc::PublicKey,
}

#[derive(Clone, FromRef)]
pub struct AppController {
    srvc: Arc<ProdService>,
    grpc_address: std::net::SocketAddr,
}

impl AppController {
    pub async fn new(seed: &[u8], cfg: AppConfig) -> Self {
        let AppConfig {
            grpc_address,
            onchain,
            private_keys,
            payments,
            ebill_client,
            electrum_url,
            refresh_interval_secs: refresh_interval,
            treasury_service_public_key,
        } = cfg;

        let key_repo = ProdPrivateKeysRepository::new(private_keys)
            .await
            .expect("private keys repo");
        let electrum_client = bdk_electrum::electrum_client::Client::new(&electrum_url)
            .expect("electrum_client::Client::new");
        let onchain_wallet = ProdOnChainWallet::new(seed, onchain, key_repo, electrum_client)
            .await
            .expect("onchain wallet");

        let payrepo = ProdPaymentRepository::new(payments, onchain_wallet.network())
            .await
            .expect("payment repo");

        let ebillnode = ebill::EBillClient::new(ebill_client);

        let refresh_interval = refresh_interval
            .to_std()
            .expect("refresh_interval conversion");

        let processor = ProdService::new(
            onchain_wallet,
            payrepo,
            ebillnode,
            refresh_interval,
            treasury_service_public_key.to_x_only_pubkey(),
        )
        .await;

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
