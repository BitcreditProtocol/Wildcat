use chrono::DurationRound;
use std::str::FromStr;
use tokio::signal;
use tracing_subscriber::{filter::LevelFilter, prelude::*};

#[derive(Debug, serde::Deserialize)]
struct MainConfig {
    log_level: String,
    appcfg: bcr_wdc_balance_collector::AppConfig,
    interval_minutes: u64,
}

#[tokio::main]
async fn main() {
    let settings = config::Config::builder()
        .add_source(config::File::with_name("config.toml"))
        .add_source(config::Environment::with_prefix("BALANCE_COLLECTOR"))
        .build()
        .expect("Failed to build wildcat config");

    let maincfg: MainConfig = settings
        .try_deserialize()
        .expect("Failed to parse wildcat config");

    tracing_log::LogTracer::init().expect("LogTracer init");
    let level_filter = LevelFilter::from_str(&maincfg.log_level).expect("log level");
    let stdout_log = tracing_subscriber::fmt::layer().with_filter(level_filter);
    let subscriber = tracing_subscriber::registry().with(stdout_log);
    tracing::subscriber::set_global_default(subscriber)
        .expect("tracing::subscriber::set_global_default");

    let controller = bcr_wdc_balance_collector::AppController::new(maincfg.appcfg)
        .await
        .expect("AppController init");

    let rounder = chrono::Duration::minutes(maincfg.interval_minutes as i64);
    loop {
        let next_interval = next_interval(rounder);
        tracing::info!(
            "Waiting for next balance collection interval happening in {} seconds",
            next_interval.as_secs()
        );
        tokio::select! {
        _ = shutdown_signal() => {
            tracing::info!("Shutdown signal received, exiting...");
        },
        _ = tokio::time::sleep(next_interval) => {
            tracing::info!("Starting balance collection...");
                controller.collect_balances(chrono::Utc::now()).await.expect("Balance collection failed");
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

// Calculate the next interval based on the current time and the specified rounder
// in case of error, it defaults to 1 hour
fn next_interval(rounder: chrono::Duration) -> std::time::Duration {
    let now = chrono::Utc::now();
    let trunced = now.duration_trunc(rounder).unwrap();
    let expected = trunced + rounder;
    let elapse = expected - now;
    elapse
        .to_std()
        .unwrap_or(std::time::Duration::from_secs(3600))
}
