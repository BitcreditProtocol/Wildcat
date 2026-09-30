// ----- standard library imports
// ----- extra library imports
// ----- local imports

// ----- end imports

pub mod surreal {
    #[derive(Debug, Clone, serde::Deserialize)]
    pub struct DBConnConfig {
        pub connection: String,
        pub namespace: String,
        pub database: String,
    }

    /// Renders a timestamp the way stored records serialize it, so bound query
    /// parameters stay comparable against persisted values.
    pub fn tstamp_param(tstamp: crate::TStamp) -> String {
        tstamp
            .format(&time::format_description::well_known::Rfc3339)
            .expect("rfc3339 timestamp")
    }
}

pub mod postgres {
    use std::{future::Future, pin::Pin};

    use anyhow::Context;
    use sqlx::Connection;

    #[derive(Debug, Clone, serde::Deserialize)]
    pub struct DBConnConfig {
        pub connection: String,
        pub max_connections: u32,
    }

    // One session lock per destination database, including when targets share a database.
    // Never use a pooled connection: dropping this connection must release the lock.
    const DATA_IMPORT_LOCK: i64 = 0x5744_4349_4d50_4f52;

    pub type ImportFuture<'a> = Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>>;

    /// Run a versioned data import once. The callback receives the transaction's
    /// connection, or None for a dry run. It must perform and validate all writes
    /// on that connection; source connections belong inside the callback so that
    /// completed imports do not need the source to be available.
    pub async fn run_data_import<F>(
        cfg: &DBConnConfig,
        import_id: &str,
        dry_run: bool,
        import: F,
    ) -> anyhow::Result<()>
    where
        F: for<'a> FnOnce(Option<&'a mut sqlx::PgConnection>) -> ImportFuture<'a> + Send,
    {
        async {
            if dry_run {
                return import(None).await;
            }

            let mut conn = sqlx::PgConnection::connect(&cfg.connection)
                .await
                .context("Failed to connect to destination PostgreSQL")?;
            println!("Waiting for destination data-import lock: {import_id}");
            sqlx::query("SELECT pg_advisory_lock($1)")
                .bind(DATA_IMPORT_LOCK)
                .execute(&mut conn)
                .await
                .context("Failed to acquire destination data-import lock")?;
            // Schema evolution is independent of the one-time data import. Even
            // completed targets must apply later migrations without their source.
            sqlx::migrate!("./migrations")
                .run_direct(&mut conn)
                .await
                .context("Failed to apply destination schema migrations")?;
            // Deliberately separate from SQLx's schema-migration bookkeeping.
            sqlx::query(
                "CREATE TABLE IF NOT EXISTS wdc_data_imports (
                    id TEXT PRIMARY KEY,
                    completed_at TIMESTAMPTZ NOT NULL DEFAULT now()
                )",
            )
            .execute(&mut conn)
            .await?;
            let completed: bool =
                sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM wdc_data_imports WHERE id = $1)")
                    .bind(import_id)
                    .fetch_one(&mut conn)
                    .await?;
            if completed {
                println!("Data import {import_id} was already applied; skipping");
                conn.close().await.context("Failed to close destination connection")?;
                return Ok(());
            }

            let mut tx = conn.begin().await?;
            if let Err(error) = import(Some(&mut tx)).await {
                return match tx.rollback().await {
                    Ok(()) => Err(error).context("Import rolled back; fix the error and rerun with source writers stopped"),
                    Err(rollback_error) => Err(error).context(format!(
                        "Rollback also failed: {rollback_error}; the destination connection will close. Fix the error and rerun with source writers stopped",
                    )),
                };
            }
            let recorded = sqlx::query("INSERT INTO wdc_data_imports (id) VALUES ($1)")
                .bind(import_id)
                .execute(&mut *tx)
                .await?;
            anyhow::ensure!(
                recorded.rows_affected() == 1,
                "Completion marker was not written"
            );
            tx.commit()
                .await
                .context("Data-import commit failed; rerun to check completion state")?;
            conn.close().await.context("Failed to close destination connection")?;
            println!("Data import {import_id} complete");
            Ok(())
        }
        .await
        .with_context(|| format!("Data import {import_id}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(serde::Serialize)]
    struct StoredRecord {
        #[serde(with = "time::serde::rfc3339")]
        tstamp: crate::TStamp,
    }

    // a bound query parameter must render identically to the stored field, or
    // surreal compares a string against a serialized array and silently matches
    // every row.
    #[test]
    fn tstamp_param_matches_stored_representation() {
        let tstamp = time::macros::datetime!(2026-08-03 12:00:00 UTC);
        let stored = serde_json::to_value(StoredRecord { tstamp }).unwrap();
        assert_eq!(stored["tstamp"], "2026-08-03T12:00:00Z");
        assert_eq!(surreal::tstamp_param(tstamp), "2026-08-03T12:00:00Z");
        assert_eq!(stored["tstamp"], surreal::tstamp_param(tstamp));
    }
}
