// ----- standard library imports
// ----- extra library imports
use cashu::nut00 as cdk00;
use cashu::nut02 as cdk02;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
// ----- local imports

///--------------------------- generate blinded messages
#[derive(Serialize, Deserialize, ToSchema)]
pub struct GenerateBlindedMessagesRequest {
    pub kid: cdk02::Id,
    pub total: cashu::Amount,
}

#[derive(Serialize, Deserialize, ToSchema)]
pub struct GenerateBlindedMessagesResponse {
    pub rid: uuid::Uuid,
    pub messages: Vec<cdk00::BlindedMessage>,
}

///--------------------------- store blinded signatures
#[derive(Serialize, Deserialize, ToSchema)]
pub struct StoreBlindedSignaturesRequest {
    pub rid: uuid::Uuid,
    pub signatures: Vec<cdk00::BlindSignature>,
    pub expiration: chrono::DateTime<chrono::Utc>,
}
