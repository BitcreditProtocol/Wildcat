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

///--------------------------- RequestToMintFromForeigneCash
#[derive(Debug, borsh::BorshSerialize, borsh::BorshDeserialize)]
pub struct RequestToMintFromForeignCashPayload {
    pub foreign_amount_sat: u64,
    pub nonce: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RequestToMintFromForeigneCash {
    pub payload: String, // b64 borsh payload
    pub signature: bitcoin::secp256k1::schnorr::Signature,
}
