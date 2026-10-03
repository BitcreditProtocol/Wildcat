// ----- standard library imports
use std::{str::FromStr, sync::Arc};
// ----- extra library imports
use axum::{
    extract::FromRef,
    routing::{delete, get, patch, post},
    Router,
};
use bcr_common::{
    cashu,
    client::{
        admin::clowder::Client as ClowderClient, core::Client as CoreClient,
        ebill::Client as EBillClient, quote, treasury::Client as TreasuryClient, Url as ClientUrl,
    },
    wire::clowder as wire_clowder,
};
use bcr_wdc_utils::{routine::RoutineHandle, surreal};
// ----- local modules
mod admin;
mod client;
mod error;
mod monitor;
mod persistence;
mod quotes;
mod service;
mod web;
// ----- local imports

// ----- end imports

use bcr_common::TStamp;

pub const MINIMUM_MONITOR_INTERVAL_SECONDS: u64 = 5;

#[derive(Clone, Debug, serde::Deserialize)]
pub struct AppConfig {
    quotes: surreal::DBConnConfig,
    core_url: ClientUrl,
    core_admin_url: ClientUrl,
    treasury_admin_url: ClientUrl,
    ebill_url: ClientUrl,
    clowder_url: reqwest::Url,
    monitor_interval_seconds: u64,
}

#[derive(Clone, FromRef)]
pub struct AppController {
    quote: Arc<service::Service>,
}

pub async fn init_app(cfg: AppConfig) -> (AppController, RoutineHandle) {
    let AppConfig {
        quotes,
        core_url,
        core_admin_url,
        treasury_admin_url,
        ebill_url,
        clowder_url,
        monitor_interval_seconds,
    } = cfg;
    let quotes_repository = persistence::surreal::DBQuotes::new(quotes)
        .await
        .expect("DB connection to quotes failed");

    let clwdr_cl = ClowderClient::new(clowder_url);
    let public_key = clwdr_cl
        .get_info()
        .await
        .expect("Failed to get Clowder ID")
        .node_id;
    let wire_clowder::MintUrlResponse { mint_url, .. } = clwdr_cl
        .get_mint_url(&public_key)
        .await
        .expect("Failed to get mint URL");
    let core_public = CoreClient::new(core_url);
    let core_admin = CoreClient::new(core_admin_url);
    let treasury_cl = TreasuryClient::new(treasury_admin_url);
    let ebill = EBillClient::new(ebill_url);
    let wdc_cl = client::WildcatCl {
        core_public,
        core_admin,
        treasury: treasury_cl,
        ebill,
    };
    let cashu_mint_url =
        cashu::MintUrl::from_str(mint_url.as_ref()).expect("cashu::MintUrl == reqwest::Url");
    let quoting_service = service::Service {
        wdc_client: Box::new(wdc_cl),
        quotes: Box::new(quotes_repository),
        mint_url: cashu_mint_url,
    };
    let quote = Arc::new(quoting_service);
    let monitor = monitor::EbillMonitor {
        srvc: quote.clone(),
    };
    let interval = std::time::Duration::from_secs(std::cmp::max(
        monitor_interval_seconds,
        MINIMUM_MONITOR_INTERVAL_SECONDS,
    ));
    let routine_handle = RoutineHandle::new(monitor, interval);
    (AppController { quote }, routine_handle)
}

pub fn web_routes<Cntrlr>() -> Router<Cntrlr>
where
    Arc<service::Service>: FromRef<Cntrlr> + Send + Sync + 'static,
    Cntrlr: Send + Sync + Clone + 'static,
{
    Router::new()
        .route("/health", get(get_health))
        .route(quote::web_ep::ENQUIRE_V1, post(web::enquire_quote))
        .route(quote::web_ep::LOOKUP_V1, get(web::lookup_quote))
        .route(quote::web_ep::RESOLVE_V1, delete(web::cancel))
        .route(quote::web_ep::RESOLVE_V1, patch(web::resolve_offer))
}

