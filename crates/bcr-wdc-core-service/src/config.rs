// ----- standard library imports
// ----- extra library imports
use bcr_common::client;
use bcr_wdc_utils::{postgres, surreal};
use bitcoin::bip32 as btc32;
// ----- local imports

// ----- end imports:

#[derive(Clone, Debug, serde::Deserialize)]
pub struct App {
    pub repository: surreal::DBConnConfig,
    pub repository_new: postgres::DBConnConfig,
    pub clowder_url: client::Url,
    pub treasury_url: client::Url,
    pub clowder_rest_url: client::Url,
    pub starting_derivation_path: btc32::DerivationPath,
    pub max_expiry_sec: u64,
    pub minimum_keyset_fees_ppk: u64,
    pub cache_expiry_sec: u64,
    pub settle_window_sec: u64,
}
