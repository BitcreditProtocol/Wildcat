// ----- standard library imports
// ----- extra library imports
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
// ----- local imports

// ----- end imports

///--------------------------- ExchangeRequest
#[derive(Debug, Serialize, Deserialize, Clone, ToSchema)]
pub struct OnlineExchangeRequest {
    pub proofs: Vec<cashu::Proof>,
    pub exchange_path: Vec<cashu::PublicKey>,
}

///--------------------------- ExchangeResponse
#[derive(Debug, Serialize, Deserialize, Clone, ToSchema)]
pub struct OnlineExchangeResponse {
    pub proofs: Vec<cashu::Proof>,
}

///--------------------------- HtlcSwapAttemptRequest
#[derive(Debug, Serialize, Deserialize, Clone, ToSchema)]
pub struct HtlcSwapAttemptRequest {
    pub preimage: String,
}
