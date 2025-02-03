#![allow(dead_code)]
// ----- standard library imports
// ----- extra library imports
use cdk::Amount;
use thiserror::Error;
// ----- local imports
use crate::keys::KeysetID;

pub type Result<T> = std::result::Result<T, Error>;
#[derive(Debug, Error)]
pub enum Error {
    #[error("Proof Repository error: {0}")]
    ProofRepository(#[from] anyhow::Error),
    #[error("Keyset Repository error: {0}")]
    KeysetRepository(anyhow::Error),

    #[error("DHKE error: {0}")]
    CdkDhke(#[from] cdk::dhke::Error),
    #[error("cdk::nut12 error: {0}")]
    CDKNUT12(#[from] cdk::nuts::nut12::Error),

    #[error("Already spent proofs")]
    ProofsAlreadySpent,
    #[error("Unknown proofs")]
    UnknownProofs,
    #[error("proofs cannot be merged together")]
    UnmergeableProofs,

    #[error("Unknown keyset {0}")]
    UnknownKeyset(KeysetID),
    #[error("Unknown amount {1} for keyset {0}")]
    UnknownAmountForKeyset(KeysetID, Amount),

    #[error("Zero amount is not allowed")]
    ZeroAmount,
    #[error("Unmatching amount: input {0} != output {1}")]
    UnmatchingAmount(Amount, Amount),
}
