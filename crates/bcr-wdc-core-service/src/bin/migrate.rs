// ----- standard library imports
// ----- extra library imports
use bcr_wdc_core_service::persistence::{
    sqlx, surreal, KeysRepository as _, SignaturesRepository as _,
};
use bcr_wdc_utils::{postgres, surreal as surreal_config};
// ----- local imports

// ----- end imports

#[derive(Debug, serde::Deserialize)]
struct MigrateConfig {
    appcfg: MigrateAppConfig,
}

#[derive(Debug, serde::Deserialize)]
struct MigrateAppConfig {
    keys: surreal_config::DBConnConfig,
    keys_new: postgres::DBConnConfig,
    signatures: surreal_config::DBConnConfig,
    signatures_new: postgres::DBConnConfig,
    proofs: surreal_config::DBConnConfig,
    proofs_new: postgres::DBConnConfig,
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
    let surreal_keys = surreal::DBKeys::new(cfg.appcfg.keys)
        .await
        .expect("Failed to connect to keys SurrealDB");
    let surreal_signatures = surreal::DBSignatures::new(cfg.appcfg.signatures)
        .await
        .expect("Failed to connect to signatures SurrealDB");
    let surreal_proofs = surreal::DBProofs::new(cfg.appcfg.proofs)
        .await
        .expect("Failed to connect to proofs SurrealDB");
    // Dump all data from SurrealDB
    let keys = surreal_keys
        .dump()
        .await
        .expect("Failed to list keys from SurrealDB");
    let signatures = surreal_signatures
        .dump()
        .await
        .expect("Failed to list signatures from SurrealDB");
    let proofs = surreal_proofs
        .dump()
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
    let sqlx_keys = sqlx::DBKeys::new(cfg.appcfg.keys_new)
        .await
        .expect("Failed to connect to keys PostgreSQL");
    let sqlx_signatures = sqlx::DBSignatures::new(cfg.appcfg.signatures_new)
        .await
        .expect("Failed to connect to signatures PostgreSQL");
    let sqlx_proofs = sqlx::DBProofs::new(cfg.appcfg.proofs_new)
        .await
        .expect("Failed to connect to proofs PostgreSQL");
    // Migrate keys to PostgreSQL
    for keyset in keys {
        let kid = keyset.0.id;
        if let Err(error) = sqlx_keys.store(keyset).await {
            println!("Skipping keyset {kid}: failed with {error}");
        }
    }
    println!("Migration for keys complete");
    // Migrate signatures to PostgreSQL
    for (y, signature) in signatures {
        if let Err(error) = sqlx_signatures.store(y, signature).await {
            println!("Skipping signature {y}: failed with {error}");
        }
    }
    println!("Migration for signatures complete");
    // Migrate proofs to PostgreSQL
    sqlx_proofs
        .insert_v0(proofs)
        .await
        .expect("DBProofs::insert_v0 failed");
    println!("Migration for proofs complete");
}
