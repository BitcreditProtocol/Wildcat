// ----- standard library imports
// ----- extra library imports
use axum::http::StatusCode;
use bcr_common::{
    cashu::{self, nut00 as cdk00, nut02 as cdk02, nut12 as cdk12},
    core::signature,
};
use bcr_wdc_utils::signatures as signatures_utils;
use thiserror::Error;
// ----- local imports

pub type Result<T> = std::result::Result<T, Error>;

#[allow(dead_code)]
#[derive(Debug, Error)]
pub enum Error {
    //db errors
    #[error("keys repository {0}")]
    KeysRepository(anyhow::Error),
    #[error("signatures repository {0}")]
    SignaturesRepository(anyhow::Error),
    #[error("Proof Repository error: {0}")]
    ProofRepository(anyhow::Error),
    #[error("Commitment Repository error: {0}")]
    CommitmentRepository(anyhow::Error),
    // external errors
    #[error("eCash sign/verify: {0}")]
    SignVerifyEcash(#[from] bcr_common::core::signature::ECashSignatureError),
    #[error("borsh signed verification: {0}")]
    BorshVerify(#[from] signature::BorshMsgSignatureError),
    #[error("clowder client {0}")]
    ClowderClient(#[from] bcr_common::clwdr_client::ClowderClientError),
    #[error("DHKE error: {0}")]
    CdkDhke(#[from] cashu::dhke::Error),
    #[error("cdk::nut00 error: {0}")]
    CDKNUT00(#[from] cdk00::Error),
    #[error("cdk::nut12 error: {0}")]
    CDKNUT12(#[from] cdk12::Error),
    #[error("checks {0}")]
    BasicChecks(#[from] signatures_utils::ChecksError),
    #[error("Verification: {0}")]
    Verify(#[from] bcr_common::core::swap::mint::VerificationError),
    #[error("Attestation: {0}")]
    Attestation(#[from] bcr_common::wire::attestation::AttestationError),
    // domain errors
    #[error("invalid inputs {0}")]
    InvalidInput(String),
    #[error("resource not found {0}")]
    ResourceNotFound(String),
    #[error("conflict {0}")]
    Conflict(String),

    #[error("inactive keyset {0}")]
    InactiveKeyset(cdk02::Id),
    #[error("active keyset {0}")]
    ActiveKeyset(cdk02::Id),

    #[error("internal {0}")]
    Internal(String),
}

impl axum::response::IntoResponse for Error {
    fn into_response(self) -> axum::response::Response {
        tracing::error!("Error: {}", self);
        let response = match self {
            Error::KeysRepository(_) => (StatusCode::INTERNAL_SERVER_ERROR, String::new()),
            Error::SignaturesRepository(_) => (StatusCode::INTERNAL_SERVER_ERROR, String::new()),
            Error::ProofRepository(_) => (StatusCode::INTERNAL_SERVER_ERROR, String::new()),
            Error::CommitmentRepository(_) => (StatusCode::INTERNAL_SERVER_ERROR, String::new()),

            Error::SignVerifyEcash(_) => (StatusCode::BAD_REQUEST, String::new()),
            Error::BorshVerify(_) => (StatusCode::BAD_REQUEST, String::new()),
            Error::ClowderClient(_) => (StatusCode::INTERNAL_SERVER_ERROR, String::new()),
            Error::CdkDhke(_) => (StatusCode::BAD_REQUEST, String::new()),
            Error::CDKNUT00(_) => (StatusCode::INTERNAL_SERVER_ERROR, String::new()),
            Error::CDKNUT12(_) => (StatusCode::INTERNAL_SERVER_ERROR, String::new()),
            Error::BasicChecks(e) => (StatusCode::BAD_REQUEST, e.to_string()),
            Error::Verify(e) => (StatusCode::BAD_REQUEST, e.to_string()),
            Error::Attestation(e) => (StatusCode::BAD_REQUEST, e.to_string()),

            Error::InvalidInput(e) => (StatusCode::BAD_REQUEST, e.to_string()),
            Error::Conflict(e) => (StatusCode::CONFLICT, e.to_string()),
            Error::ResourceNotFound(e) => (StatusCode::NOT_FOUND, e.to_string()),
            Error::ActiveKeyset(_) => (StatusCode::BAD_REQUEST, String::from("Active keyset")),
            Error::InactiveKeyset(_) => (StatusCode::BAD_REQUEST, String::from("Inactive keyset")),

            Error::Internal(_) => (StatusCode::INTERNAL_SERVER_ERROR, String::new()),
        };

        response.into_response()
    }
}
