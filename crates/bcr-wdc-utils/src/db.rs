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
}

pub mod postgres {
    use sqlx::Connection;

    #[derive(Debug, Clone, serde::Deserialize)]
    pub struct DBConnConfig {
        pub connection: String,
        pub max_connections: u32,
    }

    pub async fn run_migration(cfg: &DBConnConfig) {
        let mut conn = sqlx::postgres::PgConnection::connect(&cfg.connection)
            .await
            .expect("Failed to connect to database");
        sqlx::migrate!("./migrations")
            .run(&mut conn)
            .await
            .expect("Failed to run migration");
    }
}
