// ----- standard library imports
use std::sync::Arc;
// ----- extra library imports
use anyhow::Result as AnyResult;
use axum::{
    extract::FromRef,
    routing::{get, Router},
};
use bcr_common::cdk_payment_processor::PaymentProcessorServer;
use bcr_wdc_utils::surreal;
use bdk_wallet::{bitcoin as btc, miniscript::ToPublicKey};
use serde_with::serde_as;
// ----- local modules
mod admin;
mod ebill;
mod error;
mod onchain;
mod payment;
mod persistence;
mod service;
mod web;

// ----- end imports

pub type TStamp = chrono::DateTime<chrono::Utc>;

pub type ProdPrivateKeysRepository = persistence::surreal::DBPrivateKeys;
pub type ProdPaymentRepository = persistence::surreal::DBPayments;
pub type ProdOnChainElectrumApi = bdk_electrum::electrum_client::Client;
pub type ProdOnChainWallet = onchain::Wallet<ProdOnChainElectrumApi>;
pub type ProdService = service::Service;

#[serde_as]
#[derive(Clone, Debug, serde::Deserialize)]
pub struct AppConfig {
    grpc_address: std::net::SocketAddr,
    onchain: onchain::WalletConfig,
    private_keys: surreal::DBConnConfig,
    payments: surreal::DBConnConfig,
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
        let onchain_wallet =
            ProdOnChainWallet::new(seed, onchain, Box::new(key_repo), electrum_client)
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
            Arc::new(onchain_wallet),
            Arc::new(payrepo),
            Arc::new(ebillnode),
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
    let admin = Router::new().nest(
        "/v1/admin/ebpp/",
        Router::new().route("/onchain/balance", get(admin::balance)),
    );
    let web = Router::new().route("/health", get(get_health)).nest(
        "/v1/ebpp",
        Router::new().route("/onchain/network", get(web::network)),
    );
    Router::new().merge(admin).merge(web).with_state(ctrl)
}

async fn get_health() -> &'static str {
    "{ \"status\": \"OK\" }"
}
