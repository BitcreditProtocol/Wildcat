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
        admin::clowder::Client as ClowderClient, core::Client as CoreClient,
        ebill::Client as EbClient, treasury as cl_treasury, Url as ClientUrl,
    },
    clwdr_client::ClowderNatsClient,
};
use bcr_wdc_utils::{routine, surreal};
// ----- local modules
mod admin;
mod devmode;
mod ebill;
mod error;
mod foreign;
mod onchain;
mod persistence;
mod web;
// ----- local imports

// ----- end imports

type TStamp = chrono::DateTime<chrono::Utc>;

#[derive(Clone, Debug, serde::Deserialize)]
pub struct AppConfig {
    onchain: OnchainConfig,
    foreign: ForeignConfig,
    ebill: EbillConfig,
    core_url: ClientUrl,
    ebill_url: ClientUrl,
    clowder_rest_url: reqwest::Url,
    clowder_nats_url: reqwest::Url,
}

#[derive(Clone, Debug, serde::Deserialize)]
pub struct OnchainConfig {
    db: surreal::DBConnConfig,
    monitor_interval_sec: u32,
    quote_expiry_seconds: u32,
    min_confirmations: u32,
    min_melt_threshold: bitcoin::Amount,
    min_mint_threshold: bitcoin::Amount,
}

#[derive(Clone, Debug, serde::Deserialize)]
pub struct ForeignConfig {
    crsatonline_repo: surreal::DBConnConfig,
    crsatoffline_repo: surreal::DBConnConfig,
}

#[derive(Clone, Debug, serde::Deserialize)]
pub struct EbillConfig {
    db: surreal::DBConnConfig,
}

#[derive(Clone, FromRef)]
pub struct AppController {
    ebill: Arc<ebill::Service>,
    onchain: Arc<onchain::Service>,
    foreign: Arc<foreign::crsat::Service>,
    dev: Arc<devmode::Service>,
    clwdr_nats: Arc<ClowderNatsClient>,
}

