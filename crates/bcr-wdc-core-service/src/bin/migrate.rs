// ----- standard library imports
// ----- extra library imports
use bcr_wdc_core_service::persistence::{sqlx, surreal, Repository};
use bcr_wdc_utils::{postgres, surreal as surreal_config};
// ----- local imports

// ----- end imports

#[derive(Debug, serde::Deserialize)]
struct MigrateConfig {
    appcfg: MigrateAppConfig,
}

#[derive(Debug, serde::Deserialize)]
struct MigrateAppConfig {
    repository: surreal_config::DBConnConfig,
    repository_new: postgres::DBConnConfig,
}

#[tokio::main]
async fn main() {
    let dry_run = std::env::args().any(|arg| arg == "--dry-run");
    let settings = config::Config::builder()
        .add_source(config::File::with_name("config.toml"))
        .add_source(config::Environment::with_prefix("CORE_SERVICE").separator("__"))
        .build()
        .expect("Failed to build migrate config");
    let cfg: MigrateConfig = settings
        .try_deserialize()
        .expect("Failed to parse migrate config");
    // Connect to SurrealDB
    let surreal_repository = surreal::Repository::new(cfg.appcfg.repository)
        .await
        .expect("Failed to connect to SurrealDB");
    // Dump all data from SurrealDB
    let keys = surreal_repository
        .dump_keys()
        .await
        .expect("Failed to list keys from SurrealDB");
    let signatures = surreal_repository
        .dump_signatures()
        .await
        .expect("Failed to list signatures from SurrealDB");
    let proofs = surreal_repository
        .dump_proofs()
        .await
        .expect("Failed to list proofs from SurrealDB");
    println!("Found {} keysets in SurrealDB", keys.len());
    println!("Found {} signatures in SurrealDB", signatures.len());
    println!("Found {} proofs in SurrealDB", proofs.len());
    if dry_run {
        println!("DRY RUN: Would migrate");
        println!("   {} keysets to PostgreSQL", keys.len());
        println!("   {} signatures to PostgreSQL", signatures.len());
        println!("   {} proofs to PostgreSQL", proofs.len());
        return;
    }
    // Connect to PostgreSQL
    bcr_wdc_utils::db::postgres::run_migration(&cfg.appcfg.repository_new).await;
    let sqlx_repository = sqlx::Repository::new(cfg.appcfg.repository_new)
        .await
        .expect("Failed to connect to PostgreSQL");
    // Migrate keys to PostgreSQL
    for keyset in keys {
        let kid = keyset.0.id;
        if let Err(error) = sqlx_repository.keys_store(keyset).await {
            println!("Skipping keyset {kid}: failed with {error}");
        }
    }
    println!("Migration for keys complete");
    // Migrate signatures to PostgreSQL
    for (y, signature) in signatures {
        if let Err(error) = sqlx_repository.signature_store(y, signature).await {
            println!("Skipping signature {y}: failed with {error}");
        }
    }
    println!("Migration for signatures complete");
    // Migrate proofs to PostgreSQL
    sqlx::insert_v0(&sqlx_repository, proofs)
        .await
        .expect("SqlxRepository::insert_v0 failed");
    println!("Migration for proofs complete");
}
