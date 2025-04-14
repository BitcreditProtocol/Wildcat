// ----- standard library imports
// ----- extra library imports
use axum::http::StatusCode;
use cashu::{nut02 as cdk02, nut13::Error as CDK13Error};
use surrealdb::Error as SurrealError;
use thiserror::Error;
use uuid::Uuid;
// ----- local imports

pub type Result<T> = std::result::Result<T, Error>;
#[allow(dead_code)]
#[derive(Debug, Error)]
pub enum Error {
    // external errors wrappers
    #[error("cashu::nut13 error {0}")]
    CDK13(CDK13Error),
    #[error("CDK Wallet error {0}")]
    CDKWallet(cdk::Error),
    #[error("DB error {0}")]
    DB(SurrealError),
    #[error("Secp256k1 error {0}")]
    Secp256k1(bitcoin::secp256k1::Error),
    #[error("Serde_json error {0}")]
    SerdeJson(serde_json::Error),
    #[error("schnorr borsh message {0}")]
    SchnorrBorshMsg(bcr_wdc_keys::SchnorrBorshMsgError),
    #[error("Proof client error {0}")]
    ProofCl(bcr_wdc_swap_client::Error),
    //debit errors
    #[error("Empty inputs/outputs")]
    EmptyInputsOrOutputs,
    #[error("Zero amount is not allowed")]
    ZeroAmount,
    #[error("inactive keyset {0}")]
    InactiveKeyset(cdk02::Id),
    #[error("Unmatching amount: input {0} != output {1}")]
    UnmatchingAmount(cdk::Amount, cdk::Amount),
    #[error("Unknown keyset {0}")]
    UnknownKeyset(cdk02::Id),
    // credit errors
    #[error("error in unblinding signatures {0}")]
    UnblindSignatures(String),
    #[error("request id not found {0}")]
    RequestIDNotFound(Uuid),
    #[error("keys client {0}")]
    KeyClient(bcr_wdc_key_client::Error),
}
impl axum::response::IntoResponse for Error {
    fn into_response(self) -> axum::response::Response {
        log::error!("Error: {}", self);
        let resp = match self {
            Error::KeyClient(_) => (StatusCode::INTERNAL_SERVER_ERROR, String::from("")),
            Error::UnblindSignatures(_) => (StatusCode::INTERNAL_SERVER_ERROR, String::from("")),
            Error::RequestIDNotFound(request_id) => (
                StatusCode::BAD_REQUEST,
                format!("Request ID not found: {}", request_id),
            ),
            Error::UnknownKeyset(keyset) => (
                StatusCode::BAD_REQUEST,
                format!("Unknown keyset: {}", keyset),
            ),
            Error::UnmatchingAmount(input, output) => (
                StatusCode::BAD_REQUEST,
                format!("Unmatching amount: input {} != output {}", input, output),
            ),
            Error::InactiveKeyset(_) => (StatusCode::BAD_REQUEST, String::from("Inactive keyset")),
            Error::ZeroAmount => (
                StatusCode::BAD_REQUEST,
                String::from("Zero amount is not allowed"),
            ),
            Error::EmptyInputsOrOutputs => (
                StatusCode::BAD_REQUEST,
                String::from("Empty inputs/outputs"),
            ),

            Error::ProofCl(_) => (StatusCode::INTERNAL_SERVER_ERROR, String::new()),
            Error::SchnorrBorshMsg(_) => (StatusCode::INTERNAL_SERVER_ERROR, String::new()),
            Error::SerdeJson(_) => (StatusCode::INTERNAL_SERVER_ERROR, String::new()),
            Error::Secp256k1(_) => (StatusCode::INTERNAL_SERVER_ERROR, String::new()),
            Error::DB(_) => (StatusCode::INTERNAL_SERVER_ERROR, String::new()),
            Error::CDKWallet(_) => (StatusCode::INTERNAL_SERVER_ERROR, String::new()),
            Error::CDK13(_) => (StatusCode::INTERNAL_SERVER_ERROR, String::new()),
        };
        resp.into_response()
    }
}
