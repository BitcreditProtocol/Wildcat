use anyhow::Context;
use bcr_wdc_treasury_service::persistence::{sqlx::data_import, surreal};
use bcr_wdc_utils::{postgres, surreal::DBConnConfig as SourceConfig};

#[derive(Debug, serde::Deserialize)]
struct MigrateConfig {
    appcfg: bcr_wdc_treasury_service::config::App,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let dry_run = std::env::args().any(|arg| arg == "--dry-run");
    let settings = config::Config::builder()
        .add_source(config::File::with_name("config.toml"))
        .add_source(config::Environment::with_prefix("TREASURY_SERVICE").separator("__"))
        .build()
        .context("Failed to build migrate config")?;
    let cfg: MigrateConfig = settings
        .try_deserialize()
        .context("Failed to parse migrate config")?;
    migrate_ebill(cfg.appcfg.ebill.db, &cfg.appcfg.ebill.new, dry_run).await?;
    migrate_vault(cfg.appcfg.vault.db, &cfg.appcfg.vault.new, dry_run).await?;
    migrate_onchain(cfg.appcfg.onchain.db, &cfg.appcfg.onchain.new, dry_run).await
}

async fn migrate_ebill(
    source: SourceConfig,
    destination: &postgres::DBConnConfig,
    dry_run: bool,
) -> anyhow::Result<()> {
    postgres::run_data_import(
        destination,
        data_import::EBILL_IMPORT_ID,
        dry_run,
        move |conn| {
            Box::pin(async move {
                let source = surreal::DBEbill::new(source)
                    .await
                    .context("Failed to connect to ebill SurrealDB")?;
                let ops = source
                    .dump()
                    .await
                    .context("Failed to list ebill mint_ops")?;
                println!("Found {} ebill mint_ops in SurrealDB", ops.len());
                if let Some(conn) = conn {
                    data_import::import_ebill(conn, ops).await?;
                } else {
                    println!(
                        "DRY RUN: Would migrate {} ebill mint_ops to PostgreSQL",
                        ops.len()
                    );
                }
                Ok(())
            })
        },
    )
    .await
}

async fn migrate_vault(
    source: SourceConfig,
    destination: &postgres::DBConnConfig,
    dry_run: bool,
) -> anyhow::Result<()> {
    postgres::run_data_import(
        destination,
        data_import::VAULT_IMPORT_ID,
        dry_run,
        move |conn| {
            Box::pin(async move {
                let source = surreal::DBVault::new(source)
                    .await
                    .context("Failed to connect to vault SurrealDB")?;
                let proofs = source.dump().await.context("Failed to list vault proofs")?;
                println!("Found {} vault proofs in SurrealDB", proofs.len());
                if let Some(conn) = conn {
                    data_import::import_vault(conn, proofs).await?;
                } else {
                    println!(
                        "DRY RUN: Would migrate {} vault proofs to PostgreSQL",
                        proofs.len()
                    );
                }
                Ok(())
            })
        },
    )
    .await
}

async fn migrate_onchain(
    source: SourceConfig,
    destination: &postgres::DBConnConfig,
    dry_run: bool,
) -> anyhow::Result<()> {
    postgres::run_data_import(
        destination,
        data_import::ONCHAIN_IMPORT_ID,
        dry_run,
        move |conn| {
            Box::pin(async move {
                let source = surreal::DBOnChain::new(source)
                    .await
                    .context("Failed to connect to onchain SurrealDB")?;
                let mintops = source
                    .dump_mintops()
                    .await
                    .context("Failed to list onchain mintops")?;
                let meltops = source
                    .dump_meltops()
                    .await
                    .context("Failed to list onchain meltops")?;
                println!("Found {} onchain mintops in SurrealDB", mintops.len());
                println!("Found {} onchain meltops in SurrealDB", meltops.len());
                if let Some(conn) = conn {
                    data_import::import_onchain(conn, mintops, meltops).await?;
                } else {
                    println!(
                        "DRY RUN: Would migrate {} onchain mintops and {} meltops to PostgreSQL",
                        mintops.len(),
                        meltops.len()
                    );
                }
                Ok(())
            })
        },
    )
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
            database: "treasury".to_owned(),
        }
    }

    #[sqlx::test(migrations = false)]
    #[ignore = "requires DATABASE_URL with CREATEDB permission"]
    async fn treasury_completed_targets_skip_unavailable_surreal_sources(pool: PgPool) {
        let destination = postgres::DBConnConfig {
            connection: pool.connect_options().to_url_lossy().to_string(),
            max_connections: 1,
        };
        migrate_ebill(source("mem://"), &destination, false)
            .await
            .unwrap();
        assert!(migrate_vault(source("unavailable://"), &destination, false)
            .await
            .is_err());
        migrate_ebill(source("unavailable://"), &destination, false)
            .await
            .unwrap();
        migrate_vault(source("mem://"), &destination, false)
            .await
            .unwrap();
        migrate_onchain(source("mem://"), &destination, false)
            .await
            .unwrap();
        migrate_vault(source("unavailable://"), &destination, false)
            .await
            .unwrap();
        migrate_onchain(source("unavailable://"), &destination, false)
            .await
            .unwrap();
        let markers: i64 = sqlx::query_scalar("SELECT count(*) FROM wdc_data_imports")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(markers, 3);
    }

    #[sqlx::test(migrations = false)]
    #[ignore = "requires DATABASE_URL with CREATEDB permission"]
    async fn treasury_dry_run_leaves_postgres_unchanged(pool: PgPool) {
        let destination = postgres::DBConnConfig {
            connection: pool.connect_options().to_url_lossy().to_string(),
            max_connections: 1,
        };
        migrate_ebill(source("mem://"), &destination, true)
            .await
            .unwrap();
        migrate_vault(source("mem://"), &destination, true)
            .await
            .unwrap();
        migrate_onchain(source("mem://"), &destination, true)
            .await
            .unwrap();
        let tables: i64 =
            sqlx::query_scalar("SELECT count(*) FROM pg_tables WHERE schemaname = 'public'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(tables, 0);
    }
}
