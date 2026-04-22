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
        core::Client as CoreClient, ebill::Client as EbClient, mint::Client as MintClient,
        treasury::Client as TreasuryClient, Url as ClientUrl,
    },
    clwdr_client::{ClowderNatsClient, ClowderRestClient},
};
use bcr_wdc_utils::{routine, surreal};
use bitcoin::secp256k1;
// ----- local modules
mod admin;
mod devmode;
mod ebill;
mod error;
mod foreign;
mod monitor;
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
    satonline_repo: surreal::DBConnConfig,
    satoffline_repo: surreal::DBConnConfig,
}

#[derive(Clone, Debug, serde::Deserialize)]
pub struct EbillConfig {
    db: surreal::DBConnConfig,
}

#[derive(Clone, FromRef)]
pub struct AppController {
    ebill: Arc<ebill::Service>,
    onchain: Arc<onchain::Service>,
    crsat: Arc<foreign::crsat::Service>,
    sat: Arc<foreign::sat::Service>,
    clwdr_nats: Arc<ClowderNatsClient>,
    clwdr_rest: Arc<ClowderRestClient>,
    dev: Arc<devmode::Service>,
}

pub async fn init_app(
    secret: secp256k1::SecretKey,
    cfg: AppConfig,
) -> (AppController, Vec<routine::RoutineHandle>) {
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
    let ForeignConfig {
        crsatonline_repo,
        crsatoffline_repo,
        satonline_repo,
        satoffline_repo,
    } = foreign;
    let EbillConfig { db: mintops } = ebill;

    //clients
    let core_client = Arc::new(CoreClient::new(core_url));
    let ebill_client = EbClient::new(ebill_url);
    let clowder_rest_client = Arc::new(ClowderRestClient::new(clowder_rest_url));
    let nats_cl = ClowderNatsClient::new(clowder_nats_url)
        .await
        .expect("Failed to create clowder nats client");
    let clowder_nats_client = Arc::new(nats_cl);

    let wdc = onchain::WildcatCl {
        core_cl: core_client.clone(),
    };
    // repositories
    let repo = persistence::surreal::DBOnChain::new(onchain_repo)
        .await
        .expect("Failed to create repository");
    let signing_keys = secp256k1::Keypair::from_secret_key(secp256k1::global::SECP256K1, &secret);
    tracing::info!("signing public key: {}", signing_keys.public_key());
    let clowder_cl = onchain::ClowderCl {
        rest: clowder_rest_client.clone(),
        nats: clowder_nats_client.clone(),
        min_confirmations,
    };
    let onchain = onchain::Service {
        quote_expiry: chrono::Duration::seconds(quote_expiry_seconds as i64),
        wdc: Arc::new(wdc),
        repo: Arc::new(repo),
        clowder_cl: Arc::new(clowder_cl),
        min_mint_threshold,
        min_melt_threshold,
    };

    let crsatonlinerepo = Arc::new(
        persistence::surreal::DBForeignOnline::new(crsatonline_repo)
            .await
            .expect("Failed to create crsat online repository"),
    );
    let crsatofflinerepo = Arc::new(
        persistence::surreal::DBForeignOffline::new(crsatoffline_repo)
            .await
            .expect("Failed to create crsat offline repository"),
    );
    let crsatcore = Arc::new(foreign::clients::CoreCl {
        core: core_client.clone(),
    });
    let clowder = Arc::new(foreign::clients::ClowderCl {
        clwdr: clowder_rest_client.clone(),
    });
    let factory = Arc::new(foreign::clients::MintClientFactory {});
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
    let crsat = Arc::new(foreign::crsat::Service {
        online_repo: crsatonlinerepo,
        offline_repo: crsatofflinerepo,
        keys: crsatcore.clone(),
        clowder: clowder.clone(),
        mint_factory: factory.clone(),
        settler,
    });

    let satonlinerepo = Arc::new(
        persistence::surreal::DBForeignOnline::new(satonline_repo)
            .await
            .expect("Failed to create sat repository"),
    );
    let satofflinerepo = Arc::new(
        persistence::surreal::DBForeignOffline::new(satoffline_repo)
            .await
            .expect("Failed to create sat offline repository"),
    );
    let settler = {
        let online: Arc<dyn foreign::OnlineRepository> = satonlinerepo.clone();
        let offline: Arc<dyn foreign::OfflineRepository> = satofflinerepo.clone();
        let clwdr: Arc<dyn foreign::ClowderClient> = clowder.clone();
        let fctry: Arc<dyn foreign::MintClientFactory> = factory.clone();
        Box::new(foreign::settle::Handler::new(
            &online, &offline, &clwdr, &fctry, interval,
        ))
    };
    let sat = Arc::new(foreign::sat::Service {
        keys: crsatcore,
        online_repo: satonlinerepo,
        offline_repo: satofflinerepo,
        clowder,
        mint_factory: factory,
        settler,
    });

    let dev = devmode::Service {
        crcore: core_client.clone(),
    };
    let ebill_repo = persistence::surreal::DBEbill::new(mintops)
        .await
        .expect("Failed to create mintops repository");
    let wdccl = ebill::WildcatCl {
        core: core_client.clone(),
        ebill: Box::new(ebill_client),
    };
    let clowdercl =
        ebill::new_clowder_client(clowder_nats_client.clone(), clowder_rest_client.clone());
    let ebill = ebill::Service {
        repo: Box::new(ebill_repo),
        wildcatcl: Box::new(wdccl),
        clowdercl,
    };
    let app_ctrl = AppController {
        ebill: Arc::new(ebill),
        onchain: Arc::new(onchain),
        crsat,
        sat,
        clwdr_rest: clowder_rest_client.clone(),
        clwdr_nats: clowder_nats_client.clone(),
        dev: Arc::new(dev),
    };

    let monitor_interval = std::time::Duration::from_secs(monitor_interval_sec as u64);
    let monitors = vec![routine::RoutineHandle::new(
        monitor::OnChainMintOpMonitor {
            srvc: app_ctrl.onchain.clone(),
        },
        monitor_interval,
    )];
    (app_ctrl, monitors)
}

pub fn routes(app: AppController) -> Router {
    let web = Router::new()
        .route(MintClient::EXCHANGEONLINE_EP_V1, post(web::online_exchange))
        .route("/v1/free_money", post(devmode::free_money))
        .route(
            MintClient::EXCHANGEOFFLINE_EP_V1,
            post(web::offline_exchange),
        )
        .route(
            MintClient::MELTQUOTE_ONCHAIN_EP_V1,
            post(web::melt_quote_onchain),
        )
        .route(MintClient::MELT_ONCHAIN_EP_V1, post(web::melt_onchain))
        .route(
            MintClient::MINTQUOTE_ONCHAIN_EP_V1,
            post(web::mint_quote_onchain),
        )
        .route(MintClient::EBILLMINT_EP_V1, post(web::mint_ebill));
    let admin = Router::new()
        .route(
            TreasuryClient::REQTOPAY_EP_V1,
            post(admin::request_to_pay_ebill),
        )
        .route(TreasuryClient::TRYHTLC_EP_V1, post(admin::try_htlc_swap))
        .route(
            TreasuryClient::NEWEBILLMINTOP_EP_V1,
            post(admin::new_ebill_mintop),
        )
        .route(
            TreasuryClient::LISTEBILLMINTOPS_EP_V1,
            get(admin::list_ebill_mintops),
        )
        .route(
            TreasuryClient::EBILLMINTOPSTATUS_EP_V1,
            get(admin::ebill_mintop_status),
        );
    admin.merge(web).with_state(app)
}
