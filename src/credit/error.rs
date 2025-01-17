// ----- standard library imports
// ----- extra library imports
use thiserror::Error;
// ----- local modules
// ----- local imports
use super::{keys, quotes};

pub type Result<T> = std::result::Result<T, Error>;
#[derive(Debug, Error)]
pub enum Error {
    #[error("Quote error {0}")]
    Quote(#[from] quotes::Error),
    #[error("Key error {0}")]
    Key(#[from] keys::Error),

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
