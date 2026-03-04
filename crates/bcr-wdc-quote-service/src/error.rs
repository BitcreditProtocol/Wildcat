// ----- standard library imports
// ----- extra library imports
use anyhow::Error as AnyError;
use axum::http::StatusCode;
use bcr_common::{cashu::nut02 as cdk02, core::signature::ECashSignatureError};
use thiserror::Error;
// ----- local modules
// ----- local imports

pub type Result<T> = std::result::Result<T, Error>;
#[derive(Debug, Error)]
pub enum Error {
    // external errors wrappers
    #[error("bcr_common::borsh {0}")]
    BcrCommonBorsh(#[from] bcr_common::core::signature::BorshMsgSignatureError),
    #[error("convert {0}")]
    Convert(#[from] bcr_wdc_utils::convert::Error),
    #[error("eCash sign/verify error {0}")]
    SignWithKeys(#[from] ECashSignatureError),
    #[error("Error in parsing datetime: {0}")]
    Chrono(#[from] chrono::ParseError),
    #[error("quotes repository error {0}")]
    QuotesRepository(AnyError),
    #[error("core client error {0}")]
    CoreClient(#[from] bcr_common::client::core::Error),
    #[error("ebill client error {0}")]
    EbillClient(#[from] bcr_common::client::ebill::Error),
    #[error("treasury client error {0}")]
    TreasuryClient(#[from] bcr_common::client::treasury::Error),

    #[error("quote {0} incorrect status, expected {1}, found {2}")]
    InvalidQuoteStatus(
        uuid::Uuid,
        crate::quotes::StatusDiscriminants,
        crate::quotes::StatusDiscriminants,
    ),
    #[error("unknown quote id {0}")]
    QuoteIDNotFound(uuid::Uuid),
    #[error("Invalid amount: {0}")]
    InvalidAmount(bitcoin::Amount),
    #[error("Invalid input: {0}")]
    InvalidInput(String),
    #[error("Invalid keyset ID: {0}")]
    InvalidKeysetId(cdk02::Id),
    #[error("Internal server error: {0}")]
    InternalServer(String),
}

impl axum::response::IntoResponse for Error {
    fn into_response(self) -> axum::response::Response {
        tracing::error!("Error: {}", self);
        let resp = match self {
            Error::Convert(msg) => (StatusCode::INTERNAL_SERVER_ERROR, msg.to_string()),
            Error::InternalServer(msg) => (StatusCode::INTERNAL_SERVER_ERROR, msg),
            Error::InvalidKeysetId(_) => {
                (StatusCode::BAD_REQUEST, String::from("Invalid Kyset ID"))
            }

            Error::InvalidInput(_) => (StatusCode::BAD_REQUEST, String::from("Invalid input")),
            Error::InvalidAmount(_) => (StatusCode::BAD_REQUEST, String::from("Invalid amount")),
            Error::QuoteIDNotFound(_) => (StatusCode::NOT_FOUND, self.to_string()),
            Error::InvalidQuoteStatus(_, _, _) => {
                (StatusCode::CONFLICT, String::from("Quote invalid status"))
            }

            Error::Chrono(_) => (StatusCode::BAD_REQUEST, String::from("Malformed datetime")),

            Error::SignWithKeys(ECashSignatureError::NoKeyForAmount(amount)) => {
                (StatusCode::NOT_FOUND, format!("No key for amount {amount}"))
            }
            Error::SignWithKeys(e) => (StatusCode::BAD_REQUEST, format!("Signature error: {e}")),

            Error::CoreClient(_) => (StatusCode::INTERNAL_SERVER_ERROR, String::new()),
            Error::EbillClient(_) => (StatusCode::INTERNAL_SERVER_ERROR, String::new()),
            Error::TreasuryClient(_) => (StatusCode::INTERNAL_SERVER_ERROR, String::new()),
            Error::QuotesRepository(_) => (StatusCode::INTERNAL_SERVER_ERROR, String::new()),
            Error::BcrCommonBorsh(_) => (
                StatusCode::BAD_REQUEST,
                String::from("Invalid signature or public key"),
            ),
        };
        resp.into_response()
    }
}
