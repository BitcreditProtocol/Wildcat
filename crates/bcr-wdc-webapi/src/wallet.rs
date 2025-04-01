// ----- standard library imports
// ----- extra library imports
use bdk_wallet::bitcoin::Amount;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
// ----- local imports

// ----- end imports

///--------------------------- onchain wallet balance
#[derive(Serialize, Deserialize, ToSchema)]
pub struct Balance {
    pub immature: Amount,
    pub trusted_pending: Amount,
    pub untrusted_pending: Amount,
    pub confirmed: Amount,
}

impl std::convert::From<bdk_wallet::Balance> for Balance {
    fn from(blnc: bdk_wallet::Balance) -> Self {
        Self {
            immature: blnc.immature,
            trusted_pending: blnc.trusted_pending,
            untrusted_pending: blnc.untrusted_pending,
            confirmed: blnc.confirmed,
        }
    }
}
