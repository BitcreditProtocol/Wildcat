// ----- standard library imports
// ----- extra library imports
use bcr_common::{cashu, client};
use bcr_wdc_utils::postgres;
use bitcoin::bip32 as btc32;
// ----- local imports

// ----- end imports:

#[derive(Clone, Debug, serde::Deserialize)]
pub struct App {
    pub repository: postgres::DBConnConfig,
    pub clowder_url: client::Url,
    #[serde(default)]
    pub clowder_nkey_seed: Option<String>,
    pub clowder_rest_url: client::Url,
    pub starting_derivation_path: btc32::DerivationPath,
    pub max_expiry_sec: u64,
    pub minimum_keyset_fees_ppk: u64,
    pub cache_expiry_sec: u64,
    pub settle_window_sec: u64,
    pub ebill: Ebill,
    pub ebill_url: client::Url,
}

#[derive(Clone, Debug, serde::Deserialize)]
pub struct Ebill {
    #[serde(default = "default_multiplier")]
    pub multiplier: cashu::Amount,
}

fn default_multiplier() -> cashu::Amount {
    cashu::Amount::ONE
}
