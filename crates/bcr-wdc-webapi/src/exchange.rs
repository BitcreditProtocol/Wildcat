// ----- standard library imports
// ----- extra library imports
use bitcoin::secp256k1;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
// ----- local imports

// ----- end imports

///--------------------------- HtlcSwapAttemptRequest
#[derive(Debug, Serialize, Deserialize, Clone, ToSchema)]
pub struct HtlcSwapAttemptRequest {
    pub preimage: String,
}

///--------------------------- RequestToMintFromForeigneCash
#[derive(Debug, borsh::BorshSerialize, borsh::BorshDeserialize)]
pub struct RequestToMintFromForeigneCashPayload {
    pub foreign_amount_sat: u64,
    pub nonce: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RequestToMintFromForeigneCash {
    pub payload: String, // b64 borsh payload
    pub signature: secp256k1::schnorr::Signature,
}
