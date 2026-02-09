// ----- standard library imports
// ----- extra library imports
use anyhow::Error as AnyError;
use axum::http::StatusCode;
use bcr_common::cashu::{nut01 as cdk01, nut02 as cdk02};
use thiserror::Error;
// ----- local imports

pub type Result<T> = std::result::Result<T, Error>;
#[derive(Debug, Error)]
pub enum Error {
    // external errors wrappers
    #[error("keys repository {0}")]
    KeysRepository(AnyError),
    #[error("eCash sign/verify: {0}")]
    SignVerifyEcash(#[from] bcr_common::core::signature::ECashSignatureError),
    #[error("signatures repository {0}")]
    SignaturesRepository(AnyError),
    #[error("clowder client {0}")]
    ClowderClient(AnyError),

    #[error("MintOp not found {0}")]
    MintOpNotFound(uuid::Uuid),
    #[error("Unknown keyset {0}")]
    KeysetNotFound(cdk02::Id),
    #[error("Unknown keyset from id {0}")]
    UnknownKeysetFromId(uuid::Uuid),
    #[error("invalid mint request: {0}")]
    InvalidMintRequest(String),
    #[error("invalid generate request {0}")]
    InvalidGenerateRequest(uuid::Uuid),
    #[error("signature already exists {0}")]
    SignatureAlreadyExists(cdk01::PublicKey),
    #[error("mint operation already exists {0}")]
    MintOpAlreadyExist(uuid::Uuid),

    #[error("internal {0}")]
    Internal(String),
}

impl axum::response::IntoResponse for Error {
    fn into_response(self) -> axum::response::Response {
        tracing::error!("Error: {}", self);
        let resp = match self {
            Error::Internal(_) => (StatusCode::INTERNAL_SERVER_ERROR, String::new()),
            Error::MintOpAlreadyExist(_) => (StatusCode::CONFLICT, self.to_string()),
            Error::SignatureAlreadyExists(_) => (StatusCode::CONFLICT, self.to_string()),
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
            Error::KeysetNotFound(_) => (StatusCode::NOT_FOUND, self.to_string()),
            Error::MintOpNotFound(_) => (StatusCode::NOT_FOUND, self.to_string()),

            Error::ClowderClient(_) => (StatusCode::INTERNAL_SERVER_ERROR, String::new()),
            Error::SignaturesRepository(_) => (StatusCode::INTERNAL_SERVER_ERROR, String::new()),
            Error::SignVerifyEcash(_) => (StatusCode::BAD_REQUEST, String::new()),
            Error::KeysRepository(_) => (StatusCode::INTERNAL_SERVER_ERROR, String::new()),
        };
        resp.into_response()
    }
}
