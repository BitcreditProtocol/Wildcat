use axum::http::StatusCode;
// ----- standard library imports
// ----- extra library imports
use thiserror::Error;
// ----- end imports

/// Generic result type
pub type Result<T> = std::result::Result<T, Error>;

/// Generic error type
#[derive(Debug, Error)]
pub enum Error {
    #[error("DB error: {0}")]
    DB(#[from] surrealdb::Error),
    #[error("EBPP Client error: {0}")]
    Ebpp(#[from] bcr_wdc_ebpp_client::Error),
    #[error("E-IOU Client error: {0}")]
    Eiou(#[from] bcr_wdc_eiou_client::Error),
    #[error("Treasury Client error: {0}")]
    Treasury(#[from] bcr_wdc_treasury_client::Error),
}

impl axum::response::IntoResponse for Error {
    fn into_response(self) -> axum::response::Response {
        tracing::error!("Error: {}", self);
        let resp = match self {
            Error::Treasury(_) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                String::from("Treasury client error"),
            ),
            Error::Eiou(_) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                String::from("E-IOU client error"),
            ),
            Error::Ebpp(_) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                String::from("EBPP client error"),
            ),
            Error::DB(_) => (StatusCode::INTERNAL_SERVER_ERROR, String::from("DB error")),
        };
        resp.into_response()
    }
}
