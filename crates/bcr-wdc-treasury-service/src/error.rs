// ----- standard library imports
// ----- extra library imports
use axum::http::StatusCode;
use cashu::nut13::Error as CDK13Error;
use cdk::Error as CDKError;
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
    #[error("CDK Wallet error {0}")]
    CDKWallet(CDKError),
    #[error("DB error {0}")]
    DB(SurrealError),
    #[error("Borsh io error {0}")]
    BorshIO(borsh::io::Error),
    #[error("Secp256k1 error {0}")]
    Secp256k1(bitcoin::secp256k1::Error),
    #[error("Serde_json error {0}")]
    SerdeJson(serde_json::Error),
}

impl axum::response::IntoResponse for Error {
    fn into_response(self) -> axum::response::Response {
        log::error!("Error: {}", self);
        let resp = match self {
            Error::CDK13(_) => (StatusCode::INTERNAL_SERVER_ERROR, String::new()),
            Error::CDKWallet(_) => (StatusCode::INTERNAL_SERVER_ERROR, String::new()),
            Error::DB(_) => (StatusCode::INTERNAL_SERVER_ERROR, String::new()),
            Error::BorshIO(_) => (StatusCode::INTERNAL_SERVER_ERROR, String::new()),
            Error::Secp256k1(_) => (StatusCode::INTERNAL_SERVER_ERROR, String::new()),
            Error::SerdeJson(_) => (StatusCode::INTERNAL_SERVER_ERROR, String::new()),
        };
        resp.into_response()
    }
}
