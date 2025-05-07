// ----- standard library imports
// ----- extra library imports
use cashu::{nut00 as cdk00, nut01 as cdk01, Amount};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
// ----- local imports

///--------------------------- Generate keyset
#[derive(Serialize, Deserialize, ToSchema, Debug)]
pub struct GenerateKeysetRequest {
    pub qid: uuid::Uuid,
    pub condition: KeysetMintCondition,
    pub expire: chrono::DateTime<chrono::Utc>,
}

#[derive(Serialize, Deserialize, ToSchema, Debug)]
pub struct KeysetMintCondition {
    pub amount: Amount,
    #[schema(value_type=String)]
    pub public_key: cdk01::PublicKey,
}
///--------------------------- Pre-sign blinded message
#[derive(Serialize, Deserialize, ToSchema, Debug)]
pub struct PreSignRequest {
    pub qid: uuid::Uuid,
    pub msg: cdk00::BlindedMessage,
}

///--------------------------- Activate keyset
#[derive(Serialize, Deserialize, ToSchema, Debug)]
pub struct ActivateKeysetRequest {
    pub qid: uuid::Uuid,
}
