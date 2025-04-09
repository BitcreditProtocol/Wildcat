// ----- standard library imports
// ----- extra library imports
use anyhow::Error as AnyError;
use axum::http::StatusCode;
use bcr_wdc_key_client::Error as KeysHandlerError;
use bcr_wdc_keys::SignWithKeysError;
use bcr_wdc_treasury_client::Error as WalletError;
use cashu::nuts::nut02 as cdk02;
use thiserror::Error;
// ----- local modules
// ----- local imports

pub type Result<T> = std::result::Result<T, Error>;
#[derive(Debug, Error)]
pub enum Error {
    // external errors wrappers
    #[error("Secp256k1 {0}")]
    Secp256k1(#[from] bitcoin::secp256k1::Error),
    #[error("schnorr borsh message {0}")]
    SchnorrBorshMsg(#[from] bcr_wdc_keys::SchnorrBorshMsgError),
    #[error("Keys error {0}")]
    SignWithKeys(#[from] SignWithKeysError),
    #[error("Error in parsing datetime: {0}")]
    Chrono(#[from] chrono::ParseError),
    #[error("quotes repository error {0}")]
    QuotesRepository(AnyError),
    #[error("keys handler error {0}")]
    KeysHandler(KeysHandlerError),
    #[error("wallet error {0}")]
    Wallet(WalletError),

    #[error("Quote has been already resolved: {0}")]
    QuoteAlreadyResolved(uuid::Uuid),
    #[error("unknown quote id {0}")]
    UnknownQuoteID(uuid::Uuid),
    #[error("Invalid amount: {0}")]
    InvalidAmount(rust_decimal::Decimal),
    #[error("Invalid blindedMessages: {0}")]
    InvalidKeysetId(cdk02::Id),
}

impl axum::response::IntoResponse for Error {
    fn into_response(self) -> axum::response::Response {
        let resp = match self {
            Error::InvalidKeysetId(_) => {
                (StatusCode::BAD_REQUEST, String::from("Invalid Kyset ID"))
            }
            Error::InvalidAmount(_) => (StatusCode::BAD_REQUEST, String::from("Invalid amount")),
            Error::UnknownQuoteID(_) => (StatusCode::NOT_FOUND, String::from("Quote ID not found")),
            Error::QuoteAlreadyResolved(_) => (
                StatusCode::CONFLICT,
                String::from("Quote has been already resolved"),
            ),

            Error::Chrono(_) => (StatusCode::BAD_REQUEST, String::from("Malformed datetime")),

            Error::SignWithKeys(SignWithKeysError::NoKeyForAmount(amount)) => (
                StatusCode::NOT_FOUND,
                format!("No key for amount {}", amount),
            ),
            Error::SignWithKeys(_) => (StatusCode::BAD_REQUEST, String::new()),

            Error::Wallet(_) => (StatusCode::INTERNAL_SERVER_ERROR, String::new()),
            Error::KeysHandler(_) => (StatusCode::INTERNAL_SERVER_ERROR, String::new()),
            Error::QuotesRepository(_) => (StatusCode::INTERNAL_SERVER_ERROR, String::new()),
            Error::SchnorrBorshMsg(_) => (StatusCode::INTERNAL_SERVER_ERROR, String::new()),
            Error::Secp256k1(_) => (
                StatusCode::BAD_REQUEST,
                String::from("Invalid signature or public key"),
            ),
        };
        resp.into_response()
    }
}
