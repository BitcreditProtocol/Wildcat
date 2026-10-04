// ----- standard library imports
// ----- extra library imports
use bcr_common::cashu;
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
    // Each table is marked as migrated in SurrealDB once copied: a re-run skips it,
    // and the SurrealDB repository refuses any further write to it.
    let keys_migrated = surreal_repository
        .is_keys_migrated()
        .await
        .expect("Failed to read keys migration marker from SurrealDB");
    let signatures_migrated = surreal_repository
        .is_signatures_migrated()
        .await
        .expect("Failed to read signatures migration marker from SurrealDB");
    let commitments_migrated = surreal_repository
        .is_commitments_migrated()
        .await
        .expect("Failed to read commitments migration marker from SurrealDB");
    let reserved_ys_migrated = surreal_repository
        .is_reserved_ys_migrated()
        .await
        .expect("Failed to read reserved ys migration marker from SurrealDB");
    let proofs_migrated = surreal_repository
        .is_proofs_migrated()
        .await
        .expect("Failed to read proofs migration marker from SurrealDB");
    for (name, migrated) in [
        ("keys", keys_migrated),
        ("signatures", signatures_migrated),
        ("commitments", commitments_migrated),
        ("reserved ys", reserved_ys_migrated),
        ("proofs", proofs_migrated),
    ] {
        if migrated {
            println!("{name} already migrated, skipping");
        }
    }
    if dry_run {
        println!("DRY RUN: Would migrate");
        if !keys_migrated {
            let keys = surreal_repository
                .dump_keys()
                .await
                .expect("Failed to list keys from SurrealDB");
            println!("   {} keysets to PostgreSQL", keys.len());
        }
        if !signatures_migrated {
            let signatures = surreal_repository
                .dump_signatures()
                .await
                .expect("Failed to list signatures from SurrealDB");
            println!("   {} signatures to PostgreSQL", signatures.len());
        }
        if !commitments_migrated {
            let commitments = surreal_repository
                .dump_commitments()
                .await
                .expect("Failed to list commitments from SurrealDB");
            println!("   {} commitments to PostgreSQL", commitments.len());
        }
        if !reserved_ys_migrated {
            let reserved_ys = surreal_repository
                .dump_reserved_ys()
                .await
                .expect("Failed to list reserved ys from SurrealDB");
            println!("   {} reserved ys to PostgreSQL", reserved_ys.len());
        }
        if !proofs_migrated {
            let proofs = surreal_repository
                .dump_proofs()
                .await
                .expect("Failed to list proofs from SurrealDB");
            println!("   {} proofs to PostgreSQL", proofs.len());
        }
        return;
    }
    // Connect to PostgreSQL
    bcr_wdc_utils::db::postgres::run_migration(&cfg.appcfg.repository_new).await;
    let sqlx_repository = sqlx::Repository::new(cfg.appcfg.repository_new)
        .await
        .expect("Failed to connect to PostgreSQL");
    // Migrate keys to PostgreSQL
    if !keys_migrated {
        let keys = surreal_repository
            .dump_keys()
            .await
            .expect("Failed to list keys from SurrealDB");
        println!("Found {} keysets in SurrealDB", keys.len());
        for keyset in keys {
            let kid = keyset.id;
            if let Err(error) = sqlx_repository.keys_store(keyset).await {
                eprintln!("Failed to migrate keyset {kid}: {error}");
                std::process::exit(1);
            }
        }
        surreal_repository
            .mark_keys_migrated()
            .await
            .expect("Failed to mark keys as migrated in SurrealDB");
        println!("Migration for keys complete");
    }
    // Migrate signatures to PostgreSQL
    if !signatures_migrated {
        let signatures = surreal_repository
            .dump_signatures()
            .await
            .expect("Failed to list signatures from SurrealDB");
        println!("Found {} signatures in SurrealDB", signatures.len());
        for (y, signature) in signatures {
            if let Err(error) = sqlx_repository.signature_store(y, signature).await {
                eprintln!("Failed to migrate signature {y}: {error}");
                std::process::exit(1);
            }
        }
        surreal_repository
            .mark_signatures_migrated()
            .await
            .expect("Failed to mark signatures as migrated in SurrealDB");
        println!("Migration for signatures complete");
    }
    // Migrate commitments, reserved ys and proofs to PostgreSQL, in that order.
    //
    // PostgreSQL folds the three SurrealDB tables `commitments`, `reserved_ys` and
    // `proofs` into the single `core_proofs` table, where one `y` is at most one of
    // committed / reserved / spent. SurrealDB keeps them apart and lets the same `y`
    // appear in all three, so migrating in this order lets the spend win: `insert_v0`
    // upgrades a committed or reserved row to a spent one.
    let mut committed_ys: std::collections::HashSet<cashu::PublicKey> =
        std::collections::HashSet::new();
    if !commitments_migrated {
        let commitments = surreal_repository
            .dump_commitments()
            .await
            .expect("Failed to list commitments from SurrealDB");
        println!("Found {} commitments in SurrealDB", commitments.len());
        for commitment in commitments {
            let signature = commitment.signature;
            committed_ys.extend(commitment.inputs.iter().cloned());
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
                eprintln!("Failed to migrate commitment {signature}: {error}");
                std::process::exit(1);
            }
        }
        surreal_repository
            .mark_commitments_migrated()
            .await
            .expect("Failed to mark commitments as migrated in SurrealDB");
        println!("Migration for commitments complete");
    }
    if !reserved_ys_migrated {
        let reserved_ys = surreal_repository
            .dump_reserved_ys()
            .await
            .expect("Failed to list reserved ys from SurrealDB");
        println!("Found {} reserved ys in SurrealDB", reserved_ys.len());
        for (y, deadline) in reserved_ys {
            // A pending swap commitment reserves its own inputs, so the same `y` can show
            // up here too: the commitment already holds it, so skip it rather than fail.
            if committed_ys.contains(&y) {
                continue;
            }
            if let Err(error) = sqlx_repository.ys_store(vec![y], deadline).await {
                eprintln!("Failed to migrate reserved y {y}: {error}");
                std::process::exit(1);
            }
        }
        surreal_repository
            .mark_reserved_ys_migrated()
            .await
            .expect("Failed to mark reserved ys as migrated in SurrealDB");
        println!("Migration for reserved ys complete");
    }
    if !proofs_migrated {
        let proofs = surreal_repository
            .dump_proofs()
            .await
            .expect("Failed to list proofs from SurrealDB");
        println!("Found {} proofs in SurrealDB", proofs.len());
        if let Err(error) = sqlx::insert_v0(&sqlx_repository, proofs).await {
            eprintln!("Failed to migrate proofs: {error}");
            std::process::exit(1);
        }
        surreal_repository
            .mark_proofs_migrated()
            .await
            .expect("Failed to mark proofs as migrated in SurrealDB");
        println!("Migration for proofs complete");
    }
}
