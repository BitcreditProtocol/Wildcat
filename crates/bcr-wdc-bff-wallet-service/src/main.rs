use tokio::signal::{
    ctrl_c,
    unix::{signal, SignalKind},
};

#[derive(Debug, serde::Deserialize)]
struct MainConfig {
    bind_address: std::net::SocketAddr,
    appcfg: bcr_wdc_bff_wallet_service::AppConfig,
    log_level: log::LevelFilter,
}

#[tokio::main]
async fn main() {
    let settings = config::Config::builder()
        .add_source(config::File::with_name("config.toml"))
        .add_source(config::Environment::with_prefix("BFF_WALLET"))
        .build()
        .expect("Failed to build wildcat config");

    let maincfg: MainConfig = settings
        .try_deserialize()
        .expect("Failed to parse wildcat config");

    env_logger::builder().filter_level(maincfg.log_level).init();

    let app = bcr_wdc_bff_wallet_service::AppController::new(maincfg.appcfg);
    let router = bcr_wdc_bff_wallet_service::routes(app);

    let listener = tokio::net::TcpListener::bind(&maincfg.bind_address)
        .await
        .expect("Failed to bind to address");

    axum::serve(listener, router)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .expect("Failed to start server");
}

async fn shutdown_signal() {
    let mut terminate = signal(SignalKind::terminate()).expect("failed to install signal handler");
    tokio::select! {
        _ = ctrl_c() => {},
        _ = terminate.recv() => {},
    }
    log::info!("Shutting down...");
}
