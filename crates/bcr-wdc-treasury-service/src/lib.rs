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
use bcr_wdc_utils::{db as db_utils, nut19, routine};
// ----- local modules
mod admin;
pub mod config;
pub mod ebill;
mod error;
pub mod foreign;
pub mod onchain;
pub mod persistence;
pub mod vault;
mod web;
// ----- local imports

// ----- end imports

use bcr_common::TStamp;

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
        repository_new,
        core_url,
        ebill_url,
        clowder_rest_url,
        clowder_nats_url,
        clowder_nkey_seed,
        cache_expiry_sec,
    } = cfg;

    // one PostgreSQL pool shared by every repository
    let pg_pool = if repository_new.max_connections > 0 {
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(repository_new.max_connections)
            .connect(&repository_new.connection)
            .await
            .expect("Failed to connect to PostgreSQL");
        Some(pool)
    } else {
        None
    };

    //clients
    let core_client = Arc::new(CoreClient::new(core_url));
    let ebill_client = EbClient::new(ebill_url);
    let clowder_client = Arc::new(ClowderClient::new(clowder_rest_url));
    let nkey_seed = clowder_nkey_seed.as_deref();
    let nats_cl = ClowderNatsClient::new(clowder_nats_url.clone(), nkey_seed)
        .await
        .expect("Failed to create clowder nats client");
    let clowder_nats_client = Arc::new(nats_cl);
    let signer_cl = SignatoryNatsClient::new(clowder_nats_url, None, nkey_seed)
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
        mint_quote_expiry_seconds,
        melt_quote_expiry_seconds,
        min_confirmations,
        melt_fee_ppk,
        min_mint_threshold,
        min_feerate_sat_per_vb,
        ..
    } = onchain;
    let surreal_onchain = persistence::surreal::DBOnChain::new(onchain_repo)
        .await
        .expect("Failed to create repository");
    let migrated = surreal_onchain
        .is_migrated()
        .await
        .expect("Failed to read onchain migration marker");
    let onchain_repo: Arc<dyn onchain::Repository> =
        match db_utils::select_db_backend("treasury onchain", migrated, repository_new.clone()) {
            db_utils::DBBackend::Surreal => Arc::new(surreal_onchain),
            db_utils::DBBackend::Postgres => {
                let pool = pg_pool
                    .clone()
                    .expect("PostgreSQL pool when max_connections > 0");
                Arc::new(persistence::sqlx::DBOnChain::from_pool(pool))
            }
            db_utils::DBBackend::DefaultPostgres => {
                let pool = sqlx::PgPool::connect(&repository_new.connection)
                    .await
                    .expect("Failed to connect to PostgreSQL");
                Arc::new(persistence::sqlx::DBOnChain::from_pool(pool))
            }
        };
    let clowder_cl = onchain::ClowderCl {
        rest: clowder_client.clone(),
        nats: clowder_nats_client.clone(),
        min_confirmations,
    };
    let wdc = onchain::WildcatCl {
        core_cl: core_client.clone(),
    };
    let onchain = onchain::Service {
        melt_quote_expiry: time::Duration::seconds(melt_quote_expiry_seconds as i64),
        mint_quote_expiry: time::Duration::seconds(mint_quote_expiry_seconds as i64),
        wdc: Arc::new(wdc),
        repo: onchain_repo,
        clowder_cl: Arc::new(clowder_cl),
        min_mint_threshold,
        melt_fee_ppk,
        min_feerate_sat_per_vb,
        alpha_id: my_pk,
    };

    // eBill
    let config::Ebill {
        db: mintops,
        multiplier,
        ..
    } = ebill;
    let surreal_ebill = persistence::surreal::DBEbill::new(mintops)
        .await
        .expect("Failed to create ebill repository");
    let migrated = surreal_ebill
        .is_migrated()
        .await
        .expect("Failed to read ebill migration marker");
    let ebill_repo: Box<dyn ebill::Repository> =
        match db_utils::select_db_backend("treasury ebill", migrated, repository_new.clone()) {
            db_utils::DBBackend::Surreal => Box::new(surreal_ebill),
            db_utils::DBBackend::Postgres => {
                let pool = pg_pool
                    .clone()
                    .expect("PostgreSQL pool when max_connections > 0");
                Box::new(persistence::sqlx::DBEbill::from_pool(pool))
            }
            db_utils::DBBackend::DefaultPostgres => {
                let pool = sqlx::PgPool::connect(&repository_new.connection)
                    .await
                    .expect("Failed to connect to PostgreSQL");
                Box::new(persistence::sqlx::DBEbill::from_pool(pool))
            }
        };
    let wdccl = ebill::WildcatCl {
        core: core_client.clone(),
        ebill: Box::new(ebill_client),
    };
    let clwdcl = ebill::ClwdrCl {
        rest: clowder_client.clone(),
        nats: clowder_nats_client.clone(),
    };
    assert!(
        multiplier > cashu::Amount::ZERO,
        "Multiplier must be greater than zero"
    );
    let ebill = ebill::Service {
        repo: ebill_repo,
        wildcatcl: Box::new(wdccl),
        clowdercl: Box::new(clwdcl),
        multiplier,
    };

    // foreign
    let config::Foreign {
        online_repo,
        offline_repo,
        exchange_lock_margin_secs,
        offline_exchange_lock_secs,
        ..
    } = foreign;
    let surreal_online = persistence::surreal::DBForeignOnline::new(online_repo)
        .await
        .expect("Failed to create foreign online repository");
    let migrated = surreal_online
        .is_migrated()
        .await
        .expect("Failed to read foreign online migration marker");
    let onlinerepo: Arc<dyn foreign::OnlineRepository> = match db_utils::select_db_backend(
        "treasury foreign online",
        migrated,
        repository_new.clone(),
    ) {
        db_utils::DBBackend::Surreal => Arc::new(surreal_online),
        db_utils::DBBackend::Postgres => {
            let pool = pg_pool.clone().expect("PostgreSQL pool");
            Arc::new(persistence::sqlx::DBForeignOnline::from_pool(pool))
        }
        db_utils::DBBackend::DefaultPostgres => {
            let pool = sqlx::PgPool::connect(&repository_new.connection)
                .await
                .expect("Failed to connect to PostgreSQL");
            Arc::new(persistence::sqlx::DBForeignOnline::from_pool(pool))
        }
    };
    let foreign_offline_repo = persistence::surreal::DBForeignOffline::new(offline_repo)
        .await
        .expect("Failed to create foreign offline repository");
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
        exchange_lock_margin_secs,
        offline_exchange_lock_secs,
    };

    // vault
    let config::Vault { db } = vault;
    let surreal_vault = persistence::surreal::DBVault::new(db)
        .await
        .expect("Failed to create vault repository");
    let migrated = surreal_vault
        .is_migrated()
        .await
        .expect("Failed to read vault migration marker");
    let vault_repo: Box<dyn vault::Repository> =
        match db_utils::select_db_backend("treasury vault", migrated, repository_new.clone()) {
            db_utils::DBBackend::Surreal => Box::new(surreal_vault),
            db_utils::DBBackend::Postgres => {
                let pool = pg_pool.clone().expect("PostgreSQL pool");
                Box::new(persistence::sqlx::DBVault::from_pool(pool))
            }
            db_utils::DBBackend::DefaultPostgres => {
                let pool = sqlx::PgPool::connect(&repository_new.connection)
                    .await
                    .expect("Failed to connect to PostgreSQL");
                Box::new(persistence::sqlx::DBVault::from_pool(pool))
            }
        };
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
        repo: vault_repo,
        wdc_cl: Box::new(wdccl),
        my_url,
        mint_id: bcr_common::core::NodeId::new(my_pk, info.network),
    };

    // cache
    let cache_expiry = time::Duration::seconds(cache_expiry_sec as i64);
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
                online: onlinerepo.clone(),
                offline: offlinerepo,
                clowder: clowder.clone(),
                mint_factory: factory,
            },
            monitor_interval,
        ),
        routine::RoutineHandle::new(
            foreign::reclaim::Handler {
                online: onlinerepo,
                keys: foreigncore,
                clowder,
            },
            monitor_interval,
        ),
    ];
    (app_ctrl, monitors)
}

pub fn routes<Cntrlr>(app: Cntrlr) -> Router
where
    Cntrlr: Send + Sync + Clone + 'static,
    Arc<ebill::Service>: FromRef<Cntrlr>,
    Arc<onchain::Service>: FromRef<Cntrlr>,
    Arc<foreign::Service>: FromRef<Cntrlr>,
    Arc<vault::Service>: FromRef<Cntrlr>,
    Arc<ClowderNatsClient>: FromRef<Cntrlr>,
    Arc<dyn nut19::Cache>: FromRef<Cntrlr>,
{
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
            cl_treasury::web_ep::EXCHANGE_OFFLINE_REDEEM_V1,
            post(web::offline_redeem_exchange),
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
            cl_treasury::web_ep::MELT_ONCHAIN_ESTIMATE_V1,
            post(web::melt_onchain_estimate),
        )
        .route(
            cl_treasury::web_ep::MELT_ONCHAIN_CONFIG_V1,
            axum::routing::get(web::melt_onchain_config),
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
        )
        .route(
            cl_treasury::admin_ep::FOREIGN_BALANCE,
            get(admin::foreign_balance),
        );
    admin.merge(web).with_state(app)
}
