// ----- standard library imports
// ----- extra library imports
use bcr_common::client::Url as ClientUrl;
use bcr_wdc_utils::{postgres, surreal};
// ----- local imports

// ----- end imports:

#[derive(Clone, Debug, serde::Deserialize)]
pub struct App {
    pub onchain: Onchain,
    pub foreign: Foreign,
    pub ebill: Ebill,
    pub vault: Vault,
    pub core_url: ClientUrl,
    pub ebill_url: ClientUrl,
    pub clowder_rest_url: ClientUrl,
    pub clowder_nats_url: ClientUrl,
    pub cache_expiry_sec: u64,
}

#[derive(Clone, Debug, serde::Deserialize)]
pub struct Onchain {
    pub db: surreal::DBConnConfig,
    pub new: postgres::DBConnConfig,
    pub monitor_interval_sec: u32,
    pub melt_quote_expiry_seconds: u32,
    pub mint_quote_expiry_seconds: u32,
    pub min_confirmations: u32,
    pub min_melt_threshold: bitcoin::Amount,
    pub min_mint_threshold: bitcoin::Amount,
}

#[derive(Clone, Debug, serde::Deserialize)]
pub struct Foreign {
    pub online_repo: surreal::DBConnConfig,
    pub new_online_repo: postgres::DBConnConfig,
    pub offline_repo: surreal::DBConnConfig,
}

#[derive(Clone, Debug, serde::Deserialize)]
pub struct Ebill {
    pub db: surreal::DBConnConfig,
    pub new: postgres::DBConnConfig,
}

#[derive(Clone, Debug, serde::Deserialize)]
pub struct Vault {
    pub db: surreal::DBConnConfig,
    pub new: postgres::DBConnConfig,
}
