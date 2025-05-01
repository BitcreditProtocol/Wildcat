// ----- standard library imports
// ----- extra library imports
use anyhow::Error as AnyError;
use axum::http::StatusCode;
use bcr_wdc_utils::keys as keys_utils;
use cashu::nut02 as cdk02;
use thiserror::Error;
// ----- local imports

pub type Result<T> = std::result::Result<T, Error>;
#[derive(Debug, Error)]
pub enum Error {
    // external errors wrappers
    #[error("keys repository error {0}")]
    KeysRepository(AnyError),
    #[error("sign with keys {0}")]
    SignKeys(#[from] keys_utils::SignWithKeysError),
    #[error("verify with keys {0}")]
    VerifyKeys(#[from] keys_utils::VerifyWithKeysError),

    #[error("Unknown keyset {0}")]
    UnknownKeyset(cdk02::Id),
    #[error("Unknown keyset from id {0}")]
    UnknownKeysetFromId(uuid::Uuid),
    #[error("invalid mint request")]
    InvalidMintRequest,
    #[error("invalid generate request {0}")]
    InvalidGenerateRequest(uuid::Uuid),
}

impl axum::response::IntoResponse for Error {
    fn into_response(self) -> axum::response::Response {
        log::error!("Error: {}", self);
        let resp = match self {
            Error::InvalidGenerateRequest(_) => (
                StatusCode::BAD_REQUEST,
                String::from("Invalid generate request"),
            ),
            Error::InvalidMintRequest => (
                StatusCode::BAD_REQUEST,
                String::from("Invalid mint request"),
            ),
            Error::UnknownKeysetFromId(_) => (
                StatusCode::NOT_FOUND,
                String::from("Unknown keyset from id"),
            ),
            Error::UnknownKeyset(_) => (StatusCode::NOT_FOUND, String::from("Unknown keyset")),

            Error::VerifyKeys(_) => (StatusCode::BAD_REQUEST, String::new()),
            Error::SignKeys(_) => (StatusCode::BAD_REQUEST, String::new()),
            Error::KeysRepository(_) => (StatusCode::INTERNAL_SERVER_ERROR, String::new()),
        };
        resp.into_response()
    }
}
