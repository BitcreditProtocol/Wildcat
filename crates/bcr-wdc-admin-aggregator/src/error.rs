// ----- standard library imports
// ----- extra library imports
use axum::http::StatusCode;
use bcr_common::{
    cashu,
    client::{
        core::Error as CoreClientError, ebill::Error as EbillClientError,
        quote::Error as QuotesClientError, treasury::Error as TreasuryClientError,
    },
};
use clwdr_client::ClowderClientError;
use thiserror::Error;
// ----- local imports

// ----- end imports

pub type Result<T> = std::result::Result<T, Error>;
#[derive(Debug, Error)]
pub enum Error {
    #[error("cdk00 {0}")]
    Cdk00(#[from] cashu::nut00::Error),
    #[error("CoreClient: {0}")]
    CoreClient(#[from] CoreClientError),
    #[error("TreasuryClient: {0}")]
    TreasuryClient(#[from] TreasuryClientError),
    #[error("ClowderClient: {0}")]
    ClowderClient(#[from] ClowderClientError),
    #[error("EbillClient: {0}")]
    EBillClient(#[from] EbillClientError),
    #[error("QuotesClient: {0}")]
    QuotesClient(#[from] QuotesClientError),

    #[error("resource not found: {0}")]
    ResourceNotFound(String),
    #[error("Internal server error: {0}")]
    Internal(String),
}

impl axum::response::IntoResponse for Error {
    fn into_response(self) -> axum::response::Response {
        tracing::error!("Error: {}", self);
        let resp = match self {
            Error::CoreClient(CoreClientError::KeysetIdNotFound(e)) => {
                (StatusCode::NOT_FOUND, e.to_string())
            }
            Error::CoreClient(CoreClientError::MintOpNotFound(e)) => {
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
