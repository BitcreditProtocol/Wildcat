// ----- standard library imports
// ----- extra library imports
use bcr_wdc_core_service::persistence::{self, Repository as _};
// ----- local imports

// ----- end imports

#[derive(Debug, serde::Deserialize)]
struct MigrateConfig {
    appcfg: bcr_wdc_core_service::config::App,
}

#[tokio::main]
async fn main() {
    let dry_run = std::env::args().any(|a| a == "--dry-run");
    let settings = config::Config::builder()
        .add_source(config::File::with_name("config.toml"))
        .add_source(config::Environment::with_prefix("CORE_SERVICE").separator("__"))
        .build()
        .expect("Failed to build config");
    let cfg: MigrateConfig = settings
        .try_deserialize()
        .expect("Failed to parse migrate config");
    // Connect to SurrealDB (source)
    let surreal_repository = persistence::surreal::Repository::new(cfg.appcfg.repository)
        .await
        .expect("Failed to connect to SurrealDB");
    let migrated = surreal_repository
        .is_migrated()
        .await
        .expect("Failed to read migration marker from SurrealDB");
    if migrated {
        println!("core DB already migrated, nothing to do");
        return;
    }
    if dry_run {
        println!("DRY RUN: Would migrate");
        let keys = surreal_repository
            .dump_keys()
            .await
            .expect("Failed to list keys from SurrealDB");
        println!("   {} keysets in SurrealDB", keys.len());
        let signatures = surreal_repository
            .dump_signatures()
            .await
            .expect("Failed to list signatures from SurrealDB");
        println!("   {} signatures in SurrealDB", signatures.len());
        let commitments = surreal_repository
            .dump_commitments()
            .await
            .expect("Failed to list commitments from SurrealDB");
        println!("   {} commitments in SurrealDB", commitments.len());
        let reserved_ys = surreal_repository
            .dump_reserved_ys()
            .await
            .expect("Failed to list reserved ys from SurrealDB");
        println!("   {} reserved ys in SurrealDB", reserved_ys.len());
        let proofs = surreal_repository
            .dump_proofs()
            .await
            .expect("Failed to list proofs from SurrealDB");
        println!("   {} proofs in SurrealDB", proofs.len());
        return;
    }
    // Connect to PostgreSQL (destination)
    bcr_wdc_utils::db::postgres::run_migration(&cfg.appcfg.repository_new).await;
    let pool = sqlx::postgres::PgPool::connect(&cfg.appcfg.repository_new.connection)
        .await
        .expect("Failed to connect to PostgreSQL");
    let sqlx_repository = persistence::sqlx::Repository::from_pool(pool);
    let keys = surreal_repository
        .dump_keys()
        .await
        .expect("Failed to list keys from SurrealDB");
    println!("Found {} keysets in SurrealDB", keys.len());
    for keyset in keys {
        let kid = keyset.id;
        if let Err(e) = sqlx_repository.keys_store(keyset).await {
            eprintln!("Failed to migrate keyset {kid}: {e}");
            std::process::exit(1);
        }
    }
    // signatures
    let signatures = surreal_repository
        .dump_signatures()
        .await
        .expect("Failed to list signatures from SurrealDB");
    println!("Found {} signatures in SurrealDB", signatures.len());
    for (y, signature) in signatures {
        if let Err(e) = sqlx_repository.signature_store(y, signature).await {
            eprintln!("Failed to migrate signature {y}: {e}");
            std::process::exit(1);
        }
    }
    // Commitments, reserved ys and proofs are migrated in that order.
    //
    // PostgreSQL folds the three SurrealDB tables `commitments`, `reserved_ys` and
    // `proofs` into the single `core_proofs` table, where one `y` is at most one of
    // committed / reserved / spent. SurrealDB keeps them apart and lets the same `y`
    // appear in all three, so migrating in this order lets the spend win: `insert_v0`
    // upgrades a committed or reserved row to a spent one.
    //
    // commitments
    let commitments = surreal_repository
        .dump_commitments()
        .await
        .expect("Failed to list commitments from SurrealDB");
    println!("Found {} commitments in SurrealDB", commitments.len());
    for commitment in commitments {
        let signature = commitment.signature;
        if let Err(e) = sqlx_repository
            .commitment_store(
                commitment.inputs,
                commitment.outputs,
                commitment.expiration,
                commitment.wallet_key,
                signature,
                commitment.fp_digest,
                commitment.signed,
            )
            .await
        {
            eprintln!("Failed to migrate commitment {signature}: {e}");
            std::process::exit(1);
        }
    }
    // reserved ys
    let reserved_ys = surreal_repository
        .dump_reserved_ys()
        .await
        .expect("Failed to list reserved ys from SurrealDB");
    println!("Found {} reserved ys in SurrealDB", reserved_ys.len());
    for (y, deadline) in reserved_ys {
        if let Err(e) = sqlx_repository.ys_store(vec![y], deadline).await {
            eprintln!("Failed to migrate reserved y {y}: {e}");
            std::process::exit(1);
        }
    }
    // proofs
    let proofs = surreal_repository
        .dump_proofs()
        .await
        .expect("Failed to list proofs from SurrealDB");
    println!("Found {} proofs in SurrealDB", proofs.len());
    if let Err(e) = persistence::sqlx::insert_v0(&sqlx_repository, proofs).await {
        eprintln!("Failed to migrate proofs: {e}");
        std::process::exit(1);
    }
    surreal_repository
        .mark_migrated()
        .await
        .expect("Failed to mark core DB as migrated in SurrealDB");
    println!("Migration complete");
}
