// ----- standard library imports
use std::{str::FromStr, sync::Arc};
// ----- extra library imports
use axum::{
    extract::FromRef,
    routing::{delete, get, post},
    Router,
};
use bcr_common::{
    cashu,
    client::clowder::{ClowderNatsClient, SignatoryNatsClient},
    client::{
        admin::clowder::Client as ClowderClient, core::Client as CoreClient,
        ebill::Client as EbClient, treasury as cl_treasury,
    },
};
use bcr_wdc_utils::{nut19, routine};
// ----- local modules
mod admin;
pub mod config;
pub mod ebill;
mod error;
mod foreign;
mod onchain;
pub mod persistence;
mod vault;
mod web;
// ----- local imports

// ----- end imports

type TStamp = chrono::DateTime<chrono::Utc>;
#[derive(Clone, FromRef)]
pub struct AppController {
    ebill: Arc<ebill::Service>,
    onchain: Arc<onchain::Service>,
    foreign: Arc<foreign::Service>,
    vault: Arc<vault::Service>,
    clwdr_nats: Arc<ClowderNatsClient>,
    cache: Arc<dyn nut19::Cache>,
}

pub async fn init_app(cfg: config::App) -> (AppController, Vec<routine::RoutineHandle>) {
    let config::App {
        onchain,
        foreign,
        ebill,
        vault,
        core_url,
        ebill_url,
        clowder_rest_url,
        clowder_nats_url,
        cache_expiry_sec,
    } = cfg;

    //clients
    let core_client = Arc::new(CoreClient::new(core_url));
    let ebill_client = EbClient::new(ebill_url);
    let clowder_client = Arc::new(ClowderClient::new(clowder_rest_url));
    let nats_cl = ClowderNatsClient::new(clowder_nats_url.clone())
        .await
        .expect("Failed to create clowder nats client");
    let clowder_nats_client = Arc::new(nats_cl);
    let signer_cl = SignatoryNatsClient::new(clowder_nats_url, None)
        .await
        .expect("Failed to create signatory nats client");

    let info = clowder_client
        .get_info()
        .await
        .expect("Failed to get clowder info");
    let my_pk = secp256k1::PublicKey::from_slice(&info.node_id.to_bytes())
        .expect("secp256k1::PublicKey == cashu::PublicKey");

    // onChain
    let config::Onchain {
        db: onchain_repo,
        monitor_interval_sec,
        quote_expiry_seconds,
        min_confirmations,
        min_melt_threshold,
        min_mint_threshold,
    } = onchain;
    let onchain_repo = persistence::surreal::DBOnChain::new(onchain_repo)
        .await
        .expect("Failed to create repository");
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
        alpha_id: my_pk,
    };

    // eBill
    let config::Ebill { db: mintops, .. } = ebill;
    let ebill_repo = persistence::surreal::DBEbill::new(mintops)
        .await
        .expect("Failed to create ebill repository");
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
    let config::Foreign {
        online_repo,
        offline_repo,
    } = foreign;
    let foreign_online_repo = persistence::surreal::DBForeignOnline::new(online_repo)
        .await
        .expect("Failed to create foreign online repository");
    let foreign_offline_repo = persistence::surreal::DBForeignOffline::new(offline_repo)
        .await
        .expect("Failed to create foreign offline repository");
    let onlinerepo = Arc::new(foreign_online_repo);
    let offlinerepo = Arc::new(foreign_offline_repo);
    let clowder = Arc::new(foreign::clients::ClowderCl {
        rest: clowder_client.clone(),
        stream: clowder_nats_client.clone(),
        signatory: Box::new(signer_cl),
    });
    let factory = Arc::new(foreign::clients::MintClientFactory {
        my_pk,
        clwdr: clowder_client.clone(),
    });
    let foreigncore = Arc::new(foreign::clients::CoreCl {
        core: core_client.clone(),
    });
    let foreign = foreign::Service {
        online_repo: onlinerepo.clone(),
        offline_repo: offlinerepo.clone(),
        keys: foreigncore.clone(),
        clowder: clowder.clone(),
        mint_factory: factory.clone(),
    };

    // vault
    let config::Vault { db } = vault;
    let vault_repo = persistence::surreal::DBVault::new(db)
        .await
        .expect("Failed to create vault repository");
    let wdccl = vault::WildcatCl {
        core: core_client.clone(),
    };
    let url_response = clowder_client
        .get_mint_url(&my_pk)
        .await
        .expect("Failed to get mint url");
    let my_url = cashu::MintUrl::from_str(url_response.mint_url.as_str())
        .expect("cashu::MintUrl == reqwest::Url");
    let vault = vault::Service {
        repo: Box::new(vault_repo),
        wdc_cl: Box::new(wdccl),
        my_url,
    };

    // cache
    let cache_expiry = chrono::Duration::seconds(cache_expiry_sec as i64);
    let cache = Arc::new(nut19::InMemoryMap::new(cache_expiry));
    let app_ctrl = AppController {
        ebill: Arc::new(ebill),
        onchain: Arc::new(onchain),
        foreign: Arc::new(foreign),
        vault: Arc::new(vault),
        clwdr_nats: clowder_nats_client,
        cache,
    };

    // monitors
    let monitor_interval = std::time::Duration::from_secs(monitor_interval_sec as u64);
    let monitors = vec![
        routine::RoutineHandle::new(
            onchain::MintOpMonitor {
                srvc: app_ctrl.onchain.clone(),
            },
            monitor_interval,
        ),
        routine::RoutineHandle::new(
            foreign::settle::Handler {
                online: onlinerepo,
                offline: offlinerepo,
                clowder,
                mint_factory: factory,
            },
            monitor_interval,
        ),
    ];
    (app_ctrl, monitors)
}

pub fn routes(app: AppController) -> Router {
    let web = Router::new()
        .route(
            cl_treasury::web_ep::EXCHANGE_ONLINE_V1,
            post(web::online_exchange),
        )
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
            cl_treasury::admin_ep::REQUEST_TO_PAY_EBILL,
            post(admin::request_to_pay_ebill),
        )
        .route(
            cl_treasury::admin_ep::TRY_HTLC_SWAP,
            post(admin::try_htlc_swap),
        )
        .route(
            cl_treasury::admin_ep::NEW_EBILL_MINTOP,
            post(admin::new_ebill_mintop),
        )
        .route(
            cl_treasury::admin_ep::LIST_EBILL_MINTOPS,
            get(admin::list_ebill_mintops),
        )
        .route(
            cl_treasury::admin_ep::EBILL_MINTOP_STATUS,
            get(admin::ebill_mintop_status),
        )
        .route(
            cl_treasury::admin_ep::FEES_STORE_PROOFS,
            post(admin::store_fees_proofs),
        )
        .route(
            cl_treasury::admin_ep::FEES_TOKEN,
            get(admin::generate_fees_token),
        )
        .route(
            cl_treasury::admin_ep::DENIED_MELTOPS,
            get(admin::list_denied_meltops),
        )
        .route(
            cl_treasury::admin_ep::DENIED_MELTOP,
            delete(admin::delete_denied_meltop),
        );
    admin.merge(web).with_state(app)
}
