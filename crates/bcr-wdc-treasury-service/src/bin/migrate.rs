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
        .add_source(config::Environment::with_prefix("TREASURY_SERVICE").separator("__"))
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
    // Each table is marked as migrated in SurrealDB once copied: a re-run skips it,
    // and the SurrealDB repositories refuse any further write to it.
    let ebill_migrated = surreal_ebill
        .is_mint_ops_migrated()
        .await
        .expect("Failed to read ebill mint_ops migration marker from SurrealDB");
    let vault_migrated = surreal_vault
        .is_proofs_migrated()
        .await
        .expect("Failed to read vault proofs migration marker from SurrealDB");
    let mintops_migrated = surreal_onchain
        .is_mintops_migrated()
        .await
        .expect("Failed to read onchain mintops migration marker from SurrealDB");
    let meltops_migrated = surreal_onchain
        .is_meltops_migrated()
        .await
        .expect("Failed to read onchain meltops migration marker from SurrealDB");
    for (name, migrated) in [
        ("ebill mint_ops", ebill_migrated),
        ("vault proofs", vault_migrated),
        ("onchain mintops", mintops_migrated),
        ("onchain meltops", meltops_migrated),
    ] {
        if migrated {
            println!("{name} already migrated, skipping");
        }
    }
    if dry_run {
        println!("DRY RUN: Would migrate");
        if !ebill_migrated {
            let ebill_ops = surreal_ebill
                .dump()
                .await
                .expect("Failed to list ebill mint_ops from SurrealDB");
            println!("   {} ebill mint_ops to PostgreSQL", ebill_ops.len());
        }
        if !vault_migrated {
            let pfs = surreal_vault
                .dump()
                .await
                .expect("Failed to list vault proofs from SurrealDB");
            println!("   {} vault proofs to PostgreSQL", pfs.len());
        }
        if !mintops_migrated {
            let onchain_mintops = surreal_onchain
                .dump_mintops()
                .await
                .expect("Failed to list onchain mintops from SurrealDB");
            println!("   {} onchain mintops to PostgreSQL", onchain_mintops.len());
        }
        if !meltops_migrated {
            let onchain_meltops = surreal_onchain
                .dump_meltops()
                .await
                .expect("Failed to list onchain meltops from SurrealDB");
            println!("   {} onchain meltops to PostgreSQL", onchain_meltops.len());
        }
        return;
    }
    // Connect to PostgreSQL (destination)
    bcr_wdc_utils::db::postgres::run_migration(&cfg.appcfg.repository_new).await;
    let sqlx_ebill = sqlx::DBEbill::new(cfg.appcfg.repository_new.clone())
        .await
        .expect("Failed to connect to PostgreSQL");
    let sqlx_vault = sqlx::DBVault::new(cfg.appcfg.repository_new.clone())
        .await
        .expect("Failed to connect to PostgreSQL");
    let sqlx_onchain = sqlx::DBOnChain::new(cfg.appcfg.repository_new)
        .await
        .expect("Failed to connect to PostgreSQL");
    if !ebill_migrated {
        let ebill_ops = surreal_ebill
            .dump()
            .await
            .expect("Failed to list ebill mint_ops from SurrealDB");
        println!("Found {} ebill mint_ops in SurrealDB", ebill_ops.len());
        for op in ebill_ops {
            let uid = op.uid;
            if let Err(e) = sqlx_ebill.mint_store(op).await {
                eprintln!("Failed to migrate ebill mint_op {uid}: {e}");
                std::process::exit(1);
            }
        }
        surreal_ebill
            .mark_mint_ops_migrated()
            .await
            .expect("Failed to mark ebill mint_ops as migrated in SurrealDB");
        println!("Migration for ebill complete");
    }
    if !vault_migrated {
        let pfs = surreal_vault
            .dump()
            .await
            .expect("Failed to list vault proofs from SurrealDB");
        println!("Found {} vault proofs in SurrealDB", pfs.len());
        if let Err(e) = sqlx_vault.store_proofs(pfs).await {
            eprintln!("Failed to migrate vault proofs: {e}");
            std::process::exit(1);
        }
        surreal_vault
            .mark_proofs_migrated()
            .await
            .expect("Failed to mark vault proofs as migrated in SurrealDB");
        println!("Migration for vault complete");
    }
    if !mintops_migrated {
        let onchain_mintops = surreal_onchain
            .dump_mintops()
            .await
            .expect("Failed to list onchain mintops from SurrealDB");
        println!(
            "Found {} onchain mintops in SurrealDB",
            onchain_mintops.len()
        );
        for op in onchain_mintops {
            let qid = op.qid;
            if let Err(e) = sqlx_onchain.store_mintop(op).await {
                eprintln!("Failed to migrate onchain mintop {qid}: {e}");
                std::process::exit(1);
            }
        }
        surreal_onchain
            .mark_mintops_migrated()
            .await
            .expect("Failed to mark onchain mintops as migrated in SurrealDB");
        println!("Migration for onchain mintops complete");
    }
    if !meltops_migrated {
        let onchain_meltops = surreal_onchain
            .dump_meltops()
            .await
            .expect("Failed to list onchain meltops from SurrealDB");
        println!(
            "Found {} onchain meltops in SurrealDB",
            onchain_meltops.len()
        );
        let now = time::OffsetDateTime::now_utc();
        for op in onchain_meltops {
            let qid = op.qid;
            if let Err(e) = sqlx_onchain.store_meltop(op, now).await {
                eprintln!("Failed to migrate onchain meltop {qid}: {e}");
                std::process::exit(1);
            }
        }
        surreal_onchain
            .mark_meltops_migrated()
            .await
            .expect("Failed to mark onchain meltops as migrated in SurrealDB");
        println!("Migration for onchain meltops complete");
    }
}
