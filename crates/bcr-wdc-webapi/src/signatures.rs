// ----- standard library imports
// ----- extra library imports
use borsh::{BorshDeserialize, BorshSerialize};
use cashu::{nut00 as cdk00, nut02 as cdk02};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
// ----- local imports

///--------------------------- generate blinded messages
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct GenerateBlindedMessagesRequest {
    pub kid: cdk02::Id,
    pub total: cashu::Amount,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct GenerateBlindedMessagesResponse {
    pub request_id: uuid::Uuid,
    pub messages: Vec<cdk00::BlindedMessage>,
}

///--------------------------- store blinded signatures
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct StoreBlindSignaturesRequest {
    pub rid: uuid::Uuid,
    pub signatures: Vec<cdk00::BlindSignature>,
    pub expiration: chrono::DateTime<chrono::Utc>,
}

/// --------------------------- request to mint from ebill description
#[derive(Debug, Serialize, Deserialize, BorshSerialize, BorshDeserialize)]
pub struct RequestToMintFromEBillDesc {
    pub ebill_id: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SignedRequestToMintFromEBillDesc {
    pub data: RequestToMintFromEBillDesc,
    pub signature: bitcoin::secp256k1::schnorr::Signature,
}

/// --------------------------- request to pay ebill
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct RequestToMintFromEBillRequest {
    pub ebill_id: String,
    pub amount: cashu::Amount,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct RequestToMintfromEBillResponse {
    pub request_id: String,
    pub request: String,
}
