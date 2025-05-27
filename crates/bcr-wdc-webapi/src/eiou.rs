// ----- standard library imports
// ----- extra library imports
use bitcoin::Amount;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
// ----- local imports

// ----- end imports

///--------------------------- Balance
#[derive(Serialize, Deserialize, ToSchema, Debug)]
pub struct BalanceResponse {
    #[schema(value_type=u64)]
    pub outstanding: Amount,
    #[schema(value_type=u64)]
    pub treasury: Amount,
}
