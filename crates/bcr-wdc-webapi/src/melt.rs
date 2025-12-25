// ----- standard library imports
// ----- extra library imports
use bcr_common::wire::melt::MeltQuoteOnchainRequest;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
// ----- local imports

// ----- end imports

///--------------------------- store onchain melt
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct StoreOnchainMeltRequest {
    #[schema(value_type = Object)]
    pub melt_request: MeltQuoteOnchainRequest,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct StoreOnchainMeltResponse {
    pub quote_id: uuid::Uuid,
}

///--------------------------- load onchain melt
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct LoadOnchainMeltRequest {
    pub quote_id: uuid::Uuid,
}
