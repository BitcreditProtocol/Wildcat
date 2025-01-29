// ----- standard library imports
// ----- extra library imports
use thiserror::Error;
// ----- local modules
// ----- local imports
use super::quotes;
use crate::credit::keys::Error as CreditKeysError;
use crate::keys::Error as KeysError;

pub type Result<T> = std::result::Result<T, Error>;
#[derive(Debug, Error)]
pub enum Error {
    #[error("Quote error {0}")]
    Quote(#[from] quotes::Error),
    #[error("Key error {0}")]
    CreditKeys(#[from] CreditKeysError),
    #[error("Keys error {0}")]
    Keys(#[from] KeysError),

    #[error("unknown quote id {0}")]
    UnknownQuoteID(uuid::Uuid),
    #[error("Quote has been already resolved: {0}")]
    QuoteAlreadyResolved(uuid::Uuid),

    #[error("Invalid amount: {0}")]
    InvalidAmount(rust_decimal::Decimal),
}
impl axum::response::IntoResponse for Error {
    fn into_response(self) -> axum::response::Response {
        self.to_string().into_response()
    }
}
