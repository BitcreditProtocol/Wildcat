// ----- standard library imports
// ----- extra library imports
use anyhow::Error as AnyError;
use axum::http::StatusCode;
use bcr_wdc_treasury_client::Error as WalletError;
use bcr_wdc_utils::keys::SignWithKeysError;
use cashu::nuts::nut02 as cdk02;
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
    #[error("Keys error {0}")]
    SignWithKeys(#[from] SignWithKeysError),
    #[error("Error in parsing datetime: {0}")]
    Chrono(#[from] chrono::ParseError),
    #[error("quotes repository error {0}")]
    QuotesRepository(AnyError),
    #[error("keys handler error {0}")]
    KeysHandler(bcr_common::client::keys::Error),
    #[error("wallet error {0}")]
    Wallet(WalletError),
    #[error("ebill client error {0}")]
    EbillClient(#[from] bcr_common::client::ebill::Error),

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

            Error::SignWithKeys(SignWithKeysError::NoKeyForAmount(amount)) => {
                (StatusCode::NOT_FOUND, format!("No key for amount {amount}"))
            }
            Error::SignWithKeys(e) => (StatusCode::BAD_REQUEST, format!("Signature error: {e}")),

            Error::Wallet(_) => (StatusCode::INTERNAL_SERVER_ERROR, String::new()),
            Error::KeysHandler(_) => (StatusCode::INTERNAL_SERVER_ERROR, String::new()),
            Error::EbillClient(_) => (StatusCode::INTERNAL_SERVER_ERROR, String::new()),
            Error::QuotesRepository(_) => (StatusCode::INTERNAL_SERVER_ERROR, String::new()),
            Error::BcrCommonBorsh(_) => (
                StatusCode::BAD_REQUEST,
                String::from("Invalid signature or public key"),
            ),
        };
        resp.into_response()
    }
}
