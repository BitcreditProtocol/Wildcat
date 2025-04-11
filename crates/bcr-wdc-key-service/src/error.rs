// ----- standard library imports
// ----- extra library imports
use anyhow::Error as AnyError;
use axum::http::StatusCode;
use cashu::nuts::nut02 as cdk02;
use thiserror::Error;
// ----- local imports

pub type Result<T> = std::result::Result<T, Error>;
#[derive(Debug, Error)]
pub enum Error {
    // external errors wrappers
    #[error("keys repository error {0}")]
    KeysRepository(AnyError),
    #[error("sign with keys {0}")]
    SignKeys(#[from] bcr_wdc_keys::SignWithKeysError),
    #[error("verify with keys {0}")]
    VerifyKeys(#[from] bcr_wdc_keys::VerifyWithKeysError),

    #[error("Unknown keyset {0}")]
    UnknownKeyset(cdk02::Id),
}

impl axum::response::IntoResponse for Error {
    fn into_response(self) -> axum::response::Response {
        log::error!("Error: {}", self);
        let resp = match self {
            Error::UnknownKeyset(_) => (StatusCode::NOT_FOUND, String::from("Unknown keyset")),

            Error::VerifyKeys(_) => (StatusCode::BAD_REQUEST, String::new()),
            Error::SignKeys(_) => (StatusCode::BAD_REQUEST, String::new()),
            Error::KeysRepository(_) => (StatusCode::INTERNAL_SERVER_ERROR, String::new()),
        };
        resp.into_response()
    }
}
