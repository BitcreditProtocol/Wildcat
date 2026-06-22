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

pub mod redis {
    #[derive(Debug, Clone, serde::Deserialize)]
    pub struct DBConnConfig {
        pub url: url::Url,
        pub timeout_seconds: u64,
    }
}
