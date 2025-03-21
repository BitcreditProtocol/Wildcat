
use std::str::FromStr;

#[derive(Debug, serde::Deserialize)]
struct MainConfig {
    bind_address: std::net::SocketAddr,
    appcfg: bcr_wdc_treasury_service::AppConfig,
    log_level: log::LevelFilter,
}

#[tokio::main]
async fn main() {
    let settings = config::Config::builder()
        .add_source(config::File::with_name("config.toml"))
        .add_source(config::Environment::with_prefix("TREASURY"))
        .build()
        .expect("Failed to build treasury config");

    let maincfg: MainConfig = settings
        .try_deserialize()
        .expect("Failed to parse treasury config");

    env_logger::builder().filter_level(maincfg.log_level).init();

    let seed = [0u8; 32];
    let secret = bitcoin::secp256k1::SecretKey::from_str(
        "0a4d621638017a90cbb929e5bcbe8106a9c396d499a930a83e62814c52860bf2",
    )
    .expect("Failed to parse secret key from hex");
    let app = bcr_wdc_treasury_service::AppController::new(&seed, secret, maincfg.appcfg).await;
    let router = bcr_wdc_treasury_service::routes(app);

    let listener = tokio::net::TcpListener::bind(&maincfg.bind_address)
        .await
        .expect("Failed to bind to address");

    axum::serve(listener, router)
        .await
        .expect("Failed to start server");
}
