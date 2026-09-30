use anyhow::Context;
use bcr_wdc_core_service::{
    config::App as AppCfg,
    persistence::{sqlx::data_import, surreal},
};
use bcr_wdc_utils::{postgres, surreal::DBConnConfig as SourceConfig};

#[derive(Debug, serde::Deserialize)]
struct MigrateConfig {
    appcfg: AppCfg,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let dry_run = std::env::args().any(|arg| arg == "--dry-run");
    let settings = config::Config::builder()
        .add_source(config::File::with_name("config.toml"))
        .add_source(config::Environment::with_prefix("CORE_SERVICE").separator("__"))
        .build()
        .context("Failed to build migrate config")?;
    let cfg: MigrateConfig = settings
        .try_deserialize()
        .context("Failed to parse migrate config")?;
    migrate(cfg.appcfg.repository, &cfg.appcfg.repository_new, dry_run).await
}

async fn migrate(
    source: SourceConfig,
    destination: &postgres::DBConnConfig,
    dry_run: bool,
) -> anyhow::Result<()> {
    postgres::run_data_import(destination, data_import::IMPORT_ID, dry_run, move |conn| {
        Box::pin(async move {
            let source = surreal::Repository::new(source)
                .await
                .context("Failed to connect to Core SurrealDB")?;
            let keys = source.dump_keys().await.context("Failed to list keys")?;
            let signatures = source
                .dump_signatures()
                .await
                .context("Failed to list signatures")?;
            let commitments = source
                .dump_commitments()
                .await
                .context("Failed to list commitments")?;
            let reserved_ys = source
                .dump_reserved_ys()
                .await
                .context("Failed to list reserved ys")?;
            let proofs = source
                .dump_proofs()
                .await
                .context("Failed to list proofs")?;
            println!("Found {} keysets in SurrealDB", keys.len());
            println!("Found {} signatures in SurrealDB", signatures.len());
            println!("Found {} commitments in SurrealDB", commitments.len());
            println!("Found {} reserved ys in SurrealDB", reserved_ys.len());
            println!("Found {} proofs in SurrealDB", proofs.len());
            if let Some(conn) = conn {
                data_import::import(conn, keys, signatures, commitments, reserved_ys, proofs).await?;
            } else {
                println!(
                    "DRY RUN: Would migrate {} keysets, {} signatures, {} commitments, {} reserved ys, {} proofs to PostgreSQL",
                    keys.len(),
                    signatures.len(),
                    commitments.len(),
                    reserved_ys.len(),
                    proofs.len()
                );
            }
            Ok(())
        })
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use ::sqlx::{ConnectOptions, PgPool};

    fn source(connection: &str) -> SourceConfig {
        SourceConfig {
            connection: connection.to_owned(),
            namespace: "migration-test".to_owned(),
            database: "core".to_owned(),
        }
    }

    #[sqlx::test(migrations = false)]
    #[ignore = "requires DATABASE_URL with CREATEDB permission"]
    async fn core_completed_import_skips_unavailable_surreal_source(pool: PgPool) {
        let destination = postgres::DBConnConfig {
            connection: pool.connect_options().to_url_lossy().to_string(),
            max_connections: 1,
        };
        migrate(source("mem://"), &destination, false)
            .await
            .unwrap();
        migrate(source("unavailable://"), &destination, false)
            .await
            .unwrap();
        // Dry runs still read/report the source, even for an applied import.
        assert!(migrate(source("unavailable://"), &destination, true)
            .await
            .is_err());
    }

    #[sqlx::test(migrations = false)]
    #[ignore = "requires DATABASE_URL with CREATEDB permission"]
    async fn core_dry_run_leaves_postgres_unchanged(pool: PgPool) {
        let destination = postgres::DBConnConfig {
            connection: pool.connect_options().to_url_lossy().to_string(),
            max_connections: 1,
        };
        migrate(source("mem://"), &destination, true).await.unwrap();
        let tables: i64 =
            sqlx::query_scalar("SELECT count(*) FROM pg_tables WHERE schemaname = 'public'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(tables, 0);
    }
}
