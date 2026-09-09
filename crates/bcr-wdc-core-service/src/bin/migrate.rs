// ----- standard library imports
// ----- extra library imports
use bcr_wdc_core_service::{
    config::App as AppCfg,
    persistence::{sqlx, surreal, Repository},
};
// ----- local imports

// ----- end imports

#[derive(Debug, serde::Deserialize)]
struct MigrateConfig {
    appcfg: AppCfg,
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
    let commitments = surreal_repository
        .dump_commitments()
        .await
        .expect("Failed to list commitments from SurrealDB");
    let reserved_ys = surreal_repository
        .dump_reserved_ys()
        .await
        .expect("Failed to list reserved ys from SurrealDB");
    let proofs = surreal_repository
        .dump_proofs()
        .await
        .expect("Failed to list proofs from SurrealDB");
    println!("Found {} keysets in SurrealDB", keys.len());
    println!("Found {} signatures in SurrealDB", signatures.len());
    println!("Found {} commitments in SurrealDB", commitments.len());
    println!("Found {} reserved ys in SurrealDB", reserved_ys.len());
    println!("Found {} proofs in SurrealDB", proofs.len());
    if dry_run {
        println!("DRY RUN: Would migrate");
        println!("   {} keysets to PostgreSQL", keys.len());
        println!("   {} signatures to PostgreSQL", signatures.len());
        println!("   {} commitments to PostgreSQL", commitments.len());
        println!("   {} reserved ys to PostgreSQL", reserved_ys.len());
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
        let kid = keyset.id;
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
    // Migrate commitments, reserved ys and proofs to PostgreSQL, in that order.
    //
    // PostgreSQL folds the three SurrealDB tables `commitments`, `reserved_ys` and
    // `proofs` into the single `core_proofs` table, where one `y` is at most one of
    // committed / reserved / spent. SurrealDB keeps them apart and lets the same `y`
    // appear in all three, so migrating in this order lets the spend win: `insert_v0`
    // upgrades a committed or reserved row to a spent one, while `commitment_store`
    // and `ys_store` refuse to overwrite anything.
    for commitment in commitments {
        let signature = commitment.signature;
        if let Err(error) = sqlx_repository
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
            println!("Skipping commitment {signature}: failed with {error}");
        }
    }
    println!("Migration for commitments complete");
    // one `y` per call: `ys_store` rolls the whole batch back on a single conflict,
    // and a conflict is expected for any `y` SurrealDB held as both reserved and
    // committed.
    for (y, deadline) in reserved_ys {
        if let Err(error) = sqlx_repository.ys_store(vec![y], deadline).await {
            println!("Skipping reserved y {y}: failed with {error}");
        }
    }
    println!("Migration for reserved ys complete");
    sqlx::insert_v0(&sqlx_repository, proofs)
        .await
        .expect("SqlxRepository::insert_v0 failed");
    println!("Migration for proofs complete");
}
