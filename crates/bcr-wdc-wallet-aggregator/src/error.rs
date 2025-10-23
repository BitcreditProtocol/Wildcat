// ----- standard library imports
// ----- extra library imports
use axum::http::StatusCode;
use bcr_wdc_ebpp_client::Error as EbppClientError;
use bcr_wdc_treasury_client::Error as TreasuryClientError;
use cdk::Error as CDKError;
use clwdr_client::ClowderClientError;
use thiserror::Error;
// ----- local imports

// ----- end imports

pub type Result<T> = std::result::Result<T, Error>;

#[allow(dead_code)]
#[derive(Debug, Error)]
pub enum Error {
    #[error("CDK Client error: {0}")]
    Cdk(#[from] CDKError),
    #[error("Keyset Client: {0}")]
    Keys(#[from] bcr_common::client::keys::Error),
    #[error("Swap Client: {0}")]
    Swap(#[from] bcr_common::client::swap::Error),
    #[error("Treasury Client error: {0}")]
    Treasury(#[from] TreasuryClientError),
    #[error("EBPP Client error: {0}")]
    Ebpp(#[from] EbppClientError),
    #[error("Clowder Client error: {0}")]
    ClowderClient(#[from] ClowderClientError),
    #[error("Clowder Client Not Initialized")]
    ClowderClientNoInit,

    #[error("not yet implemented: {0}")]
    NotYet(String),
}

impl axum::response::IntoResponse for Error {
    fn into_response(self) -> axum::response::Response {
        tracing::error!("Error: {}", self);
        let response = match self {
            Error::NotYet(msg) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("{msg} not yet implemented"),
            ),

            Error::Ebpp(_) => (StatusCode::INTERNAL_SERVER_ERROR, String::new()),
            Error::Treasury(_) => (StatusCode::INTERNAL_SERVER_ERROR, String::new()),
            Error::Swap(_) => (StatusCode::INTERNAL_SERVER_ERROR, String::new()),
            Error::ClowderClient(err) => (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()),
            Error::ClowderClientNoInit => (StatusCode::INTERNAL_SERVER_ERROR, String::new()),

            Error::Keys(bcr_common::client::keys::Error::InvalidRequest) => {
                (StatusCode::BAD_REQUEST, String::new())
            }
            Error::Keys(bcr_common::client::keys::Error::ResourceNotFound(kid)) => {
                (StatusCode::NOT_FOUND, format!("keyset Id {kid} not found"))
            }
            Error::Keys(_) => (StatusCode::INTERNAL_SERVER_ERROR, String::new()),

            Error::Cdk(_) => (StatusCode::INTERNAL_SERVER_ERROR, String::new()),
        };

        response.into_response()
    }
}
