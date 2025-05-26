use bitcoin::hashes::Hash;
use std::str::FromStr;
use tokio::signal;
use tracing_subscriber::{filter::LevelFilter, prelude::*};

#[derive(Debug, serde::Deserialize)]
struct MainConfig {
    bind_address: std::net::SocketAddr,
    appcfg: bcr_wdc_treasury_service::AppConfig,
    log_level: String,
}

#[derive(Debug, serde::Deserialize)]
struct SeedConfig {
    mnemonic: bip39::Mnemonic,
}

#[tokio::main]
async fn main() {
    let settings = config::Config::builder()
        .add_source(config::File::with_name("config.toml"))
        .add_source(config::Environment::with_prefix("TREASURY_SERVICE"))
        .build()
        .expect("Failed to build treasury config");

    let maincfg: MainConfig = settings
        .try_deserialize()
        .expect("Failed to parse treasury config");

    tracing_log::LogTracer::init().expect("LogTracer init");
    let level_filter = LevelFilter::from_str(&maincfg.log_level).expect("log level");
    let stdout_log = tracing_subscriber::fmt::layer().with_filter(level_filter);
    let subscriber = tracing_subscriber::registry().with(stdout_log);
    tracing::subscriber::set_global_default(subscriber)
        .expect("tracing::subscriber::set_global_default");

    // seed is acquired from environment variables
    let settings = config::Config::builder()
        .add_source(config::Environment::with_prefix("TREASURY_SERVICE"))
        .build()
        .expect("Failed to build seed config");
    let seedcfg: SeedConfig = settings
        .try_deserialize()
        .expect("Failed to parse seed config");
    let seed = seedcfg.mnemonic.to_seed("treasury-service");
    let signing_seed = seedcfg.mnemonic.to_seed("treasury-service-signing");
    let signing_slice = bitcoin::hashes::sha256::Hash::hash(&signing_seed);
    let secret = bitcoin::secp256k1::SecretKey::from_slice(signing_slice.as_byte_array())
        .expect("Failed to create secret key from seed");

    let app = bcr_wdc_treasury_service::AppController::new(&seed, secret, maincfg.appcfg).await;
    let router = bcr_wdc_treasury_service::routes(app);

    let listener = tokio::net::TcpListener::bind(&maincfg.bind_address)
        .await
        .expect("Failed to bind to address");

    axum::serve(listener, router)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .expect("Failed to start server");
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
