// ----- standard library imports
// ----- extra library imports
use axum::http::StatusCode;
use bcr_wdc_key_client::Error as KeyClientError;
use bcr_wdc_utils::signatures as signatures_utils;
use cashu::nuts::nut01 as cdk01;
use cashu::nuts::nut02 as cdk02;
use cashu::Amount;
use thiserror::Error;
// ----- local imports

pub type Result<T> = std::result::Result<T, Error>;
#[allow(dead_code)]
#[derive(Debug, Error)]
pub enum Error {
    // external errors wrappers
    #[error("Proof Repository error: {0}")]
    ProofRepository(anyhow::Error),
    #[error("Keyset Client error: {0}")]
    KeysClient(KeyClientError),
    #[error("DHKE error: {0}")]
    CdkDhke(#[from] cashu::dhke::Error),
    #[error("cdk::nut12 error: {0}")]
    CDKNUT12(#[from] cashu::nuts::nut12::Error),
    #[error("invalid inputs {0}")]
    InvalidInput(signatures_utils::ChecksError),
    #[error("invalid outputs {0}")]
    InvalidOutput(signatures_utils::ChecksError),

    #[error("Invalid proof")]
    InvalidProof(cashu::secret::Secret),
    #[error("Invalid blinded message")]
    InvalidBlindedMessage(cdk01::PublicKey),
    #[error("Already spent proofs")]
    ProofsAlreadySpent,
    #[error("Unknown keyset {0}")]
    UnknownKeyset(cdk02::Id),
    #[error("inactive keyset {0}")]
    InactiveKeyset(cdk02::Id),
    #[error("active keyset {0}")]
    ActiveKeyset(cdk02::Id),
    #[error("Unknown amount {1} for keyset {0}")]
    UnknownAmountForKeyset(cdk02::Id, Amount),
    #[error("Unmatching amount: input {0} != output {1}")]
    UnmatchingAmount(Amount, Amount),
}

impl axum::response::IntoResponse for Error {
    fn into_response(self) -> axum::response::Response {
        tracing::error!("Error: {}", self);
        let response = match self {
            Error::UnmatchingAmount(_, _) => {
                (StatusCode::BAD_REQUEST, String::from("Unmatching amount"))
            }
            Error::UnknownAmountForKeyset(_, _) => (
                StatusCode::NOT_FOUND,
                String::from("Unknown amount for keyset"),
            ),
            Error::ActiveKeyset(_) => (StatusCode::BAD_REQUEST, String::from("Active keyset")),
            Error::InactiveKeyset(_) => (StatusCode::BAD_REQUEST, String::from("Inactive keyset")),
            Error::UnknownKeyset(_) => (StatusCode::NOT_FOUND, String::from("Unknown keyset")),
            Error::ProofsAlreadySpent => (
                StatusCode::BAD_REQUEST,
                String::from("Proofs already spent"),
            ),
            Error::InvalidProof(_) => (StatusCode::BAD_REQUEST, String::from("Invalid proof")),
            Error::InvalidOutput(e) => (StatusCode::BAD_REQUEST, format!("Invalid outputs: {e}")),
            Error::InvalidInput(e) => (StatusCode::BAD_REQUEST, format!("Invalid inputs: {e}")),
            Error::InvalidBlindedMessage(_) => (
                StatusCode::BAD_REQUEST,
                String::from("Invalid blinded message"),
            ),

            Error::CDKNUT12(_) => (StatusCode::INTERNAL_SERVER_ERROR, String::new()),
            Error::CdkDhke(_) => (StatusCode::BAD_REQUEST, String::new()),
            Error::KeysClient(_) => (StatusCode::INTERNAL_SERVER_ERROR, String::new()),
            Error::ProofRepository(_) => (StatusCode::INTERNAL_SERVER_ERROR, String::new()),
        };

        response.into_response()
    }
}
