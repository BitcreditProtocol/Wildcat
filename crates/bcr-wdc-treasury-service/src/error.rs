// ----- standard library imports
// ----- extra library imports
use axum::http::StatusCode;
use bcr_wdc_utils::signatures as signatures_utils;
use cashu::{
    nut00::Error as CDK00Error, nut02 as cdk02, nut10::Error as CDK10Error,
    nut11::Error as CDK11Error, nut12::Error as CDK12Error, nut13::Error as CDK13Error,
    nut20::Error as CDK20Error,
};
use surrealdb::Error as SurrealError;
use thiserror::Error;
use uuid::Uuid;
// ----- local imports

pub type Result<T> = std::result::Result<T, Error>;
#[allow(dead_code)]
#[derive(Debug, Error)]
pub enum Error {
    // external errors wrappers
    #[error("bcr_common::signature::borsh {0}")]
    BcrBorshSignature(#[from] bcr_common::core::signature::BorshMsgSignatureError),
    #[error("borsh {0}")]
    Borsh(#[from] borsh::io::Error),

    #[error("unblind {0}")]
    Unblind(#[from] bcr_wdc_utils::signatures::UnblindError),

    #[error("cashu::nut20 {0}")]
    CDK20(#[from] CDK20Error),
    #[error("cashu::nut00 {0}")]
    CDK00(#[from] CDK00Error),
    #[error("cashu::nut10 {0}")]
    CDK10(#[from] CDK10Error),
    #[error("cashu::nut11 {0}")]
    CDK11(#[from] CDK11Error),
    #[error("cashu::nut12 {0}")]
    CDK12(#[from] CDK12Error),
    #[error("cashu::nut13 {0}")]
    CDK13(#[from] CDK13Error),
    #[error("CDK Wallet {0}")]
    CDKWallet(#[from] cdk::Error),
    #[error("CDK secret {0}")]
    CDKSecret(#[from] cdk::secret::Error),
    #[error("DB error {0}")]
    DB(#[from] SurrealError),
    #[error("Secp256k1 error {0}")]
    Secp256k1(#[from] bitcoin::secp256k1::Error),
    #[error("Serde_json error {0}")]
    SerdeJson(#[from] serde_json::Error),
    #[error("schnorr borsh message {0}")]
    SchnorrBorshMsg(#[from] bcr_wdc_utils::keys::SchnorrBorshMsgError),
    #[error("keys client {0}")]
    KeyClient(#[from] bcr_common::client::keys::Error),
    #[error("clowder client {0}")]
    ClowderClient(#[from] clwdr_client::ClowderClientError),
    #[error("Swap client {0}")]
    SwapClient(#[from] bcr_common::client::swap::Error),
    #[error("Quote client error {0}")]
    QuoteClient(#[from] bcr_common::client::quote::Error),
    // internal errors
    #[error("internal sat wallet has not enough funds: requested {0}, available {1}")]
    InsufficientFunds(cdk::Amount, cdk::Amount),

    #[error("invalid inputs {0}")]
    InvalidInput(String),
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

    #[error("internal {0}")]
    Internal(String),
}

impl axum::response::IntoResponse for Error {
    fn into_response(self) -> axum::response::Response {
        tracing::error!("Error: {}", self);
        let resp = match self {
            Error::Internal(_) => (StatusCode::INTERNAL_SERVER_ERROR, String::from("")),
            Error::EBillNotFound(id) => {
                (StatusCode::NOT_FOUND, format!("EBill ID not found: {id}"))
            }
            Error::KeyClient(_) => (StatusCode::INTERNAL_SERVER_ERROR, String::from("")),
            Error::UnblindSignatures(_) => (StatusCode::INTERNAL_SERVER_ERROR, String::from("")),
            Error::RequestIDNotFound(request_id) => (
                StatusCode::BAD_REQUEST,
                format!("Request ID not found: {request_id}"),
            ),
            Error::UnknownKeyset(keyset) => {
                (StatusCode::BAD_REQUEST, format!("Unknown keyset: {keyset}"))
            }
            Error::UnmatchingAmount(input, output) => (
                StatusCode::BAD_REQUEST,
                format!("Unmatching amount: input {input} != output {output}"),
            ),
            Error::ActiveKeyset(kid) => (StatusCode::BAD_REQUEST, format!("Active keyset {kid}")),
            Error::InactiveKeyset(kid) => {
                (StatusCode::BAD_REQUEST, format!("Inactive keyset {kid}"))
            }

            Error::InsufficientFunds(_, _) => (StatusCode::INTERNAL_SERVER_ERROR, String::new()),

            Error::InvalidOutput(e) => (StatusCode::BAD_REQUEST, format!("Invalid outputs: {e}")),
            Error::InvalidInput(e) => (StatusCode::BAD_REQUEST, format!("Invalid inputs: {e}")),
            Error::QuoteClient(_) => (StatusCode::INTERNAL_SERVER_ERROR, String::new()),
            Error::ClowderClient(_) => (StatusCode::INTERNAL_SERVER_ERROR, String::new()),
            Error::SwapClient(_) => (StatusCode::INTERNAL_SERVER_ERROR, String::new()),
            Error::SchnorrBorshMsg(_) => (StatusCode::INTERNAL_SERVER_ERROR, String::new()),
            Error::SerdeJson(_) => (StatusCode::INTERNAL_SERVER_ERROR, String::new()),
            Error::Secp256k1(_) => (StatusCode::INTERNAL_SERVER_ERROR, String::new()),
            Error::DB(_) => (StatusCode::INTERNAL_SERVER_ERROR, String::new()),
            Error::CDKSecret(_) => (StatusCode::BAD_REQUEST, String::new()),
            Error::CDKWallet(_) => (StatusCode::INTERNAL_SERVER_ERROR, String::new()),
            Error::CDK13(_) => (StatusCode::INTERNAL_SERVER_ERROR, String::new()),
            Error::CDK12(_) => (StatusCode::INTERNAL_SERVER_ERROR, String::new()),
            Error::CDK11(_) => (StatusCode::INTERNAL_SERVER_ERROR, String::new()),
            Error::CDK10(_) => (StatusCode::INTERNAL_SERVER_ERROR, String::new()),
            Error::CDK00(_) => (StatusCode::INTERNAL_SERVER_ERROR, String::new()),
            Error::CDK20(_) => (StatusCode::INTERNAL_SERVER_ERROR, String::new()),
            Error::Unblind(_) => (StatusCode::INTERNAL_SERVER_ERROR, String::new()),
            Error::Borsh(_) => (StatusCode::INTERNAL_SERVER_ERROR, String::new()),
            Error::BcrBorshSignature(_) => (StatusCode::INTERNAL_SERVER_ERROR, String::new()),
        };
        resp.into_response()
    }
}
