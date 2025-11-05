// ----- standard library imports
// ----- extra library imports
use bcr_common::wire::{borsh as wire_borsh, keys as wire_keys};
use bitcoin::secp256k1;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
// ----- local imports

// ----- end imports

///--------------------------- Online ExchangeRequest
#[derive(Debug, Serialize, Deserialize, Clone, ToSchema)]
pub struct OnlineExchangeRequest {
    pub proofs: Vec<cashu::Proof>,
    #[schema(value_type = Vec<String>)]
    pub exchange_path: Vec<secp256k1::PublicKey>,
}

///--------------------------- Online ExchangeResponse
#[derive(Debug, Serialize, Deserialize, Clone, ToSchema)]
pub struct OnlineExchangeResponse {
    pub proofs: Vec<cashu::Proof>,
}

///--------------------------- Offline ExchangeRequest
#[derive(Debug, Serialize, Deserialize, Clone, ToSchema)]
pub struct OfflineExchangeRequest {
    pub fingerprints: Vec<wire_keys::ProofFingerprint>,
    #[schema(value_type = Vec<String>)]
    pub hashes: Vec<bitcoin::hashes::sha256::Hash>,
    pub wallet_pk: cashu::PublicKey,
}

///--------------------------- Offline ExchangePayload
#[derive(Debug, Clone, borsh::BorshSerialize, borsh::BorshDeserialize)]
pub struct OfflineExchangePayload {
    #[borsh(
        serialize_with = "wire_borsh::serialize_vecof_cdkproof",
        deserialize_with = "wire_borsh::deserialize_vecof_cdkproof"
    )]
    pub proofs: Vec<cashu::Proof>,
}

///--------------------------- Offline ExchangeResponse
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct OfflineExchangeResponse {
    pub content: String, // b64 borsh-serialized OfflineExchangePayload
    #[schema(value_type = String)]
    pub signature: bitcoin::secp256k1::schnorr::Signature,
}

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
