// ----- standard library imports
use std::{collections::HashMap, sync::Arc};
// ----- extra library imports
use async_trait::async_trait;
use bcr_common::{
    cashu,
    client::admin::treasury::Client as TreasuryClient,
    clwdr_client::ClowderNatsClient,
    core::signature,
    wire::{clowder as wire_clowder, swap as wire_swap},
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
pub trait TreasuryService: Send + Sync {
    async fn store_proofs(&self, proofs: Vec<cashu::Proof>) -> Result<()>;
}

pub struct TreasuryCl {
    pub cl: Box<TreasuryClient>,
}
#[async_trait]
impl TreasuryService for TreasuryCl {
    async fn store_proofs(&self, proofs: Vec<cashu::Proof>) -> Result<()> {
        self.cl.fees_store_proofs(proofs).await?;
        Ok(())
    }
}

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
    async fn get_keyset(&self, kid: &cashu::Id) -> Result<cashu::KeySet>;
}

pub struct KeysSignService {
    pub srvc: Arc<keys::service::Service>,
}

#[async_trait]
impl KeysService for KeysSignService {
    async fn info(&self, id: &cashu::Id) -> Result<cashu::KeySetInfo> {
        self.srvc.info(*id).await.map(cashu::KeySetInfo::from)
    }

    async fn sign_blinds(
        &self,
        blinds: &[cashu::BlindedMessage],
    ) -> Result<Vec<cashu::BlindSignature>> {
        let signatures = self.srvc.sign_blinds(blinds.iter()).await?;
        Ok(signatures)
    }

    async fn verify_proofs(&self, proofs: &[cashu::Proof]) -> Result<()> {
        self.srvc.verify_proofs(proofs).await
    }

    async fn verify_fingerprints(&self, fps: &[signature::ProofFingerprint]) -> Result<()> {
        self.srvc.verify_fingerprints(fps).await?;
        Ok(())
    }

    async fn list_kinfos(&self) -> Result<HashMap<cashu::Id, cashu::KeySetInfo>> {
        let kinfos = self
            .srvc
            .list_info(keys::service::ListFilters::default())
            .await?;
        let kmap: HashMap<cashu::Id, cashu::KeySetInfo> = HashMap::from_iter(
            kinfos
                .into_iter()
                .map(|kinfo| (kinfo.id, cashu::KeySetInfo::from(kinfo))),
        );
        Ok(kmap)
    }

    async fn get_keyset(&self, kid: &cashu::Id) -> Result<cashu::KeySet> {
        let keyset = self.srvc.keys(*kid).await?;
        Ok(cashu::KeySet::from(keyset))
    }
}

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait ClowderClient: Send + Sync {
    async fn commit_to_swap(
        &self,
        request: wire_swap::SwapCommitmentRequest,
    ) -> Result<(String, schnorr::Signature)>;

    async fn signal_swap_event(
        &self,
        inputs: Vec<cashu::Proof>,
        outputs: Vec<cashu::BlindedMessage>,
        fees: Vec<cashu::BlindSignature>,
        commitment: schnorr::Signature,
        signatures: Vec<cashu::BlindSignature>,
    ) -> Result<()>;
}

pub struct ClowderCl {
    pub stream: Arc<ClowderNatsClient>,
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
        let response = self.stream.swap_commitment(request).await?;
        Ok((content, response.commitment))
    }

    async fn signal_swap_event(
        &self,
        proofs: Vec<cashu::Proof>,
        blinds: Vec<cashu::BlindedMessage>,
        fees: Vec<cashu::BlindSignature>,
        commitment: schnorr::Signature,
        signatures: Vec<cashu::BlindSignature>,
    ) -> Result<()> {
        let request = wire_clowder::SwapRequest {
            proofs,
            blinds,
            commitment,
        };
        let response = wire_clowder::SwapResponse { signatures, fees };
        self.stream.mint_swap(request, response).await?;
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

        async fn signal_swap_event(
            &self,
            _inputs: Vec<cashu::Proof>,
            _outputs: Vec<cashu::BlindedMessage>,
            _fees: Vec<cashu::BlindSignature>,
            _commitment: schnorr::Signature,
            _signatures: Vec<cashu::BlindSignature>,
        ) -> Result<()> {
            Ok(())
        }
    }

    pub struct DummyTreasuryClient;
    #[async_trait]
    impl TreasuryService for DummyTreasuryClient {
        async fn store_proofs(&self, _proofs: Vec<cashu::Proof>) -> Result<()> {
            Ok(())
        }
    }
}
