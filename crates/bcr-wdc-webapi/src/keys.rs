// ----- standard library imports
// ----- extra library imports
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
// ----- local imports

///--------------------------- Generate keyset
#[derive(Serialize, Deserialize, ToSchema, Debug)]
pub struct EnableNewMintingOpRequest {
    pub kid: cashu::Id,
    pub condition: KeysetMintCondition,
    pub expire: chrono::DateTime<chrono::Utc>,
}

#[derive(Serialize, Deserialize, ToSchema, Debug)]
pub struct KeysetMintCondition {
    pub amount: cashu::Amount,
    #[schema(value_type=String)]
    pub public_key: cashu::PublicKey,
}
///--------------------------- Pre-sign blinded message
#[derive(Serialize, Deserialize, ToSchema, Debug)]
pub struct SignRequest {
    pub kid: cashu::Id,
    pub msg: cashu::BlindedMessage,
}

///--------------------------- Deactivate keyset
#[derive(Serialize, Deserialize, ToSchema, Debug)]
pub struct DeactivateKeysetRequest {
    pub kid: cashu::Id,
}

#[derive(Serialize, Deserialize, ToSchema, Debug)]
pub struct DeactivateKeysetResponse {
    pub kid: cashu::Id,
}
