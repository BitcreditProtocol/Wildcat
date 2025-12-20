// ----- standard library imports
// ----- extra library imports
use anyhow::Error as AnyError;
use axum::http::StatusCode;
use bcr_common::client::{
    ebill::Error as EbillClientError, keys::Error as KeysClientError,
    quote::Error as QuotesClientError,
};
use clwdr_client::ClowderClientError;
use thiserror::Error;
// ----- local imports

// ----- end imports

pub type Result<T> = std::result::Result<T, Error>;
#[derive(Debug, Error)]
pub enum Error {
    #[error("ClowderClient: {0}")]
    ClowderClient(#[from] ClowderClientError),
    #[error("EbillClient: {0}")]
    EBillClient(#[from] EbillClientError),
    #[error("QuotesClient: {0}")]
    QuotesClient(#[from] QuotesClientError),
    #[error("KeysClient: {0}")]
    KeysClient(#[from] KeysClientError),

    #[error("resource not found: {0}")]
    ResourceNotFound(String),
    #[error("Internal server error: {0}")]
    Internal(#[from] AnyError),
}

impl axum::response::IntoResponse for Error {
    fn into_response(self) -> axum::response::Response {
        tracing::error!("Error: {}", self);
        let resp = match self {
            Error::KeysClient(KeysClientError::KeysetIdNotFound(e)) => {
                (StatusCode::NOT_FOUND, e.to_string())
            }
            Error::KeysClient(KeysClientError::MintOpNotFound(e)) => {
                (StatusCode::NOT_FOUND, e.to_string())
            }
            Error::QuotesClient(QuotesClientError::ResourceNotFound(e)) => {
                (StatusCode::NOT_FOUND, e.to_string())
            }
            Error::ResourceNotFound(e) => {
                (StatusCode::NOT_FOUND, format!("resource not found: {e}"))
            }
            _ => (StatusCode::INTERNAL_SERVER_ERROR, self.to_string()),
        };
        resp.into_response()
    }
}
