// ----- standard library imports
use std::collections::HashMap;
// ----- extra library imports
use bcr_wdc_treasury_service::{
    ebill::Repository as _, foreign::OnlineRepository as _, onchain::Repository as _, persistence,
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
    let surreal_ebill = persistence::surreal::DBEbill::new(cfg.appcfg.ebill.db)
        .await
        .expect("Failed to connect to ebill SurrealDB");
    let surreal_vault = persistence::surreal::DBVault::new(cfg.appcfg.vault.db)
        .await
        .expect("Failed to connect to vault SurrealDB");
    let surreal_onchain = persistence::surreal::DBOnChain::new(cfg.appcfg.onchain.db)
        .await
        .expect("Failed to connect to onchain SurrealDB");
    let surreal_foreign =
        persistence::surreal::DBForeignOnline::new(cfg.appcfg.foreign.online_repo)
            .await
            .expect("Failed to connect to foreign online SurrealDB");
    if dry_run {
        println!("DRY RUN: Would migrate");
        let ebill_ops = surreal_ebill
            .dump()
            .await
            .expect("Failed to list ebill mint_ops from SurrealDB");
        println!("   {} ebill mint_ops in SurrealDB", ebill_ops.len());
        let pfs = surreal_vault
            .dump()
            .await
            .expect("Failed to list vault proofs from SurrealDB");
        println!("   {} vault proofs in SurrealDB", pfs.len());
        let onchain_mintops = surreal_onchain
            .dump_mintops()
            .await
            .expect("Failed to list onchain mintops from SurrealDB");
        println!("   {} onchain mintops in SurrealDB", onchain_mintops.len());
        let onchain_meltops = surreal_onchain
            .dump_meltops()
            .await
            .expect("Failed to list onchain meltops from SurrealDB");
        println!("   {} onchain meltops in SurrealDB", onchain_meltops.len());
        let denied_meltops = surreal_onchain
            .dump_denied_meltops()
            .await
            .expect("Failed to list onchain denied meltops from SurrealDB");
        println!(
            "   {} onchain denied meltops in SurrealDB",
            denied_meltops.len()
        );
        let proofs = surreal_foreign
            .dump_proofs()
            .await
            .expect("Failed to list foreign proofs from SurrealDB");
        println!("   {} foreign proofs in SurrealDB", proofs.len());
        let htlcs = surreal_foreign
            .dump_htlcs()
            .await
            .expect("Failed to list foreign htlcs from SurrealDB");
        println!("   {} foreign htlcs in SurrealDB", htlcs.len());
        let issued = surreal_foreign
            .dump_issued()
            .await
            .expect("Failed to list foreign issued proofs from SurrealDB");
        println!("   {} foreign issued proofs in SurrealDB", issued.len());
        return;
    }
    // Connect to PostgreSQL (destination)
    bcr_wdc_utils::db::postgres::run_migration(&cfg.appcfg.repository_new).await;
    let pool = sqlx::postgres::PgPool::connect(&cfg.appcfg.repository_new.connection)
        .await
        .expect("Failed to connect to PostgreSQL");
    //ebill
    let migrated = surreal_ebill
        .is_migrated()
        .await
        .expect("Failed to read ebill mint_ops migration marker from SurrealDB");
    if migrated {
        println!("ebill mint_ops already migrated, skipping");
    } else {
        let sqlx_ebill = persistence::sqlx::DBEbill::from_pool(pool.clone());
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
            .mark_migrated()
            .await
            .expect("Failed to mark ebill mint_ops as migrated in SurrealDB");
    }
    // vault
    let migrated = surreal_vault
        .is_migrated()
        .await
        .expect("Failed to read vault proofs migration marker from SurrealDB");
    if migrated {
        println!("vault proofs already migrated, skipping");
    } else {
        let sqlx_vault = persistence::sqlx::DBVault::from_pool(pool.clone());
        let pfs = surreal_vault
            .dump()
            .await
            .expect("Failed to list vault proofs from SurrealDB");
        println!("Found {} vault proofs in SurrealDB", pfs.len());
        sqlx_vault
            .store_proofs(pfs)
            .await
            .expect("Failed to migrate vault proofs");
        surreal_vault
            .mark_migrated()
            .await
            .expect("Failed to mark vault proofs as migrated in SurrealDB");
    }
    // onchain
    let migrated = surreal_onchain
        .is_migrated()
        .await
        .expect("Failed to read onchain migration marker from SurrealDB");
    if migrated {
        println!("onchain already migrated, skipping");
    } else {
        let sqlx_onchain = persistence::sqlx::DBOnChain::from_pool(pool.clone());
        // onchain - mintops
        let onchain_mintops = surreal_onchain
            .dump_mintops()
            .await
            .expect("Failed to list onchain mintops from SurrealDB");
        println!(
            "Found {} onchain mintops in SurrealDB",
            onchain_mintops.len(),
        );
        for op in onchain_mintops {
            sqlx_onchain
                .store_mintop(op)
                .await
                .expect("Failed to migrate onchain mintop");
        }
        // onchain - meltops
        let onchain_meltops = surreal_onchain
            .dump_meltops()
            .await
            .expect("Failed to list onchain meltops from SurrealDB");
        println!(
            "Found {} onchain meltops in SurrealDB",
            onchain_meltops.len(),
        );
        let now = time::OffsetDateTime::now_utc();
        for op in onchain_meltops {
            sqlx_onchain
                .store_meltop(op, now)
                .await
                .expect("Failed to migrate onchain meltop");
        }
        // onchain - denied meltops
        let denied_meltops = surreal_onchain
            .dump_denied_meltops()
            .await
            .expect("Failed to list onchain denied meltops from SurrealDB");
        println!(
            "Found {} onchain denied meltops in SurrealDB",
            denied_meltops.len(),
        );
        for op in denied_meltops {
            sqlx_onchain
                .store_denied_meltop(op)
                .await
                .expect("Failed to migrate onchain denied meltop");
        }
        surreal_onchain
            .mark_migrated()
            .await
            .expect("Failed to mark onchain as migrated in SurrealDB");
    }
    // foreign online
    let migrated = surreal_foreign
        .is_migrated()
        .await
        .expect("Failed to read foreign online migration marker from SurrealDB");
    if migrated {
        println!("foreign online already migrated, skipping");
    } else {
        let sqlx_foreign = persistence::sqlx::DBForeignOnline::from_pool(pool.clone());
        // foreign - proofs
        let proofs = surreal_foreign
            .dump_proofs()
            .await
            .expect("Failed to list foreign proofs from SurrealDB");
        println!("Found {} foreign proofs in SurrealDB", proofs.len());
        let mut by_mint: HashMap<_, Vec<_>> = HashMap::new();
        for (mint_id, proof) in proofs {
            by_mint.entry(mint_id).or_default().push(proof);
        }
        for (mint_id, proofs) in by_mint {
            sqlx_foreign
                .store(mint_id, proofs)
                .await
                .expect("Failed to migrate foreign proofs");
        }
        // foreign - htlcs
        let htlcs = surreal_foreign
            .dump_htlcs()
            .await
            .expect("Failed to list foreign htlcs from SurrealDB");
        println!("Found {} foreign htlcs in SurrealDB", htlcs.len());
        let mut by_lock: HashMap<_, Vec<_>> = HashMap::new();
        for (mint_id, hash, proof) in htlcs {
            by_lock.entry((mint_id, hash)).or_default().push(proof);
        }
        for ((mint_id, hash), proofs) in by_lock {
            sqlx_foreign
                .store_htlc(mint_id, hash, proofs)
                .await
                .expect("Failed to migrate foreign htlcs");
        }
        // foreign - issued
        let issued = surreal_foreign
            .dump_issued()
            .await
            .expect("Failed to list foreign issued proofs from SurrealDB");
        println!("Found {} foreign issued proofs in SurrealDB", issued.len());
        let mut by_lock: HashMap<_, Vec<_>> = HashMap::new();
        for (hash, locktime, proof) in issued {
            by_lock.entry((hash, locktime)).or_default().push(proof);
        }
        for ((hash, locktime), proofs) in by_lock {
            sqlx_foreign
                .store_issued(hash, locktime, proofs)
                .await
                .expect("Failed to migrate foreign issued proofs");
        }
        surreal_foreign
            .mark_migrated()
            .await
            .expect("Failed to mark foreign online as migrated in SurrealDB");
    }
}
