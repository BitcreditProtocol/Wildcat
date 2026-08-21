// ----- standard library imports
// ----- extra library imports
use bcr_common::{cashu, client::Url as ClientUrl};
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
    pub melt_fee_ppk: u64,
    pub min_mint_threshold: bitcoin::Amount,
    #[serde(default = "default_min_feerate_sat_per_vb")]
    pub min_feerate_sat_per_vb: f64,
}

fn default_min_feerate_sat_per_vb() -> f64 {
    0.1
}

#[derive(Clone, Debug, serde::Deserialize)]
pub struct Foreign {
    pub online_repo: surreal::DBConnConfig,
    pub new_online_repo: postgres::DBConnConfig,
    pub offline_repo: surreal::DBConnConfig,
    /// How much sooner the eCash this mint issues expires than the collateral it
    /// holds, so it can reclaim before its own claim lapses.
    #[serde(default = "default_exchange_lock_margin_secs")]
    pub exchange_lock_margin_secs: u64,
}

fn default_exchange_lock_margin_secs() -> u64 {
    15 * 60
}

#[derive(Clone, Debug, serde::Deserialize)]
pub struct Ebill {
    pub db: surreal::DBConnConfig,
    pub new: postgres::DBConnConfig,
    #[serde(default = "default_multiplier")]
    pub multiplier: cashu::Amount,
}

fn default_multiplier() -> cashu::Amount {
    cashu::Amount::ONE
}

#[derive(Clone, Debug, serde::Deserialize)]
pub struct Vault {
    pub db: surreal::DBConnConfig,
    pub new: postgres::DBConnConfig,
}
