#[derive(Debug, serde::Deserialize)]
struct MainConfig {
    bind_address: std::net::SocketAddr,
    appcfg: bcr_wdc_quote_service::AppConfig,
    log_level: log::LevelFilter,
}

#[tokio::main]
async fn main() {
    let settings = config::Config::builder()
        .add_source(config::File::with_name("wildcat.toml"))
        .add_source(config::Environment::with_prefix("WILDCAT"))
        .build()
        .expect("Failed to build wildcat config");

    let maincfg: MainConfig = settings
        .try_deserialize()
        .expect("Failed to parse wildcat config");

    env_logger::builder().filter_level(maincfg.log_level).init();

    // we keep seed separate from the app config
    let seed = [0u8; 32];
    let app = bcr_wdc_quote_service::AppController::new(&seed, maincfg.appcfg).await;
    let router = bcr_wdc_quote_service::credit_routes(app);

    let listener = tokio::net::TcpListener::bind(&maincfg.bind_address)
        .await
        .expect("Failed to bind to address");

    axum::serve(listener, router)
        .await
        .expect("Failed to start server");
}
