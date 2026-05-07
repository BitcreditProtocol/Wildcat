// ----- standard library imports
use std::{collections::HashMap, sync::Arc};
// ----- extra library imports
use async_trait::async_trait;
use bcr_common::{
    cashu,
    client::admin::clowder as clwdr_rest,
    clwdr_client::ClowderNatsClient,
    core::signature,
    wire::{
        attestation::{self as wire_attestation, IssuanceAttestation},
        clowder::messages as wire_clowder,
        swap as wire_swap,
    },
};
use bitcoin::secp256k1::{PublicKey, schnorr};
// ----- local imports
use crate::{
    error::{Error, Result},
    keys,
};
// ----- local modules
pub mod service;

// ----- end imports

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait KeysService: Send + Sync {
    async fn info(&self, id: &cashu::Id) -> Result<cashu::KeySetInfo>;
    async fn sign_blinds(
        &self,
        blinds: &[cashu::BlindedMessage],
    ) -> Result<Vec<cashu::BlindSignature>>;
    async fn verify_proofs(&self, proof: &[cashu::Proof]) -> Result<()>;
    async fn verify_fingerprints(&self, fp: &[signature::ProofFingerprint]) -> Result<()>;
    async fn list_kinfos(&self) -> Result<HashMap<cashu::Id, cashu::KeySetInfo>>;
}

pub struct KeysSignService {
    pub keys: Arc<keys::service::Service>,
}

#[async_trait]
impl KeysService for KeysSignService {
    async fn info(&self, id: &cashu::Id) -> Result<cashu::KeySetInfo> {
        self.keys.info(*id).await.map(cashu::KeySetInfo::from)
    }

    async fn sign_blinds(
        &self,
        blinds: &[cashu::BlindedMessage],
    ) -> Result<Vec<cashu::BlindSignature>> {
        let signatures = self.keys.sign_blinds(blinds.iter()).await?;
        Ok(signatures)
    }

    async fn verify_proofs(&self, proofs: &[cashu::Proof]) -> Result<()> {
        self.keys.verify_proofs(proofs).await
    }

    async fn verify_fingerprints(&self, fps: &[signature::ProofFingerprint]) -> Result<()> {
        self.keys.verify_fingerprints(fps).await?;
        Ok(())
    }

    async fn list_kinfos(&self) -> Result<HashMap<cashu::Id, cashu::KeySetInfo>> {
        let kinfos = self
            .keys
            .list_info(keys::service::ListFilters::default())
            .await?;
        let kmap: HashMap<cashu::Id, cashu::KeySetInfo> = HashMap::from_iter(
            kinfos
                .into_iter()
                .map(|kinfo| (kinfo.id, cashu::KeySetInfo::from(kinfo))),
        );
        Ok(kmap)
    }
}

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait ClowderClient: Send + Sync {
    async fn commit_to_swap(
        &self,
        request: wire_swap::SwapCommitmentRequest,
    ) -> Result<(String, schnorr::Signature)>;

    async fn post_swap(
        &self,
        inputs: Vec<cashu::Proof>,
        outputs: Vec<cashu::BlindedMessage>,
        commitment: schnorr::Signature,
        attestation: IssuanceAttestation,
        signatures: Vec<cashu::BlindSignature>,
    ) -> Result<()>;

    async fn verify_attestation(
        &self,
        alpha_id: &PublicKey,
        inputs: &[cashu::Proof],
        attestation: &IssuanceAttestation,
    ) -> Result<()>;
}

pub struct ClowderCl {
    pub nats: Arc<ClowderNatsClient>,
    pub rest: clwdr_rest::Client,
}

#[async_trait]
impl ClowderClient for ClowderCl {
    async fn commit_to_swap(
        &self,
        request: wire_swap::SwapCommitmentRequest,
    ) -> Result<(String, schnorr::Signature)> {
        let content = signature::serialize_borsh_msg_b64(&request)
            .map_err(|e| Error::Internal(format!("failed to serialize commitment: {e}")))?;
        let request = wire_clowder::SwapCommitmentRequest {
            inputs: request.inputs,
            outputs: request.outputs,
            expiry: request.expiry,
            wallet_key: request.wallet_key.into(),
        };
        let response = self.nats.swap_commitment(request).await?;
        Ok((content, response.commitment))
    }

    async fn post_swap(
        &self,
        proofs: Vec<cashu::Proof>,
        blinds: Vec<cashu::BlindedMessage>,
        commitment: schnorr::Signature,
        attestation: IssuanceAttestation,
        signatures: Vec<cashu::BlindSignature>,
    ) -> Result<()> {
        let request = wire_clowder::SwapRequest {
            proofs,
            blinds,
            commitment,
            attestation,
        };
        let response = wire_clowder::SwapResponse { signatures };
        self.nats.mint_swap(request, response).await?;
        Ok(())
    }

    async fn verify_attestation(
        &self,
        alpha_id: &PublicKey,
        inputs: &[cashu::Proof],
        attestation: &IssuanceAttestation,
    ) -> Result<()> {
        let betas = self
            .rest
            .get_betas()
            .await
            .map_err(|e| Error::Internal(format!("failed to fetch beta cohort: {e}")))?;
        let beta = betas
            .mints
            .iter()
            .find(|b| b.node_id == attestation.beta_id)
            .ok_or(Error::Attestation(
                wire_attestation::AttestationError::UnknownBeta(attestation.beta_id),
            ))?;
        wire_attestation::verify_attestation_local(alpha_id, inputs, attestation, |id| {
            id == &attestation.beta_id
        })
        .map_err(Error::Attestation)?;
        let beta_cl = clwdr_rest::Client::new(beta.clowder.clone());
        let response = beta_cl
            .post_attest_verify(&wire_attestation::AttestationVerifyRequest {
                alpha_id: *alpha_id,
                attestation: attestation.clone(),
            })
            .await
            .map_err(|e| Error::Internal(format!("attest verify call failed: {e}")))?;
        wire_attestation::verify_attestation_response(
            alpha_id,
            &attestation.beta_id,
            attestation,
            &response,
        )
        .map_err(Error::Attestation)?;
        Ok(())
    }
}

#[cfg(feature = "test-utils")]
pub mod test_utils {
    use super::*;

    pub struct DummyClowderClient;

    #[async_trait]
    impl ClowderClient for DummyClowderClient {
        async fn commit_to_swap(
            &self,
            request: wire_swap::SwapCommitmentRequest,
        ) -> Result<(String, schnorr::Signature)> {
            let mint_kp = crate::test_utils::mint_kp();
            signature::serialize_n_schnorr_sign_borsh_msg(&request, &mint_kp)
                .map_err(|e| Error::Internal(format!("failed to sign commitment: {e}")))
        }

        async fn post_swap(
            &self,
            _inputs: Vec<cashu::Proof>,
            _outputs: Vec<cashu::BlindedMessage>,
            _commitment: schnorr::Signature,
            _attestation: IssuanceAttestation,
            _signatures: Vec<cashu::BlindSignature>,
        ) -> Result<()> {
            Ok(())
        }

        async fn verify_attestation(
            &self,
            _alpha_id: &PublicKey,
            _inputs: &[cashu::Proof],
            _attestation: &IssuanceAttestation,
        ) -> Result<()> {
            Ok(())
        }
    }
}
