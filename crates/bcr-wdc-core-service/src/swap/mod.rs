// ----- standard library imports
use std::{collections::HashMap, sync::Arc};
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
    keys,
};
// ----- local modules
pub mod service;

// ----- end imports

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
        signatures: Vec<cashu::BlindSignature>,
    ) -> Result<()>;
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

    async fn post_swap(
        &self,
        proofs: Vec<cashu::Proof>,
        blinds: Vec<cashu::BlindedMessage>,
        commitment: schnorr::Signature,
        signatures: Vec<cashu::BlindSignature>,
    ) -> Result<()> {
        let request = wire_clowder::SwapRequest {
            proofs,
            blinds,
            commitment,
        };
        let response = wire_clowder::SwapResponse { signatures };
        self.nats.mint_swap(request, response).await?;
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
            _signatures: Vec<cashu::BlindSignature>,
        ) -> Result<()> {
            Ok(())
        }
    }
}
