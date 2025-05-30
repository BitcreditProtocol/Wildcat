// ----- standard library imports
use bcr_ebill_api::{
    service::notification_service::{create_nostr_clients, create_nostr_consumer},
    MintConfig, NostrConfig,
};
use std::str::FromStr;
// ----- extra library imports
use tokio::signal;
use tracing::info;
use tracing_subscriber::{filter::LevelFilter, prelude::*};
// ----- local modules
mod job;
// ----- end imports

#[derive(Debug, serde::Deserialize)]
struct MainConfig {
    bind_address: std::net::SocketAddr,
    appcfg: bcr_wdc_ebill_service::AppConfig,
    log_level: String,
}

#[tokio::main]
async fn main() {
    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("failed to install default provider for rustls ring");
    // parse and create config
    let settings = config::Config::builder()
        .add_source(config::File::with_name("config.toml"))
        .add_source(config::Environment::with_prefix("EBILL"))
        .build()
        .expect("Failed to build ebill config");

    let maincfg: MainConfig = settings
        .try_deserialize()
        .expect("Failed to parse ebill config");

    tracing_log::LogTracer::init().expect("LogTracer init");
    let level_filter = LevelFilter::from_str(&maincfg.log_level).expect("log level");
    let stdout_log = tracing_subscriber::fmt::layer().with_filter(level_filter);
    let subscriber = tracing_subscriber::registry().with(stdout_log);
    tracing::subscriber::set_global_default(subscriber)
        .expect("tracing::subscriber::set_global_default");

    // create bcr_ebill_api config
    let api_config = bcr_ebill_api::Config {
        bitcoin_network: maincfg.appcfg.bitcoin_network.clone(),
        esplora_base_url: maincfg.appcfg.esplora_base_url.clone(),
        nostr_config: NostrConfig {
            relays: maincfg.appcfg.nostr_cfg.relays.clone(),
            only_known_contacts: maincfg.appcfg.nostr_cfg.only_known_contacts,
        },
        mint_config: MintConfig {
            default_mint_url: maincfg.appcfg.mint_config.default_mint_url.clone(),
            default_mint_node_id: maincfg.appcfg.mint_config.default_mint_node_id.clone(),
        },
        db_config: bcr_ebill_api::SurrealDbConfig {
            connection_string: maincfg.appcfg.ebill_db.connection.clone(),
            namespace: maincfg.appcfg.ebill_db.namespace.clone(),
            database: maincfg.appcfg.ebill_db.database.clone(),
        },
        data_dir: maincfg.appcfg.data_dir.clone(),
    };
    bcr_ebill_api::init(api_config.clone()).expect("Could not initialize E-Bill API");

    // initialize DB context
    let db = bcr_ebill_api::get_db_context(&api_config)
        .await
        .expect("Failed to create DB context");

    // initialize identity keys
    let keys = db
        .identity_store
        .get_or_create_key_pair()
        .await
        .expect("Failed to get, or create local identity keys");
    let local_node_id = keys.get_public_key();
    info!("Local node id: {local_node_id:?}");
    info!("Local npub: {:?}", keys.get_nostr_npub());
    info!("Local npriv: {:?}", keys.get_nostr_npriv());
    info!("Local npub as hex: {:?}", keys.get_nostr_npub_as_hex());

    // set up nostr clients for existing identities
    let nostr_clients = create_nostr_clients(
        &api_config,
        db.identity_store.clone(),
        db.company_store.clone(),
    )
    .await
    .expect("Failed to create nostr clients");
    let db_clone = db.clone();
    // set up application context
    let app =
        bcr_wdc_ebill_service::AppController::new(api_config, nostr_clients.clone(), db).await;
    let router = bcr_wdc_ebill_service::routes(app.clone());

    // run jobs in background
    let app_clone = app.clone();
    tokio::spawn(async move {
        job::run(
            app_clone.clone(),
            maincfg.appcfg.job_runner_initial_delay_seconds,
            maincfg.appcfg.job_runner_check_interval_seconds,
        )
        .await
    });

    // set up nostr event consumer
    let nostr_consumer = create_nostr_consumer(
        nostr_clients,
        app.contact_service.clone(),
        db_clone.nostr_event_offset_store.clone(),
        db_clone.notification_store.clone(),
        app.push_service.clone(),
        db_clone.bill_blockchain_store.clone(),
        db_clone.bill_store.clone(),
        db_clone.nostr_contact_store.clone(),
    )
    .await
    .expect("Failed to create Nostr consumer");
    // run nostr consumer in background
    let nostr_handle = tokio::spawn(async move {
        nostr_consumer.start().await.expect("nostr consumer failed");
    });

    // run web server
    let listener = tokio::net::TcpListener::bind(&maincfg.bind_address)
        .await
        .expect("Failed to bind to address");

    info!(
        "E-Bill Service running at http://{} with config: {:?}",
        &maincfg.bind_address, &maincfg
    );
    axum::serve(listener, router)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .expect("Failed to start server");
    nostr_handle.abort();
}

async fn shutdown_signal() {
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}
