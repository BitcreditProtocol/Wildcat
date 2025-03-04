// ----- standard library imports
// ----- extra library imports
use axum::http::StatusCode;
use cashu::Amount;
use cashu::nuts::nut02 as cdk02;
use thiserror::Error;
// ----- local imports

pub type Result<T> = std::result::Result<T, Error>;
#[allow(dead_code)]
#[derive(Debug, Error)]
pub enum Error {
    // external errors wrappers
    #[error("Proof Repository error: {0}")]
    ProofRepository(#[from] anyhow::Error),
    #[error("Keyset Client error: {0}")]
    KeysetClient(anyhow::Error),
    #[error("DHKE error: {0}")]
    CdkDhke(#[from] cashu::dhke::Error),
    #[error("cdk::nut12 error: {0}")]
    CDKNUT12(#[from] cashu::nuts::nut12::Error),

    #[error("Already spent proofs")]
    ProofsAlreadySpent,
    #[error("Unknown keyset {0}")]
    UnknownKeyset(cdk02::Id),
    #[error("inactive keyset {0}")]
    InactiveKeyset(cdk02::Id),
    #[error("Unknown amount {1} for keyset {0}")]
    UnknownAmountForKeyset(cdk02::Id, Amount),
    #[error("Empty inputs/outputs")]
    EmptyInputsOrOutputs,
    #[error("Zero amount is not allowed")]
    ZeroAmount,
    #[error("Unmatching amount: input {0} != output {1}")]
    UnmatchingAmount(Amount, Amount),
}

impl axum::response::IntoResponse for Error {
    fn into_response(self) -> axum::response::Response {
        let response = match self {
            Error::UnmatchingAmount(_, _) => {
                (StatusCode::BAD_REQUEST, String::from("Unmatching amount"))
            }
            Error::EmptyInputsOrOutputs => (
                StatusCode::BAD_REQUEST,
                String::from("Empty inputs or outputs vectors is not allowed"),
            ),
            Error::ZeroAmount => (
                StatusCode::BAD_REQUEST,
                String::from("Zero amount is not allowed"),
            ),
            Error::UnknownAmountForKeyset(_, _) => (
                StatusCode::NOT_FOUND,
                String::from("Unknown amount for keyset"),
            ),
            Error::InactiveKeyset(_) => (StatusCode::GONE, String::from("Inactive keyset")),
            Error::UnknownKeyset(_) => (StatusCode::NOT_FOUND, String::from("Unknown keyset")),
            Error::ProofsAlreadySpent => {
                (StatusCode::FORBIDDEN, String::from("Proofs already spent"))
            }

            Error::CDKNUT12(_) => (StatusCode::INTERNAL_SERVER_ERROR, String::new()),
            Error::CdkDhke(_) => (StatusCode::INTERNAL_SERVER_ERROR, String::new()),
            Error::KeysetClient(_) => (StatusCode::INTERNAL_SERVER_ERROR, String::new()),
            Error::ProofRepository(_) => (StatusCode::INTERNAL_SERVER_ERROR, String::new()),
        };

        response.into_response()
    }
}
