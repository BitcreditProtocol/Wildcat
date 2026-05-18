// ----- standard library imports
// ----- extra library imports
use bcr_common::{
    cashu,
    client::admin::clowder as clwdr_rest,
    wire::attestation::{self as wire_attestation, AttestationError, IssuanceAttestation},
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

pub async fn verify(
    rest: &clwdr_rest::Client,
    alpha_id: &PublicKey,
    inputs: &[cashu::Proof],
    attestation: &IssuanceAttestation,
) -> Result<(), VerifyError> {
    let betas = rest.get_betas().await?;
    wire_attestation::verify_attestation_local(alpha_id, inputs, attestation, |id| {
        betas.mints.iter().any(|b| &b.node_id == id)
    })?;
    let beta = betas
        .mints
        .iter()
        .find(|b| b.node_id == attestation.beta_id)
        .ok_or(AttestationError::UnknownBeta(attestation.beta_id))?;
    let beta_cl = clwdr_rest::Client::new(beta.clowder.clone());
    let response = beta_cl
        .post_attest_verify(&wire_attestation::AttestationVerifyRequest {
            alpha_id: *alpha_id,
            attestation: attestation.clone(),
        })
        .await?;
    wire_attestation::verify_attestation_response(
        alpha_id,
        &attestation.beta_id,
        attestation,
        &response,
    )?;
    Ok(())
}
