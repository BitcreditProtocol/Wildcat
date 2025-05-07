// ----- standard library imports
// ----- extra library imports
use cashu::nut00 as cdk00;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
// ----- local imports

// ----- end imports

///--------------------------- Burn tokens
#[derive(Serialize, Deserialize, ToSchema, Debug)]
pub struct BurnRequest {
    pub proofs: Vec<cdk00::Proof>,
}

#[derive(Serialize, Deserialize, ToSchema)]
pub struct BurnResponse {}

///--------------------------- Recover tokens
#[derive(Serialize, Deserialize, ToSchema, Debug)]
pub struct RecoverRequest {
    pub proofs: Vec<cdk00::Proof>,
}

#[derive(Serialize, Deserialize, ToSchema)]
pub struct RecoverResponse {}
