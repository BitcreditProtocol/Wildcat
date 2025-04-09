// ----- standard library imports
// ----- extra library imports
use cashu::nut00 as cdk00;
use cashu::nut02 as cdk02;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
// ----- local imports

///--------------------------- Pre-sign blinded message
#[derive(Serialize, Deserialize, ToSchema)]
pub struct PreSignRequest {
    pub kid: cdk02::Id,
    pub qid: uuid::Uuid,
    pub expire: chrono::DateTime<chrono::Utc>,
    pub msg: cdk00::BlindedMessage,
}

///--------------------------- Activate keyset
#[derive(Serialize, Deserialize, ToSchema)]
pub struct ActivateKeysetRequest {
    pub kid: cdk02::Id,
    pub qid: uuid::Uuid,
}
