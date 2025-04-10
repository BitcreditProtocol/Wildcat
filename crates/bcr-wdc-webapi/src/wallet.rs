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
    #[schema(value_type=u64)]
    pub immature: Amount,
    #[schema(value_type=u64)]
    pub trusted_pending: Amount,
    #[schema(value_type=u64)]
    pub untrusted_pending: Amount,
    #[schema(value_type=u64)]
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