pub fn admin_routes<Cntrlr>() -> Router<Cntrlr>
where
    Arc<service::Service>: FromRef<Cntrlr> + Send + Sync + 'static,
    Cntrlr: Send + Sync + Clone + 'static,
{
    Router::new()
        .route(quote::admin_ep::LIST, get(admin::list_quotes))
        .route(quote::admin_ep::LOOKUP, get(admin::lookup_quote))
        .route(quote::admin_ep::UPDATE, patch(admin::update_quote))
        .route(
            quote::admin_ep::ENABLE_MINTING,
            patch(admin::enable_minting),
        )
        .route(
            quote::admin_ep::SHARED_EBILL_HISTORY,
            get(admin::get_shared_ebill_history),
        )
}

async fn get_health() -> &'static str {
    "{ \"status\": \"OK\" }"
}

#[cfg(feature = "test-utils")]
pub mod test_utils {
    use super::*;
    use crate::{error::Error, persistence::inmemory::QuotesIDMap, service::MintingStatus};

    pub struct DummyWdcClient;

    #[async_trait::async_trait]
    impl service::WdcClient for DummyWdcClient {
        async fn get_keyset_with_expiration_date(
            &self,
            _expiration_date: time::Date,
        ) -> error::Result<cashu::Id> {
            Err(Error::InvalidInput("DummyWdcClient".into()))
        }
        async fn get_keys(
            &self,
            _keyset_id: cashu::Id,
        ) -> error::Result<bcr_common::ecash::KeySet> {
            Err(Error::InvalidInput("DummyWdcClient".into()))
        }
        async fn add_new_mint_operation(
            &self,
            _qid: uuid::Uuid,
            _kid: cashu::Id,
            _pk: cashu::PublicKey,
            _target: cashu::Amount,
            _bill_id: bcr_common::core::BillId,
        ) -> error::Result<()> {
            Err(Error::InvalidInput("DummyWdcClient".into()))
        }
        async fn sign(
            &self,
            _msgs: &[cashu::BlindedMessage],
        ) -> error::Result<Vec<cashu::BlindSignature>> {
            Err(Error::InvalidInput("DummyWdcClient".into()))
        }
        async fn get_minting_status(&self, _qid: uuid::Uuid) -> error::Result<MintingStatus> {
            Err(Error::InvalidInput("DummyWdcClient".into()))
        }
        async fn validate_and_decrypt_shared_bill(
            &self,
            _shared_bill: &bcr_common::wire::quotes::SharedBill,
        ) -> error::Result<bcr_common::wire::quotes::BillInfo> {
            Err(Error::InvalidInput("DummyWdcClient".into()))
        }
        async fn validate_endorsed_bill_matches_shared_bill(
            &self,
            _bill_id: bcr_common::core::BillId,
            _shared_bill_data: String,
        ) -> error::Result<bool> {
            Err(Error::InvalidInput("DummyWdcClient".into()))
        }
        async fn get_shared_ebill_history(
            &self,
            _bill_id: bcr_common::core::BillId,
            _shared_bill_data: String,
        ) -> error::Result<Vec<bcr_common::wire::bill::BillHistoryBlock>> {
            Err(Error::InvalidInput("DummyWdcClient".into()))
        }
        async fn get_ebill(
            &self,
            _bid: bcr_common::core::BillId,
        ) -> error::Result<bcr_common::wire::bill::BitcreditBill> {
            Err(Error::InvalidInput("DummyWdcClient".into()))
        }
        async fn collect_fees(&self, _proofs: Vec<cashu::Proof>) -> error::Result<()> {
            Err(Error::InvalidInput("DummyWdcClient".into()))
        }
    }

    pub fn test_controller() -> AppController {
        let service = service::Service {
            wdc_client: Box::new(DummyWdcClient),
            quotes: Box::new(QuotesIDMap::default()),
            mint_url: cashu::MintUrl::from_str("http://localhost:3338").expect("MintUrl"),
        };
        AppController {
            quote: Arc::new(service),
        }
    }
}
