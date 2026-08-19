// ----- standard library imports
use std::sync::Arc;
// ----- extra library imports
use async_trait::async_trait;
use bcr_common::{
    cashu,
    client::{
        admin::{clowder as clowder_rest, core::BRError},
        clowder::ClowderNatsClient,
    },
    core::{signature, BillId},
    wire::{attestation::AttestedFingerprints, clowder as wire_clowder, swap as wire_swap},
};
use bitcoin::secp256k1::{schnorr, PublicKey};
// ----- local imports
use crate::{
    error::{Error, Result},
    persistence::SignatureOwner,
};

// ----- end imports

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PublicKeyOwner {
    Alpha,
    Beta,
}

impl From<PublicKeyOwner> for SignatureOwner {
    fn from(value: PublicKeyOwner) -> Self {
        match value {
            PublicKeyOwner::Alpha => SignatureOwner::Alpha,
            PublicKeyOwner::Beta => SignatureOwner::Beta,
        }
    }
}

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait ClowderClient: Send + Sync {
    async fn mint_ebill(
        &self,
        keyset_id: cashu::Id,
        quote_id: uuid::Uuid,
        amount: cashu::Amount,
        bill_id: BillId,
        signatures: Vec<cashu::BlindSignature>,
    ) -> Result<Vec<cashu::BlindSignature>>;

    async fn new_keyset(&self, keyset: cashu::KeySet) -> Result<()>;

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

    async fn authenticate_attestation(
        &self,
        alpha_id: &PublicKey,
        inputs: &AttestedFingerprints,
    ) -> Result<()>;

    async fn verify_pk(&self, mint_pk: &PublicKey) -> Result<PublicKeyOwner>;
}

pub struct ClowderCl {
    pub nats: Arc<ClowderNatsClient>,
    pub rest: clowder_rest::Client,
}

#[async_trait]
impl ClowderClient for ClowderCl {
    async fn mint_ebill(
        &self,
        keyset_id: cashu::Id,
        quote_id: uuid::Uuid,
        amount: cashu::Amount,
        bill_id: BillId,
        signatures: Vec<cashu::BlindSignature>,
    ) -> Result<Vec<cashu::BlindSignature>> {
        let response = self
            .nats
            .mint_bill(
                wire_clowder::MintEbillRequest {
                    amount,
                    keyset_id,
                    quote_id,
                    bill_id,
                },
                wire_clowder::MintEbillResponse { signatures },
            )
            .await?;
        Ok(response.signatures)
    }

    async fn new_keyset(&self, keyset: cashu::KeySet) -> Result<()> {
        let request = wire_clowder::KeysetCreationRequest {
            id: keyset.id,
            expiry: keyset.final_expiry.unwrap_or_default(),
            unit: keyset.unit.clone(),
        };
        let response = wire_clowder::KeysetCreationResponse {
            public_keys: keyset.keys.keys().clone(),
            id: keyset.id,
            expiry: keyset.final_expiry.unwrap_or_default(),
            unit: keyset.unit,
        };
        self.nats.new_keyset(request, response).await?;
        Ok(())
    }

    async fn commit_to_swap(
        &self,
        request: wire_swap::SwapCommitmentRequest,
    ) -> Result<(String, schnorr::Signature)> {
        let (content, _) = signature::serialize_borsh_msg_b64(&request)
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
        self.nats.mint_swap(request, response).await?;
        Ok(())
    }

    async fn authenticate_attestation(
        &self,
        alpha_id: &PublicKey,
        inputs: &AttestedFingerprints,
    ) -> Result<()> {
        bcr_wdc_utils::attestation::authenticate_with_betas(&self.rest, alpha_id, inputs).await?;
        Ok(())
    }

    async fn verify_pk(&self, mint_pk: &PublicKey) -> Result<PublicKeyOwner> {
        if self
            .rest
            .get_betas()
            .await?
            .mints
            .iter()
            .any(|mint| mint.node_id == *mint_pk)
        {
            return Ok(PublicKeyOwner::Beta);
        }
        if self
            .rest
            .get_alphas()
            .await?
            .mints
            .iter()
            .any(|mint| mint.node_id == *mint_pk)
        {
            return Ok(PublicKeyOwner::Alpha);
        }
        tracing::warn!("unknown pubkey {mint_pk}");
        Err(Error::InvalidInput(BRError::Unknown))
    }
}
