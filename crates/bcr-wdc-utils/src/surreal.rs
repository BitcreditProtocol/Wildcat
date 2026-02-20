// ----- standard library imports
// ----- extra library imports
// ----- local imports

// ----- end imports

#[derive(Debug, Clone, serde::Deserialize)]
pub struct DBConnConfig {
    pub connection: String,
    pub namespace: String,
    pub database: String,
}
