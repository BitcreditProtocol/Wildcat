// ----- standard library imports
// ----- extra library imports
use axum::http::StatusCode;
use bcr_wdc_key_client::Error as KeyClientError;
use cdk::Error as CDKError;
use thiserror::Error;
// ----- local imports

pub type Result<T> = std::result::Result<T, Error>;

#[allow(dead_code)]
#[derive(Debug, Error)]
pub enum Error {
    #[error("Keyset Client error: {0}")]
    KeysClient(KeyClientError),

    #[error("CDK Client error: {0}")]
    CDKClient(CDKError),
}

impl axum::response::IntoResponse for Error {
    fn into_response(self) -> axum::response::Response {
        let response = match self {
            Error::KeysClient(_) => (StatusCode::INTERNAL_SERVER_ERROR, String::new()),
            Error::CDKClient(_) => (StatusCode::INTERNAL_SERVER_ERROR, String::new()),
        };

        response.into_response()
    }
}
