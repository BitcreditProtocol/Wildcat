use std::str::FromStr;
use tokio::signal;
use tracing_subscriber::{filter::LevelFilter, prelude::*};

#[derive(Debug, serde::Deserialize)]
struct MainConfig {
    appcfg: bcr_wdc_demo_faucet::AppConfig,
    log_level: String,
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let settings = config::Config::builder()
        .add_source(config::File::with_name("config.toml"))
        .add_source(config::Environment::with_prefix("DEMO_FAUCET").separator("__"))
        .build()
        .expect("Failed to build demo faucet config");

    let maincfg: MainConfig = settings
        .try_deserialize()
        .expect("Failed to parse demo faucet config");

    tracing_log::LogTracer::init().expect("LogTracer init");
    let level_filter = LevelFilter::from_str(&maincfg.log_level).expect("log level");
    let stdout_log = tracing_subscriber::fmt::layer().with_filter(level_filter);
    let subscriber = tracing_subscriber::registry().with(stdout_log);
    tracing::subscriber::set_global_default(subscriber)
        .expect("tracing::subscriber::set_global_default");

    tracing::info!("Starting demo faucet with config: {:#?}", maincfg);

    loop {
        let cancellation = tokio_util::sync::CancellationToken::new();
        let main_loop =
            bcr_wdc_demo_faucet::main_loop(maincfg.appcfg.clone(), cancellation.clone());
        tokio::select! {
            _ = shutdown_signal() => {
                tracing::info!("Received shutdown signal, exiting main loop.");
                cancellation.cancel();
                break;
            },
            result = main_loop => {
                tracing::debug!("main loop exited");
                match result {
                    Ok(_) => {
                        tracing::info!("Main loop completed successfully.");
                        break;
                    },
                    Err(e) => {
                        tracing::error!("Main loop encountered an error: {}", e);
                    }
                }
            }
        }
    }
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
