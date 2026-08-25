// ----- standard library imports
//
// ----- extra library imports
use axum::http::StatusCode;
use bcr_common::{
    cashu,
    client::{
        admin::clowder::Error as ClowderClientError, core::Error as CoreClientError,
        ebill::Error as EbillClientError, quote::Error as QuotesClientError,
        treasury::Error as TreasuryClientError,
    },
};
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
    #[error("forbidden: {0}")]
    Forbidden(String),
}

impl axum::response::IntoResponse for Error {
    fn into_response(self) -> axum::response::Response {
        let message = self.to_string();
        let resp = match self {
            Error::CoreClient(CoreClientError::ResourceNotFound(e)) => {
                (StatusCode::NOT_FOUND, e.to_string())
            }
            Error::TreasuryClient(TreasuryClientError::ResourceNotFound(e)) => {
                (StatusCode::NOT_FOUND, e.to_string())
            }
            Error::EBillClient(EbillClientError::ResourceNotFound(e)) => {
                (StatusCode::NOT_FOUND, e.to_string())
            }
            Error::QuotesClient(QuotesClientError::ResourceNotFound(e)) => {
                (StatusCode::NOT_FOUND, e.to_string())
            }
            Error::ResourceNotFound(e) => {
                (StatusCode::NOT_FOUND, format!("resource not found: {e}"))
            }
            Error::Forbidden(e) => (StatusCode::FORBIDDEN, e),
            Error::QuotesClient(QuotesClientError::InvalidRequest(e)) => {
                (StatusCode::BAD_REQUEST, e.to_string())
            }
            _ => (StatusCode::INTERNAL_SERVER_ERROR, message.clone()),
        };
        if resp.0.is_server_error() {
            tracing::error!("Error: {message}");
        } else {
            tracing::debug!("Error: {message}");
        }
        resp.into_response()
    }
}

#[cfg(test)]
mod tests {
    use axum::response::IntoResponse;

    use super::*;

    #[test]
    fn ebill_resource_not_found_is_not_an_internal_error() {
        let response =
            Error::EBillClient(EbillClientError::ResourceNotFound("bill".into())).into_response();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[test]
    fn quote_invalid_request_is_not_an_internal_error() {
        let response = Error::QuotesClient(QuotesClientError::InvalidRequest(
            serde_json::Value::String(String::from("Credit authorization is required")),
        ))
        .into_response();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
}
