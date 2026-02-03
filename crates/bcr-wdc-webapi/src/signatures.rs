// ----- standard library imports
// ----- extra library imports
use bcr_common::cashu;
use serde::{Deserialize, Serialize};
// ----- local imports

// ----- end imports

///--------------------------- generate blinded messages
#[derive(Debug, Serialize, Deserialize)]
pub struct GenerateBlindedMessagesRequest {
    pub kid: cashu::Id,
    pub total: cashu::Amount,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct GenerateBlindedMessagesResponse {
    pub request_id: uuid::Uuid,
    pub messages: Vec<cashu::BlindedMessage>,
}

///--------------------------- store blinded signatures
#[derive(Debug, Serialize, Deserialize)]
pub struct StoreBlindSignaturesRequest {
    pub rid: uuid::Uuid,
    pub signatures: Vec<cashu::BlindSignature>,
}
