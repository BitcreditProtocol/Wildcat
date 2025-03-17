// ----- standard library imports
// ----- extra library imports
use axum::http::StatusCode;
use cashu::nut13::Error as CDK13Error;
use surrealdb::Error as SurrealError;
use thiserror::Error;
// ----- local imports

pub type Result<T> = std::result::Result<T, Error>;
#[allow(dead_code)]
#[derive(Debug, Error)]
pub enum Error {
    // external errors wrappers
    #[error("cashu::nut13 error {0}")]
    CDK13(CDK13Error),
    #[error("DB error {0}")]
    DB(SurrealError),
}

impl axum::response::IntoResponse for Error {
    fn into_response(self) -> axum::response::Response {
        let resp = match self {
            Error::CDK13(_) => (StatusCode::INTERNAL_SERVER_ERROR, String::new()),
            Error::DB(_) => (StatusCode::INTERNAL_SERVER_ERROR, String::new()),
        };
        resp.into_response()
    }
}
