// ----- standard library imports
// ----- extra library imports
use bcr_wdc_mint_service::persistence;
// ----- local imports

// ----- end imports

#[derive(Debug, serde::Deserialize)]
struct MigrateConfig {
    appcfg: bcr_wdc_mint_service::config::App,
}

#[tokio::main]
async fn main() {
    let settings = config::Config::builder()
        .add_source(config::File::with_name("config.toml"))
        .add_source(config::Environment::with_prefix("MINT_SERVICE").separator("__"))
        .build()
        .expect("Failed to build config");
    let cfg: MigrateConfig = settings
        .try_deserialize()
        .expect("Failed to parse migrate config");
    let pool = bcr_wdc_utils::db::postgres::connect(&cfg.appcfg.repository)
        .await
        .expect("Failed to connect to PostgreSQL");
    let repository = persistence::sqlx::Repository::from_pool(pool);
    match repository.migrate_legacy_swaps().await {
        Ok(migrated) => println!("Migrated {migrated} swaps to versioned blobs"),
        Err(e) => {
            eprintln!("Failed to migrate swaps: {e}");
            std::process::exit(1);
        }
    }
}
