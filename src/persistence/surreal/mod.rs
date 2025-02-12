// ----- standard library imports
// ----- extra library imports
// ----- local modules
pub mod quotes;
pub mod keysets;
// ----- local imports


#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct ConnectionConfig {
    pub connection: String,
    pub namespace: String,
    pub database: String,
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct DBConfig {
    pub quotes: ConnectionConfig,
    pub quoteskeys: ConnectionConfig,
}
