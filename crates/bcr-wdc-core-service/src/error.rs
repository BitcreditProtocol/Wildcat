// ----- standard library imports
// ----- extra library imports
use axum::http::StatusCode;
use bcr_common::{
    cashu::{self, nut00 as cdk00, nut02 as cdk02, nut12 as cdk12},
    client::admin::core::{BRError, RNFError},
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
    #[error("Reserved Ys Repository error: {0}")]
    ReservedYsRepository(anyhow::Error),
    // external errors
    #[error("eCash sign/verify: {0}")]
    SignVerifyEcash(#[from] bcr_common::core::signature::ECashSignatureError),
    #[error("borsh signed verification: {0}")]
    BorshVerify(#[from] signature::BorshMsgSignatureError),
    #[error("treasury client {0}")]
    TreasuryClient(#[from] bcr_common::client::admin::treasury::Error),
    #[error("clowder client {0}")]
    ClowderClient(#[from] bcr_common::client::clowder::ClowderClientError),
    #[error("clowder rest client {0}")]
    ClowderRestClient(#[from] bcr_common::client::admin::clowder::Error),
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
    #[error("AttestationVerify: {0}")]
    AttestationVerify(#[from] bcr_wdc_utils::attestation::VerifyError),
    // domain errors
    #[error("invalid inputs {0}")]
    InvalidInput(BRError),
    #[error("resource not found {0}")]
    ResourceNotFound(RNFError),
    #[error("conflict {0}")]
    Conflict(String),

    #[error("inactive keyset {0}")]
    InactiveKeyset(cdk02::Id),
    #[error("active keyset {0}")]
    ActiveKeyset(cdk02::Id),

    #[error("internal {0}")]
    Internal(String),
    #[error("service temporarily unavailable, retry later")]
    ServiceUnavailable,
}

impl axum::response::IntoResponse for Error {
    fn into_response(self) -> axum::response::Response {
        tracing::error!("Error: {}", self);
        let response = match self {
            Error::KeysRepository(_) => (StatusCode::INTERNAL_SERVER_ERROR, String::new()),
            Error::SignaturesRepository(_) => (StatusCode::INTERNAL_SERVER_ERROR, String::new()),
            Error::ProofRepository(_) => (StatusCode::INTERNAL_SERVER_ERROR, String::new()),
            Error::CommitmentRepository(_) => (StatusCode::INTERNAL_SERVER_ERROR, String::new()),
            Error::ReservedYsRepository(_) => (StatusCode::INTERNAL_SERVER_ERROR, String::new()),

            Error::SignVerifyEcash(e) => {
                let v = BRError::Generic(e.to_string());
                let j = serde_json::to_string(&v).unwrap_or_default();
                (StatusCode::BAD_REQUEST, j)
            }
            Error::BorshVerify(e) => {
                let v = BRError::Generic(e.to_string());
                let j = serde_json::to_string(&v).unwrap_or_default();
                (StatusCode::BAD_REQUEST, j)
            }
            Error::ClowderClient(_) => (StatusCode::INTERNAL_SERVER_ERROR, String::new()),
            Error::TreasuryClient(_) => (StatusCode::INTERNAL_SERVER_ERROR, String::new()),
            Error::ClowderRestClient(_) => (StatusCode::INTERNAL_SERVER_ERROR, String::new()),
            Error::CdkDhke(e) => {
                let v = BRError::Generic(e.to_string());
                let j = serde_json::to_string(&v).unwrap_or_default();
                (StatusCode::BAD_REQUEST, j)
            }
            Error::CDKNUT00(_) => (StatusCode::INTERNAL_SERVER_ERROR, String::new()),
            Error::CDKNUT12(_) => (StatusCode::INTERNAL_SERVER_ERROR, String::new()),
            Error::BasicChecks(e) => {
                let v = BRError::Generic(e.to_string());
                let j = serde_json::to_string(&v).unwrap_or_default();
                (StatusCode::BAD_REQUEST, j)
            }
            Error::Verify(e) => {
                let v = BRError::Generic(e.to_string());
                let j = serde_json::to_string(&v).unwrap_or_default();
                (StatusCode::BAD_REQUEST, j)
            }
            Error::Attestation(e) => {
                let v = BRError::Generic(e.to_string());
                let j = serde_json::to_string(&v).unwrap_or_default();
                (StatusCode::BAD_REQUEST, j)
            }
            Error::AttestationVerify(e) => match e {
                bcr_wdc_utils::attestation::VerifyError::Attestation(a) => {
                    let v = BRError::Generic(a.to_string());
                    let j = serde_json::to_string(&v).unwrap_or_default();
                    (StatusCode::BAD_REQUEST, j)
                }
                bcr_wdc_utils::attestation::VerifyError::Rest(_) => {
                    (StatusCode::INTERNAL_SERVER_ERROR, String::new())
                }
            },
            Error::InvalidInput(brerror) => {
                let v = serde_json::to_string(&brerror).unwrap_or(brerror.to_string());
                (StatusCode::BAD_REQUEST, v)
            }
            Error::Conflict(e) => (StatusCode::CONFLICT, e.to_string()),
            Error::ResourceNotFound(e) => {
                let v = serde_json::to_string(&e).unwrap_or(e.to_string());
                (StatusCode::NOT_FOUND, v)
            }
            Error::ActiveKeyset(id) => {
                let v = BRError::Generic(format!("Active keyset: {id}"));
                let j = serde_json::to_string(&v).unwrap_or_default();
                (StatusCode::BAD_REQUEST, j)
            }
            Error::InactiveKeyset(id) => {
                let v = BRError::Generic(format!("Inactive keyset: {id}"));
                let j = serde_json::to_string(&v).unwrap_or_default();
                (StatusCode::BAD_REQUEST, j)
            }

            Error::Internal(_) => (StatusCode::INTERNAL_SERVER_ERROR, String::new()),
            Error::ServiceUnavailable => (StatusCode::SERVICE_UNAVAILABLE, String::new()),
        };

        response.into_response()
    }
}
