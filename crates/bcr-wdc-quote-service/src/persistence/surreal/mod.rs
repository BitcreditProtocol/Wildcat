// ----- standard library imports
// ----- extra library imports
// ----- local modules
pub mod keysets;
pub mod quotes;
// ----- local imports

#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct ConnectionConfig {
    pub connection: String,
    pub namespace: String,
    pub database: String,
    pub table: String,
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct DBConfig {
    pub quotes: ConnectionConfig,
    pub quotes_keys: ConnectionConfig,
}
