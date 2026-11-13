// ----- standard library imports
// ----- extra library imports
use bcr_wdc_core_service::persistence::{self, Repository as _};
// ----- local imports

// ----- end imports

fn default_batch_size() -> usize {
    5000
}

#[derive(Debug, serde::Deserialize)]
struct MigrateConfig {
    appcfg: bcr_wdc_core_service::config::App,
    /// rows read from SurrealDB per query, overridable via CORE_SERVICE__BATCH_SIZE
    #[serde(default = "default_batch_size")]
    batch_size: usize,
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
    let batch_size = cfg.batch_size;
    assert!(batch_size > 0, "batch_size must be greater than 0");
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
            .count_signatures()
            .await
            .expect("Failed to count signatures in SurrealDB");
        println!("   {signatures} signatures in SurrealDB");
        let commitments = surreal_repository
            .count_commitments()
            .await
            .expect("Failed to count commitments in SurrealDB");
        println!("   {commitments} commitments in SurrealDB");
        let reserved_ys = surreal_repository
            .count_reserved_ys()
            .await
            .expect("Failed to count reserved ys in SurrealDB");
        println!("   {reserved_ys} reserved ys in SurrealDB");
        let proofs = surreal_repository
            .count_proofs()
            .await
            .expect("Failed to count proofs in SurrealDB");
        println!("   {proofs} proofs in SurrealDB");
        return;
    }
    // Connect to PostgreSQL (destination)
    bcr_wdc_utils::db::postgres::run_migration(&cfg.appcfg.repository_new).await;
    let pool = bcr_wdc_utils::db::postgres::connect(&cfg.appcfg.repository_new)
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
    // Every other table is read in batches of `batch_size` rows, to bound memory usage.
    //
    // signatures
    let mut start = 0;
    loop {
        let signatures = surreal_repository
            .dump_signatures(start, batch_size)
            .await
            .expect("Failed to list signatures from SurrealDB");
        let len = signatures.len();
        for (y, signature) in signatures {
            if let Err(e) = sqlx_repository.signature_store(y, signature).await {
                eprintln!("Failed to migrate signature {y}: {e}");
                std::process::exit(1);
            }
        }
        start += len;
        if len < batch_size {
            break;
        }
    }
    println!("Found {start} signatures in SurrealDB");
    // Commitments, reserved ys and proofs are migrated in that order.
    //
    // PostgreSQL folds the three SurrealDB tables `commitments`, `reserved_ys` and
    // `proofs` into the single `core_proofs` table, where one `y` is at most one of
    // committed / reserved / spent. SurrealDB keeps them apart and lets the same `y`
    // appear in all three, so migrating in this order lets the spend win: `insert_v0`
    // upgrades a committed or reserved row to a spent one.
    //
    // commitments
    let mut start = 0;
    loop {
        let commitments = surreal_repository
            .dump_commitments(start, batch_size)
            .await
            .expect("Failed to list commitments from SurrealDB");
        let len = commitments.len();
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
        start += len;
        if len < batch_size {
            break;
        }
    }
    println!("Found {start} commitments in SurrealDB");
    // reserved ys
    let mut start = 0;
    loop {
        let reserved_ys = surreal_repository
            .dump_reserved_ys(start, batch_size)
            .await
            .expect("Failed to list reserved ys from SurrealDB");
        let len = reserved_ys.len();
        for (y, deadline) in reserved_ys {
            if let Err(e) = sqlx_repository.ys_store(vec![y], deadline).await {
                eprintln!("Failed to migrate reserved y {y}: {e}");
                std::process::exit(1);
            }
        }
        start += len;
        if len < batch_size {
            break;
        }
    }
    println!("Found {start} reserved ys in SurrealDB");
    // proofs
    let mut start = 0;
    loop {
        let proofs = surreal_repository
            .dump_proofs(start, batch_size)
            .await
            .expect("Failed to list proofs from SurrealDB");
        let len = proofs.len();
        if let Err(e) = persistence::sqlx::insert_v0(&sqlx_repository, proofs).await {
            eprintln!("Failed to migrate proofs {start}..{}: {e}", start + len);
            std::process::exit(1);
        }
        start += len;
        if len < batch_size {
            break;
        }
    }
    println!("Found {start} proofs in SurrealDB");
    surreal_repository
        .mark_migrated()
        .await
        .expect("Failed to mark core DB as migrated in SurrealDB");
    println!("Migration complete");
}