pub async fn init_app(cfg: AppConfig) -> (AppController, Vec<routine::RoutineHandle>) {
    let AppConfig {
        onchain,
        foreign,
        ebill,
        core_url,
        ebill_url,
        clowder_rest_url,
        clowder_nats_url,
    } = cfg;
    let OnchainConfig {
        db: onchain_repo,
        monitor_interval_sec,
        quote_expiry_seconds,
        min_confirmations,
        min_melt_threshold,
        min_mint_threshold,
    } = onchain;
    let EbillConfig { db: mintops } = ebill;
    let ForeignConfig {
        crsatonline_repo,
        crsatoffline_repo,
    } = foreign;

    //clients
    let core_client = Arc::new(CoreClient::new(core_url));
    let ebill_client = EbClient::new(ebill_url);
    let clowder_client = Arc::new(ClowderClient::new(clowder_rest_url));
    let nats_cl = ClowderNatsClient::new(clowder_nats_url)
        .await
        .expect("Failed to create clowder nats client");
    let clowder_nats_client = Arc::new(nats_cl);

    // repositories
    let onchain_repo = persistence::surreal::DBOnChain::new(onchain_repo)
        .await
        .expect("Failed to create repository");
    let ebill_repo = persistence::surreal::DBEbill::new(mintops)
        .await
        .expect("Failed to create mintops repository");
    let foreign_online_repo = persistence::surreal::DBForeignOnline::new(crsatonline_repo)
        .await
        .expect("Failed to create foreign online repository");
    let foreign_offline_repo = persistence::surreal::DBForeignOffline::new(crsatoffline_repo)
        .await
        .expect("Failed to create foreign offline repository");

    // onChain
    let clowder_cl = onchain::ClowderCl {
        rest: clowder_client.clone(),
        nats: clowder_nats_client.clone(),
        min_confirmations,
    };
    let wdc = onchain::WildcatCl {
        core_cl: core_client.clone(),
    };
    let onchain = onchain::Service {
        quote_expiry: chrono::Duration::seconds(quote_expiry_seconds as i64),
        wdc: Arc::new(wdc),
        repo: Arc::new(onchain_repo),
        clowder_cl: Arc::new(clowder_cl),
        min_mint_threshold,
        min_melt_threshold,
    };

    // eBill
    let wdccl = ebill::WildcatCl {
        core: core_client.clone(),
        ebill: Box::new(ebill_client),
    };
    let clwdcl = ebill::ClwdrCl {
        rest: clowder_client.clone(),
        nats: clowder_nats_client.clone(),
    };
    let ebill = ebill::Service {
        repo: Box::new(ebill_repo),
        wildcatcl: Box::new(wdccl),
        clowdercl: Box::new(clwdcl),
    };

    // foreign
    let info = clowder_client
        .get_info()
        .await
        .expect("Failed to get clowder info");
    let my_pk = secp256k1::PublicKey::from_slice(&info.node_id.to_bytes())
        .expect("secp256k1::PublicKey == cashu::PublicKey");
    let clowder = Arc::new(foreign::clients::ClowderCl {
        clwdr: clowder_client.clone(),
    });
    let factory = Arc::new(foreign::clients::MintClientFactory { my_pk });
    let crsatonlinerepo = Arc::new(foreign_online_repo);
    let crsatofflinerepo = Arc::new(foreign_offline_repo);
    let crsatcore = Arc::new(foreign::clients::CoreCl {
        core: core_client.clone(),
    });
    let interval = std::time::Duration::from_secs(monitor_interval_sec as u64);
    let settler = {
        let online: Arc<dyn foreign::OnlineRepository> = crsatonlinerepo.clone();
        let offline: Arc<dyn foreign::OfflineRepository> = crsatofflinerepo.clone();
        let clwdr: Arc<dyn foreign::ClowderClient> = clowder.clone();
        let fctry: Arc<dyn foreign::MintClientFactory> = factory.clone();
        Box::new(foreign::settle::Handler::new(
            &online, &offline, &clwdr, &fctry, interval,
        ))
    };
    let foreign = foreign::crsat::Service {
        online_repo: crsatonlinerepo,
        offline_repo: crsatofflinerepo,
        keys: crsatcore.clone(),
        clowder: clowder.clone(),
        mint_factory: factory.clone(),
        settler,
    };

    let dev = devmode::Service {
        crcore: core_client.clone(),
    };
    let app_ctrl = AppController {
        ebill: Arc::new(ebill),
        onchain: Arc::new(onchain),
        foreign: Arc::new(foreign),
        dev: Arc::new(dev),
        clwdr_nats: clowder_nats_client,
    };

    let monitor_interval = std::time::Duration::from_secs(monitor_interval_sec as u64);
    let monitors = vec![routine::RoutineHandle::new(
        onchain::MintOpMonitor {
            srvc: app_ctrl.onchain.clone(),
        },
        monitor_interval,
    )];
    (app_ctrl, monitors)
}

pub fn routes(app: AppController) -> Router {
    let web = Router::new()
        .route(
            cl_treasury::web_ep::EXCHANGE_ONLINE_V1,
            post(web::online_exchange),
        )
        .route("/v1/free_money", post(devmode::free_money))
        .route(
            cl_treasury::web_ep::EXCHANGE_OFFLINE_V1,
            post(web::offline_exchange),
        )
        .route(
            cl_treasury::web_ep::MELTQUOTE_ONCHAIN_V1,
            post(web::melt_quote_onchain),
        )
        .route(
            cl_treasury::web_ep::MELT_ONCHAIN_V1,
            post(web::melt_onchain),
        )
        .route(
            cl_treasury::web_ep::MINTQUOTE_ONCHAIN_V1,
            post(web::mint_quote_onchain),
        )
        .route(
            cl_treasury::web_ep::MINT_ONCHAIN_V1,
            post(web::mint_onchain),
        )
        .route(cl_treasury::web_ep::EBILLMINT_V1, post(web::mint_ebill));
    let admin = Router::new()
        .route(
            cl_treasury::admin_ep::REQUEST_TO_PAY_EBILL_V1,
            post(admin::request_to_pay_ebill),
        )
        .route(
            cl_treasury::admin_ep::TRY_HTLC_SWAP_V1,
            post(admin::try_htlc_swap),
        )
        .route(
            cl_treasury::admin_ep::NEW_EBILL_MINTOP_V1,
            post(admin::new_ebill_mintop),
        )
        .route(
            cl_treasury::admin_ep::LIST_EBILL_MINTOPS_V1,
            get(admin::list_ebill_mintops),
        )
        .route(
            cl_treasury::admin_ep::EBILL_MINTOP_STATUS_V1,
            get(admin::ebill_mintop_status),
        );
    admin.merge(web).with_state(app)
}
