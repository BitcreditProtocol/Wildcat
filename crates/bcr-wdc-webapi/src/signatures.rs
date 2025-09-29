// ----- standard library imports
// ----- extra library imports
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
}
