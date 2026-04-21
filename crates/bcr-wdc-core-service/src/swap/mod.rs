// ----- standard library imports
use std::sync::Arc;
// ----- extra library imports
use async_trait::async_trait;
use bcr_common::{
    cashu,
    clwdr_client::ClowderNatsClient,
    core::signature,
    wire::{clowder::messages as wire_clowder, swap as wire_swap},
};
use bitcoin::secp256k1::schnorr;
// ----- local imports
use crate::{
    error::{Error, Result},
    keys::service::Service as KeysService,
};
// ----- local modules
pub mod service;

// ----- end imports

#[async_trait]
pub trait SigningService: Send + Sync {
    async fn info(&self, id: &cashu::Id) -> Result<cashu::KeySetInfo>;
    async fn sign_blinds(
        &self,
        blinds: &[cashu::BlindedMessage],
    ) -> Result<Vec<cashu::BlindSignature>>;
    async fn verify_proofs(&self, proof: &[cashu::Proof]) -> Result<()>;
    async fn verify_fingerprints(&self, fp: &[signature::ProofFingerprint]) -> Result<()>;
}

pub struct KeysSignService {
    pub keys: Arc<KeysService>,
}

#[async_trait]
impl SigningService for KeysSignService {
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
}

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait ClowderClient: Send + Sync {
    async fn commit_to_swap(
        &self,
        request: wire_swap::SwapCommitmentRequest,
    ) -> Result<(String, schnorr::Signature)>;
}

pub struct ClowderCl {
    pub nats: Arc<ClowderNatsClient>,
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
    }
}
