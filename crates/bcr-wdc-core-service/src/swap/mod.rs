// ----- standard library imports
use std::sync::Arc;
// ----- extra library imports
use async_trait::async_trait;
use bcr_common::{
    cashu,
    core::signature,
    wire::{clowder::messages as wire_clowder, swap as wire_swap},
};
use bitcoin::secp256k1::schnorr;
use clwdr_client::ClowderNatsClient;
// ----- local imports
use crate::{
    error::Result,
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
        let content = encode_commitment_content(&request)?;
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
    use crate::error::Error;
    use bitcoin::{
        base64::prelude::*,
        hashes::{sha256::Hash as Sha256, Hash},
        secp256k1 as secp,
    };

    pub struct DummyClowderClient;

    #[async_trait]
    impl ClowderClient for DummyClowderClient {
        async fn commit_to_swap(
            &self,
            request: wire_swap::SwapCommitmentRequest,
        ) -> Result<(String, schnorr::Signature)> {
            let content = encode_commitment_content(&request)?;
            let mint_kp = crate::test_utils::mint_kp();
            let unbased = BASE64_STANDARD
                .decode(&content)
                .map_err(|e| Error::InvalidInput(e.to_string()))?;
            let sha = Sha256::hash(&unbased);
            let secp_msg = secp::Message::from_digest(*sha.as_ref());
            let signature = secp::global::SECP256K1.sign_schnorr(&secp_msg, &mint_kp);
            Ok((content, signature))
        }
    }
}

fn encode_commitment_content(request: &wire_swap::SwapCommitmentRequest) -> Result<String> {
    use bitcoin::base64::{engine::general_purpose::STANDARD, Engine};
    let serialized = borsh::to_vec(request).map_err(|e| {
        crate::error::Error::Internal(format!("failed to serialize commitment: {e}"))
    })?;
    Ok(STANDARD.encode(serialized))
}
