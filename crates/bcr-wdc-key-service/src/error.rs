// ----- standard library imports
// ----- extra library imports
use anyhow::Error as AnyError;
use axum::http::StatusCode;
use bcr_wdc_utils::keys as keys_utils;
use cashu::{nut01 as cdk01, nut02 as cdk02};
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
    #[error("signatures repository error {0}")]
    SignaturesRepository(AnyError),

    #[error("Unknown keyset {0}")]
    UnknownKeyset(cdk02::Id),
    #[error("Unknown keyset from id {0}")]
    UnknownKeysetFromId(uuid::Uuid),
    #[error("invalid mint request: {0}")]
    InvalidMintRequest(String),
    #[error("invalid generate request {0}")]
    InvalidGenerateRequest(uuid::Uuid),
    #[error("signature already exists {0}")]
    SignatureAlreadyExists(cdk01::PublicKey),

    #[error("internal error {0}")]
    Internal(String),
}

impl axum::response::IntoResponse for Error {
    fn into_response(self) -> axum::response::Response {
        tracing::error!("Error: {}", self);
        let resp = match self {
            Error::Internal(_) => (StatusCode::INTERNAL_SERVER_ERROR, String::new()),
            Error::SignatureAlreadyExists(y) => (
                StatusCode::CONFLICT,
                format!("Signature {y} already exists"),
            ),
            Error::InvalidGenerateRequest(_) => (
                StatusCode::BAD_REQUEST,
                String::from("Invalid generate request"),
            ),
            Error::InvalidMintRequest(msg) => (
                StatusCode::BAD_REQUEST,
                format!("Invalid mint request: {msg}"),
            ),
            Error::UnknownKeysetFromId(_) => (
                StatusCode::NOT_FOUND,
                String::from("Unknown keyset from id"),
            ),
            Error::UnknownKeyset(_) => (StatusCode::NOT_FOUND, String::from("Unknown keyset")),

            Error::SignaturesRepository(_) => (StatusCode::INTERNAL_SERVER_ERROR, String::new()),
            Error::VerifyKeys(_) => (StatusCode::BAD_REQUEST, String::new()),
            Error::SignKeys(_) => (StatusCode::BAD_REQUEST, String::new()),
            Error::KeysRepository(_) => (StatusCode::INTERNAL_SERVER_ERROR, String::new()),
        };
        resp.into_response()
    }
}
