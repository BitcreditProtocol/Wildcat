use tokio::signal::{
    ctrl_c,
    unix::{signal, SignalKind},
};

#[derive(Debug, serde::Deserialize)]
struct MainConfig {
    bind_address: std::net::SocketAddr,
    log_level: log::LevelFilter,
    appcfg: bcr_wdc_ebpp::AppConfig,
}

#[tokio::main]
async fn main() {
    let settings = config::Config::builder()
        .add_source(config::File::with_name("config.toml"))
        .add_source(config::Environment::with_prefix("EBPP"))
        .build()
        .expect("Failed to build wildcat config");

    let maincfg: MainConfig = settings
        .try_deserialize()
        .expect("Failed to parse wildcat config");

    env_logger::builder().filter_level(maincfg.log_level).init();

    let controller = bcr_wdc_ebpp::AppController::new(maincfg.appcfg).await;
    let mut grpc_server = controller
        .new_grpc_server()
        .await
        .expect("AppController::new_grpc_server");
    grpc_server
        .start(None)
        .await
        .expect("PaymentProcessorServer::start");

    let listener = tokio::net::TcpListener::bind(&maincfg.bind_address)
        .await
        .expect("Failed to bind to address");
    let router = bcr_wdc_ebpp::routes(controller);
    axum::serve(listener, router)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .expect("Failed to start server");

    grpc_server
        .stop()
        .await
        .expect("PaymentProcessorServer::stop");
}

async fn shutdown_signal() {
    let mut terminate = signal(SignalKind::terminate()).expect("failed to install signal handler");
    tokio::select! {
        _ = ctrl_c() => {},
        _ = terminate.recv() => {},
    }
    log::info!("Shutting down...");
}
