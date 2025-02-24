// ----- standard library imports
// ----- extra library imports
use anyhow::Error as AnyError;
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
    #[error("Quote repository error {0}")]
    QuoteRepository(#[from] AnyError),
    #[error("Borsh error {0}")]
    Borsh(#[from] borsh::io::Error),
    #[error("Secp256k1 error {0}")]
    Secp256k1(#[from] bitcoin::secp256k1::Error),
}

impl axum::response::IntoResponse for Error {
    fn into_response(self) -> axum::response::Response {
        (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            self.to_string(),
        )
            .into_response()
    }
}
