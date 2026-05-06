// ----- standard library imports
//
// ----- extra library imports
use axum::http::StatusCode;
use bcr_common::{
    cashu,
    cdk_common::Error as CDKError,
    client::{admin::clowder::Error as ClowderRestError, treasury::Error as TreasuryClientError},
    clwdr_client::ClowderClientError,
};
use thiserror::Error;
// ----- local imports

// ----- end imports

pub type Result<T> = std::result::Result<T, Error>;

#[allow(dead_code)]
#[derive(Debug, Error)]
pub enum Error {
    #[error("DB: {0}")]
    DB(anyhow::Error),
    #[error("cashu::nut00:: {0}")]
    Cdk00(#[from] cashu::nut00::Error),
    #[error("borsh:: {0}")]
    Borsh(#[from] borsh::io::Error),
    #[error("bcr_common::borsh:: {0}")]
    BcrCommonBorsh(#[from] bcr_common::core::signature::BorshMsgSignatureError),
    #[error("CDK Client error: {0}")]
    Cdk(#[from] CDKError),
    #[error("Core Client: {0}")]
    Core(#[from] bcr_common::client::core::Error),
    #[error("Treasury Client error: {0}")]
    Treasury(#[from] TreasuryClientError),
    #[error("Clowder Client error: {0}")]
    ClowderClient(#[from] ClowderClientError),
    #[error("Clowder rest error: {0}")]
    ClowderRestClient(#[from] ClowderRestError),
    #[error("Clowder Client Not Initialized")]
    ClowderClientNoInit,

    #[error("invalid: {0}")]
    Invalid(String),
    #[error("not yet implemented: {0}")]
    NotYet(String),
    #[error("invalid input: {0}")]
    InvalidInput(String),
    #[error("invalid signature: {0}")]
    InvalidSignature(String),
    #[error("commitment not found")]
    CommitmentNotFound,
    #[error("commitment mismatch")]
    CommitmentMismatch,
}

impl axum::response::IntoResponse for Error {
    fn into_response(self) -> axum::response::Response {
        tracing::error!("Error: {}", self);
        let response = match self {
            Error::InvalidInput(e) => (StatusCode::BAD_REQUEST, e.to_string()),
            Error::NotYet(msg) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("{msg} not yet implemented"),
            ),
            Error::Invalid(msg) => (StatusCode::BAD_REQUEST, msg.to_string()),

            Error::Treasury(_) => (StatusCode::INTERNAL_SERVER_ERROR, String::new()),
            Error::ClowderClient(err) => (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()),
            Error::ClowderRestClient(err) => (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()),
            Error::ClowderClientNoInit => (StatusCode::INTERNAL_SERVER_ERROR, String::new()),

            Error::Core(bcr_common::client::core::Error::InvalidRequest(msg)) => {
                (StatusCode::BAD_REQUEST, msg)
            }
            Error::Core(bcr_common::client::core::Error::ResourceNotFound(_)) => {
                (StatusCode::NOT_FOUND, self.to_string())
            }
            Error::Core(_) => (StatusCode::INTERNAL_SERVER_ERROR, String::new()),

            Error::Cdk(_) => (StatusCode::INTERNAL_SERVER_ERROR, String::new()),
            Error::BcrCommonBorsh(_) => (StatusCode::BAD_REQUEST, String::new()),
            Error::InvalidSignature(msg) => (StatusCode::BAD_REQUEST, msg),
            Error::CommitmentNotFound => (StatusCode::BAD_REQUEST, self.to_string()),
            Error::CommitmentMismatch => (StatusCode::BAD_REQUEST, self.to_string()),
            Error::Borsh(_) => (StatusCode::INTERNAL_SERVER_ERROR, String::new()),
            Error::Cdk00(_) => (StatusCode::INTERNAL_SERVER_ERROR, String::new()),
            Error::DB(_) => (StatusCode::INTERNAL_SERVER_ERROR, String::new()),
        };

        response.into_response()
    }
}
