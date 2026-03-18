// ----- standard library imports
// ----- extra library imports
use axum::http::StatusCode;
use bcr_common::cashu::{self, nut01 as cdk01, nut02 as cdk02, nut12 as cdk12, Amount};
use bcr_wdc_utils::signatures as signatures_utils;
use thiserror::Error;
// ----- local imports

pub type Result<T> = std::result::Result<T, Error>;

#[allow(dead_code)]
#[derive(Debug, Error)]
pub enum Error {
    // external errors wrappers
    #[error("keys repository {0}")]
    KeysRepository(anyhow::Error),
    #[error("eCash sign/verify: {0}")]
    SignVerifyEcash(#[from] bcr_common::core::signature::ECashSignatureError),
    #[error("signatures repository {0}")]
    SignaturesRepository(anyhow::Error),
    #[error("clowder client {0}")]
    ClowderClient(anyhow::Error),
    #[error("Proof Repository error: {0}")]
    ProofRepository(anyhow::Error),
    #[error("DHKE error: {0}")]
    CdkDhke(#[from] cashu::dhke::Error),
    #[error("cdk::nut12 error: {0}")]
    CDKNUT12(#[from] cdk12::Error),
    #[error("invalid inputs {0}")]
    InvalidInput(String),
    #[error("invalid outputs {0}")]
    InvalidOutput(signatures_utils::ChecksError),

    #[error("MintOp not found {0}")]
    MintOpNotFound(uuid::Uuid),
    #[error("Unknown keyset {0}")]
    KeysetNotFound(cdk02::Id),
    #[error("Unknown keyset from id {0}")]
    UnknownKeysetFromId(uuid::Uuid),
    #[error("invalid mint request: {0}")]
    InvalidMintRequest(String),
    #[error("invalid generate request {0}")]
    InvalidGenerateRequest(uuid::Uuid),
    #[error("signature already exists {0}")]
    SignatureAlreadyExists(cdk01::PublicKey),
    #[error("mint operation already exists {0}")]
    MintOpAlreadyExist(uuid::Uuid),

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

    #[error("internal {0}")]
    Internal(String),
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
            Error::Internal(_) => (StatusCode::INTERNAL_SERVER_ERROR, String::new()),
            Error::MintOpAlreadyExist(_) => (StatusCode::CONFLICT, self.to_string()),
            Error::SignatureAlreadyExists(_) => (StatusCode::CONFLICT, self.to_string()),
            Error::InvalidGenerateRequest(_) => (
                StatusCode::BAD_REQUEST,
                String::from("Invalid generate request"),
            ),
            Error::InvalidMintRequest(msg) => (
                StatusCode::BAD_REQUEST,
                format!("Invalid mint request: {msg}"),
            ),
            Error::UnknownKeysetFromId(_) => (
                StatusCode::NOT_FOUND,
                String::from("Unknown keyset from id"),
            ),
            Error::KeysetNotFound(_) => (StatusCode::NOT_FOUND, self.to_string()),
            Error::MintOpNotFound(_) => (StatusCode::NOT_FOUND, self.to_string()),

            Error::ClowderClient(_) => (StatusCode::INTERNAL_SERVER_ERROR, String::new()),
            Error::SignaturesRepository(_) => (StatusCode::INTERNAL_SERVER_ERROR, String::new()),
            Error::SignVerifyEcash(_) => (StatusCode::BAD_REQUEST, String::new()),
            Error::KeysRepository(_) => (StatusCode::INTERNAL_SERVER_ERROR, String::new()),
            Error::CDKNUT12(_) => (StatusCode::INTERNAL_SERVER_ERROR, String::new()),
            Error::CdkDhke(_) => (StatusCode::BAD_REQUEST, String::new()),
            Error::ProofRepository(_) => (StatusCode::INTERNAL_SERVER_ERROR, String::new()),
        };

        response.into_response()
    }
}
