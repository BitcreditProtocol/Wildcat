// ----- standard library imports
// ----- extra library imports
use bcr_wdc_treasury_service::{
    ebill::Repository as _,
    onchain::Repository as _,
    persistence::{sqlx, surreal},
    vault::Repository as _,
};

// ----- local imports

// ----- end imports:

#[derive(Debug, serde::Deserialize)]
struct MigrateConfig {
    appcfg: bcr_wdc_treasury_service::config::App,
}

#[tokio::main]
async fn main() {
    let dry_run = std::env::args().any(|a| a == "--dry-run");
    let settings = config::Config::builder()
        .add_source(config::File::with_name("config.toml"))
        .add_source(config::Environment::with_prefix("TREASURY_SERVICE"))
        .build()
        .expect("Failed to build config");
    let cfg: MigrateConfig = settings
        .try_deserialize()
        .expect("Failed to parse migrate config");
    // Connect to SurrealDB (source)
    let surreal_ebill = surreal::DBEbill::new(cfg.appcfg.ebill.db)
        .await
        .expect("Failed to connect to ebill SurrealDB");
    let surreal_vault = surreal::DBVault::new(cfg.appcfg.vault.db)
        .await
        .expect("Failed to connect to vault SurrealDB");
    let surreal_onchain = surreal::DBOnChain::new(cfg.appcfg.onchain.db)
        .await
        .expect("Failed to connect to onchain SurrealDB");
    // Read all
    let ebill_ops = surreal_ebill
        .dump()
        .await
        .expect("Failed to list ebill mint_ops from SurrealDB");
    let pfs = surreal_vault
        .dump()
        .await
        .expect("Failed to list vault proofs from SurrealDB");
    let onchain_mintops = surreal_onchain
        .dump_mintops()
        .await
        .expect("Failed to list onchain mintops from SurrealDB");
    let onchain_meltops = surreal_onchain
        .dump_meltops()
        .await
        .expect("Failed to list onchain meltops from SurrealDB");
    println!("Found {} ebill mint_ops in SurrealDB", ebill_ops.len());
    println!("Found {} vault proofs in SurrealDB", pfs.len());
    println!(
        "Found {} onchain mintops in SurrealDB",
        onchain_mintops.len()
    );
    println!(
        "Found {} onchain meltops in SurrealDB",
        onchain_meltops.len()
    );
    if dry_run {
        println!("DRY RUN: Would migrate");
        println!("   {} ebill mint_ops to PostgreSQL", ebill_ops.len());
        println!("   {} vault proofs to PostgreSQL", pfs.len());
        println!("   {} onchain mintops to PostgreSQL", onchain_mintops.len());
        println!("   {} onchain meltops to PostgreSQL", onchain_meltops.len());
        return;
    }
    // Connect to PostgreSQL (destination)
    let sqlx_ebill = sqlx::DBEbill::new(cfg.appcfg.ebill.new)
        .await
        .expect("Failed to connect to PostgreSQL");
    let sqlx_vault = sqlx::DBVault::new(cfg.appcfg.vault.new)
        .await
        .expect("Failed to connect to PostgreSQL");
    let sqlx_onchain = sqlx::DBOnChain::new(cfg.appcfg.onchain.new)
        .await
        .expect("Failed to connect to PostgreSQL");
    for op in ebill_ops {
        let uid = op.uid;
        if let Err(e) = sqlx_ebill.mint_store(op).await {
            println!("Skipping mint_op {uid}: failed with {e}");
        }
    }
    println!("Migration for ebill complete");
    sqlx_vault
        .store_proofs(pfs)
        .await
        .expect("VaultDB::store_proofs failed");
    println!("Migration for vault complete");

    for op in onchain_mintops {
        sqlx_onchain
            .store_mintop(op)
            .await
            .expect("Failed to store onchain mintop");
    }
    println!("Migration for onchain mintops complete");
    let now = chrono::Utc::now();
    for op in onchain_meltops {
        sqlx_onchain
            .store_meltop(op, now)
            .await
            .expect("Failed to store onchain meltop");
    }
    println!("Migration for onchain meltops complete");
}
