// ----- standard library imports
// ----- extra library imports
use anyhow::Error as AnyError;
use axum::http::StatusCode;
use bcr_common::{
    cashu,
    cashu::{
        nut00::Error as CDK00Error, nut10::Error as CDK10Error, nut11::Error as CDK11Error,
        nut12::Error as CDK12Error, nut13::Error as CDK13Error, nut20::Error as CDK20Error,
    },
    cdk,
};
use bcr_wdc_utils::signatures as signatures_utils;
use thiserror::Error;
use uuid::Uuid;
// ----- local imports

pub type Result<T> = std::result::Result<T, Error>;
#[allow(dead_code)]
#[derive(Debug, Error)]
pub enum Error {
    // external errors wrappers
    #[error("bcr_common::signature::ecash {0}")]
    BcrEcash(#[from] bcr_common::core::signature::ECashSignatureError),
    #[error("bcr_common::signature::borsh {0}")]
    BcrBorshSignature(#[from] bcr_common::core::signature::BorshMsgSignatureError),
    #[error("borsh {0}")]
    Borsh(#[from] borsh::io::Error),
    #[error("bitcoin::address: {0}")]
    BtcParse(#[from] bitcoin::address::ParseError),

    #[error("cashu::nut00 {0}")]
    CDK00(#[from] CDK00Error),
    #[error("cashu::nut10 {0}")]
    CDK10(#[from] CDK10Error),
    #[error("cashu::nut11 {0}")]
    CDK11(#[from] CDK11Error),
    #[error("cashu::nut12 {0}")]
    CDK12(#[from] CDK12Error),
    #[error("cashu::nut13 {0}")]
    CDK13(#[from] CDK13Error),
    #[error("cashu::nut20 {0}")]
    CDK20(#[from] CDK20Error),
    #[error("CDK Wallet {0}")]
    CDKWallet(#[from] cdk::Error),
    #[error("CDK secret {0}")]
    CDKSecret(#[from] cdk::secret::Error),
    #[error("DB error {0}")]
    DB(#[source] AnyError),
    #[error("Secp256k1 error {0}")]
    Secp256k1(#[from] bitcoin::secp256k1::Error),
    #[error("Serde_json error {0}")]
    SerdeJson(#[from] serde_json::Error),
    #[error("core client {0}")]
    CoreClient(#[from] bcr_common::client::core::Error),
    #[error("clowder client {0}")]
    ClowderClient(#[from] bcr_common::clwdr_client::ClowderClientError),
    #[error("quote client {0}")]
    QuoteClient(#[from] bcr_common::client::quote::Error),
    #[error("ebill client {0}")]
    EbillClient(#[from] bcr_common::client::ebill::Error),
    // internal errors
    #[error("internal sat wallet has not enough funds: requested {0}, available {1}")]
    InsufficientFunds(cdk::Amount, cdk::Amount),

    #[error("invalid inputs {0}")]
    InvalidInput(String),
    #[error("invalid outputs {0}")]
    InvalidOutput(signatures_utils::ChecksError),
    #[error("inactive keyset {0}")]
    InactiveKeyset(cashu::Id),
    #[error("active keyset {0}")]
    ActiveKeyset(cashu::Id),
    #[error("Unmatching amount: input {0} != output {1}")]
    UnmatchingAmount(cdk::Amount, cdk::Amount),
    #[error("Unknown keyset {0}")]
    UnknownKeyset(cashu::Id),
    #[error("error in unblinding signatures {0}")]
    UnblindSignatures(String),
    #[error("request id not found {0}")]
    RequestIDNotFound(Uuid),
    #[error("ebill id not found {0}")]
    EBillNotFound(String),
    #[error("Insufficient amount for melting {0}")]
    InsufficientOnchainMeltAmount(bitcoin::Amount),
    #[error("Insufficient amount for minting {0}")]
    InsufficientOnchainMintAmount(bitcoin::Amount),
    #[error("Proofs supplied for melting does not match original request")]
    MeltAmountMismatch(cashu::Amount),
    #[error("Signatures supplied for minting does not match original request")]
    MintAmountMismatch(cashu::Amount),

    #[error("internal {0}")]
    Internal(String),

    #[error("Clowder unavailable for onchain operations")]
    ClowderUnavailable,
}

impl axum::response::IntoResponse for Error {
    fn into_response(self) -> axum::response::Response {
        tracing::error!("Error: {}", self);
        let resp = match self {
            Error::Internal(_) => (StatusCode::INTERNAL_SERVER_ERROR, String::from("")),
            Error::EBillNotFound(id) => {
                (StatusCode::NOT_FOUND, format!("EBill ID not found: {id}"))
            }
            Error::CoreClient(_) => (StatusCode::INTERNAL_SERVER_ERROR, String::from("")),
            Error::EbillClient(_) => (StatusCode::INTERNAL_SERVER_ERROR, String::from("")),
            Error::UnblindSignatures(_) => (StatusCode::INTERNAL_SERVER_ERROR, String::from("")),
            Error::RequestIDNotFound(request_id) => (
                StatusCode::NOT_FOUND,
                format!("Request ID not found: {request_id}"),
            ),
            Error::UnknownKeyset(keyset) => {
                (StatusCode::BAD_REQUEST, format!("Unknown keyset: {keyset}"))
            }
            Error::UnmatchingAmount(input, output) => (
                StatusCode::BAD_REQUEST,
                format!("Unmatching amount: input {input} != output {output}"),
            ),
            Error::ActiveKeyset(kid) => (StatusCode::BAD_REQUEST, format!("Active keyset {kid}")),
            Error::InactiveKeyset(kid) => {
                (StatusCode::BAD_REQUEST, format!("Inactive keyset {kid}"))
            }

            Error::InsufficientFunds(_, _) => (StatusCode::INTERNAL_SERVER_ERROR, String::new()),

            Error::InvalidOutput(e) => (StatusCode::BAD_REQUEST, format!("Invalid outputs: {e}")),
            Error::InvalidInput(e) => (StatusCode::BAD_REQUEST, format!("Invalid inputs: {e}")),
            Error::QuoteClient(_) => (StatusCode::INTERNAL_SERVER_ERROR, String::new()),
            Error::ClowderClient(_) => (StatusCode::INTERNAL_SERVER_ERROR, String::new()),
            Error::SerdeJson(_) => (StatusCode::INTERNAL_SERVER_ERROR, String::new()),
            Error::Secp256k1(_) => (StatusCode::INTERNAL_SERVER_ERROR, String::new()),
            Error::DB(_) => (StatusCode::INTERNAL_SERVER_ERROR, String::new()),
            Error::CDKSecret(_) => (StatusCode::BAD_REQUEST, String::new()),
            Error::CDKWallet(_) => (StatusCode::INTERNAL_SERVER_ERROR, String::new()),
            Error::CDK20(_) => (StatusCode::INTERNAL_SERVER_ERROR, String::new()),
            Error::CDK13(_) => (StatusCode::INTERNAL_SERVER_ERROR, String::new()),
            Error::CDK12(_) => (StatusCode::INTERNAL_SERVER_ERROR, String::new()),
            Error::CDK11(_) => (StatusCode::INTERNAL_SERVER_ERROR, String::new()),
            Error::CDK10(_) => (StatusCode::INTERNAL_SERVER_ERROR, String::new()),
            Error::CDK00(_) => (StatusCode::INTERNAL_SERVER_ERROR, String::new()),
            Error::BtcParse(_) => (StatusCode::BAD_REQUEST, String::new()),
            Error::Borsh(_) => (StatusCode::INTERNAL_SERVER_ERROR, String::new()),
            Error::BcrBorshSignature(_) => (StatusCode::INTERNAL_SERVER_ERROR, String::new()),
            Error::BcrEcash(_) => (StatusCode::INTERNAL_SERVER_ERROR, String::new()),
            Error::InsufficientOnchainMeltAmount(_) => (StatusCode::BAD_REQUEST, String::new()),
            Error::InsufficientOnchainMintAmount(_) => (StatusCode::BAD_REQUEST, String::new()),
            Error::MeltAmountMismatch(_) => (StatusCode::BAD_REQUEST, String::new()),
            Error::MintAmountMismatch(_) => (StatusCode::BAD_REQUEST, String::new()),
            Error::ClowderUnavailable => (
                StatusCode::SERVICE_UNAVAILABLE,
                "Clowder unavailable".to_string(),
            ),
        };
        resp.into_response()
    }
}
