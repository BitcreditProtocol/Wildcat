use std::str::FromStr;
use tokio::signal;
use tracing_subscriber::{filter::LevelFilter, prelude::*};

#[derive(Debug, serde::Deserialize)]
struct MainConfig {
    bind_address: std::net::SocketAddr,
    #[serde(default)]
    admin_bind_address: Option<std::net::SocketAddr>,
    appcfg: bcr_wdc_quote_service::AppConfig,
    log_level: String,
}

/// No default: a container that silently fell back to loopback would break every
/// internal caller without closing any real exposure, so a missing setting must
/// fail startup instead.
fn require_admin_bind_address(
    admin_bind_address: Option<std::net::SocketAddr>,
) -> std::net::SocketAddr {
    admin_bind_address.unwrap_or_else(|| {
        panic!(
            "missing required setting `admin_bind_address` (env QUOTE_SERVICE__ADMIN_BIND_ADDRESS): \
             the admin listener has no default and must be set explicitly"
        )
    })
}

#[tokio::main]
async fn main() {
    let settings = config::Config::builder()
        .add_source(config::File::with_name("config.toml"))
        .add_source(config::Environment::with_prefix("QUOTE_SERVICE").separator("__"))
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

    let admin_bind_address = require_admin_bind_address(maincfg.admin_bind_address);

    let (app, routine_hndl) = bcr_wdc_quote_service::init_app(maincfg.appcfg).await;
    let web_router = bcr_wdc_quote_service::web_routes().with_state(app.clone());
    let admin_router = bcr_wdc_quote_service::admin_routes().with_state(app);

    bcr_wdc_utils::serve::serve_split(
        web_router,
        admin_router,
        maincfg.bind_address,
        admin_bind_address,
        shutdown_signal(),
    )
    .await
    .expect("Failed to start server");
    routine_hndl.stop().await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn require_admin_bind_address_accepts_a_configured_value() {
        let addr: std::net::SocketAddr = "127.0.0.1:3339".parse().unwrap();
        assert_eq!(require_admin_bind_address(Some(addr)), addr);
    }

    #[test]
    #[should_panic(expected = "admin_bind_address")]
    fn require_admin_bind_address_refuses_to_start_when_unset() {
        require_admin_bind_address(None);
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
