// ----- standard library imports
// ----- extra library imports
use axum::http::StatusCode;
use bcr_wdc_utils::signatures as signatures_utils;
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
    CDK13(#[from] CDK13Error),
    #[error("CDK Wallet error {0}")]
    CDKWallet(#[from] cdk::Error),
    #[error("DB error {0}")]
    DB(#[from] SurrealError),
    #[error("Secp256k1 error {0}")]
    Secp256k1(#[from] bitcoin::secp256k1::Error),
    #[error("Serde_json error {0}")]
    SerdeJson(#[from] serde_json::Error),
    #[error("schnorr borsh message {0}")]
    SchnorrBorshMsg(#[from] bcr_wdc_utils::keys::SchnorrBorshMsgError),
    #[error("keys client {0}")]
    KeyClient(#[from] bcr_wdc_key_client::Error),
    #[error("Swap client error {0}")]
    SwapClient(#[from] bcr_wdc_swap_client::Error),
    #[error("Quote client error {0}")]
    QuoteClient(#[from] bcr_wdc_quote_client::Error),
    // internal errors
    #[error("internal sat wallet has not enough funds: requested {0}, available {1}")]
    InsufficientFunds(cdk::Amount, cdk::Amount),

    #[error("invalid inputs {0}")]
    InvalidInput(signatures_utils::ChecksError),
    #[error("invalid outputs {0}")]
    InvalidOutput(signatures_utils::ChecksError),
    #[error("inactive keyset {0}")]
    InactiveKeyset(cdk02::Id),
    #[error("active keyset {0}")]
    ActiveKeyset(cdk02::Id),
    #[error("Unmatching amount: input {0} != output {1}")]
    UnmatchingAmount(cdk::Amount, cdk::Amount),
    #[error("Unknown keyset {0}")]
    UnknownKeyset(cdk02::Id),
    #[error("error in unblinding signatures {0}")]
    UnblindSignatures(String),
    #[error("request id not found {0}")]
    RequestIDNotFound(Uuid),
    #[error("ebill id not found {0}")]
    EBillNotFound(String),
}
impl axum::response::IntoResponse for Error {
    fn into_response(self) -> axum::response::Response {
        tracing::error!("Error: {}", self);
        let resp = match self {
            Error::EBillNotFound(id) => {
                (StatusCode::NOT_FOUND, format!("EBill ID not found: {id}"))
            }
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
            Error::ActiveKeyset(kid) => (StatusCode::BAD_REQUEST, format!("Active keyset {kid}")),
            Error::InactiveKeyset(kid) => {
                (StatusCode::BAD_REQUEST, format!("Inactive keyset {kid}"))
            }

            Error::InsufficientFunds(_, _) => (StatusCode::INTERNAL_SERVER_ERROR, String::new()),

            Error::InvalidOutput(e) => (StatusCode::BAD_REQUEST, format!("Invalid outputs: {e}")),
            Error::InvalidInput(e) => (StatusCode::BAD_REQUEST, format!("Invalid inputs: {e}")),
            Error::QuoteClient(_) => (StatusCode::INTERNAL_SERVER_ERROR, String::new()),
            Error::SwapClient(_) => (StatusCode::INTERNAL_SERVER_ERROR, String::new()),
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
