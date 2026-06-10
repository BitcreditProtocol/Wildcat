// ----- standard library imports
// ----- extra library imports
use bcr_common::{
    client::admin::clowder as clwdr_rest,
    wire::attestation::{AttestationError, AttestedFingerprints},
};
use bitcoin::secp256k1::PublicKey;
use thiserror::Error;
// ----- local imports

// ----- end imports

#[derive(Debug, Error)]
pub enum VerifyError {
    #[error("attestation: {0}")]
    Attestation(#[from] AttestationError),
    #[error("clowder REST: {0}")]
    Rest(#[from] clwdr_rest::Error),
}

pub async fn authenticate_with_betas(
    rest: &clwdr_rest::Client,
    alpha_id: &PublicKey,
    inputs: &AttestedFingerprints,
) -> Result<(), VerifyError> {
    let betas = rest.get_betas().await?;
    inputs.authenticate(alpha_id, |id| betas.mints.iter().any(|b| &b.node_id == id))?;
    Ok(())
}
