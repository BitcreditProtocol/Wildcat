// ----- standard library imports
// ----- extra library imports
use anyhow::Error as AnyError;
use axum::http::StatusCode;
use bcr_wdc_keys::Error as KeysError;
use cashu::nuts::nut02 as cdk02;
use thiserror::Error;
// ----- local imports

pub type Result<T> = std::result::Result<T, Error>;
#[derive(Debug, Error)]
pub enum Error {
    // external errors wrappers
    #[error("keys repository error {0}")]
    KeysRepository(AnyError),
    #[error("keys error {0}")]
    Keys(#[from] KeysError),

    #[error("Unknown keyset {0}")]
    UnknownKeyset(cdk02::Id),
}

impl axum::response::IntoResponse for Error {
    fn into_response(self) -> axum::response::Response {
        let resp = match self {
            Error::UnknownKeyset(_) => (StatusCode::NOT_FOUND, String::from("Unknown keyset")),

            Error::Keys(_) => (StatusCode::BAD_REQUEST, String::new()),
            Error::KeysRepository(_) => (StatusCode::INTERNAL_SERVER_ERROR, String::new()),
        };
        resp.into_response()
    }
}
