// ----- standard library imports
// ----- extra library imports
// ----- local imports

// ----- end imports

pub enum DBBackend {
    Surreal,
    Postgres,
    DefaultPostgres,
}

pub fn select_db_backend(
    component: &str,
    migrated: bool,
    pg_configuration: postgres::DBConnConfig,
) -> DBBackend {
    let we_have_connections = pg_configuration.max_connections > 0;
    match (migrated, we_have_connections) {
        (true, true) => DBBackend::Postgres,
        (false, false) => DBBackend::Surreal,
        (true, false) => {
            tracing::warn!("################################################################");
            tracing::warn!("#  WARNING: {component} is marked as migrated to PostgreSQL");
            tracing::warn!("#  but PostgreSQL is NOT configured (max_connections == 0):");
            tracing::warn!("#  running on Postgres with default settings.");
            tracing::warn!("#  Set repository_new.max_connections > 0.");
            tracing::warn!("################################################################");
            DBBackend::DefaultPostgres
        }
        (false, true) => {
            tracing::warn!("################################################################");
            tracing::warn!("#  WARNING: {component} is not marked as migrated to PostgreSQL");
            tracing::warn!("#  but PostgreSQL is configured (max_connections > 0):");
            tracing::warn!("#  running on SurrealDB");
            tracing::warn!("#  Please run the migration script.");
            tracing::warn!("################################################################");
            DBBackend::Surreal
        }
    }
}

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
    use std::{str::FromStr, time::Duration};

    use sqlx::{
        postgres::{PgConnectOptions, PgConnection, PgPool, PgPoolOptions},
        ConnectOptions, Connection,
    };

    #[derive(Debug, Clone, serde::Deserialize)]
    pub struct DBConnConfig {
        pub connection: String,
        pub max_connections: u32,
    }

    /// sqlx logs every statement at DEBUG by default, flooding the service logs:
    fn connect_options(cfg: &DBConnConfig) -> Result<PgConnectOptions, sqlx::Error> {
        let options = PgConnectOptions::from_str(&cfg.connection)?
            .log_statements(log::LevelFilter::Info)
            .log_slow_statements(log::LevelFilter::Warn, Duration::from_millis(100));
        Ok(options)
    }

    /// `max_connections == 0` (PostgreSQL not configured) falls back to sqlx's
    pub async fn connect(cfg: &DBConnConfig) -> Result<PgPool, sqlx::Error> {
        let mut pool = PgPoolOptions::new();
        if cfg.max_connections > 0 {
            pool = pool.max_connections(cfg.max_connections);
        } else {
            tracing::warn!(
                "PostgreSQL max_connections == 0, using default sqlx connection pool settings"
            );
        }
        pool.connect_with(connect_options(cfg)?).await
    }

    pub async fn run_migration(cfg: &DBConnConfig) {
        let options = connect_options(cfg).expect("Invalid database connection string");
        let mut conn = PgConnection::connect_with(&options)
            .await
            .expect("Failed to connect to database");
        sqlx::migrate!("./migrations")
            .run(&mut conn)
            .await
            .expect("Failed to run migration");
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
