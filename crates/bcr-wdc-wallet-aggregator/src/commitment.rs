// ----- standard library imports
use std::collections::HashMap;
// ----- extra library imports
use async_trait::async_trait;
use bcr_common::{
    cashu::{self, nut07 as cdk07},
    client::core::Client as CoreClient,
    core::signature::deserialize_borsh_msg,
    wire::{keys as wire_keys, swap as wire_swap},
};
use bitcoin::secp256k1::schnorr;
// ----- local imports
use crate::{
    error::{Error, Result},
    TStamp,
};

// ----- end imports

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait Repository: Send + Sync {
    async fn clean_expired(&self, now: TStamp) -> Result<()>;
    /// check if any of the proof fingerprints appear in an existing commitment
    /// returns true if any of the proofs is committed
    async fn check_committed_inputs(&self, ys: &[cashu::PublicKey]) -> Result<bool>;
    /// check if any of the blinded Messages appear in an existing commitment
    /// returns true if any of the secrets is committed
    async fn check_committed_outputs(&self, secrets: &[cashu::PublicKey]) -> Result<bool>;

    async fn store(
        &self,
        inputs: Vec<cashu::PublicKey>,
        outputs: Vec<cashu::PublicKey>,
        expiration: TStamp,
        commitment: schnorr::Signature,
    ) -> Result<()>;
    async fn find(
        &self,
        inputs: &[cashu::PublicKey],
        outputs: &[cashu::PublicKey],
    ) -> Result<Option<schnorr::Signature>>;
    async fn delete(&self, commitment: schnorr::Signature) -> Result<()>;
}

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait Signer: Send + Sync {
    async fn sign(&self, content: &[u8]) -> Result<bitcoin::secp256k1::schnorr::Signature>;
}

pub struct Service {
    pub repo: Box<dyn Repository>,
    pub signer: Box<dyn Signer>,
}

impl Service {
    pub const MIN_EXPIRATION: chrono::Duration = chrono::Duration::seconds(90);
    pub async fn commit(
        &self,
        now: TStamp,
        request: wire_swap::CommitmentRequest,
        core_cl: &CoreClient,
    ) -> Result<wire_swap::CommitmentResponse> {
        let payload: wire_swap::CommitmentContent = deserialize_borsh_msg(&request.content)?;
        // expiration check
        if payload.expiration < now + Self::MIN_EXPIRATION {
            return Err(Error::InvalidInput(String::from("expiration too soon")));
        }
        validate_commitment_inputs(core_cl, &payload.inputs).await?;
        let signatures = core_cl.restore(payload.outputs.clone()).await?;
        if !signatures.is_empty() {
            return Err(Error::InvalidInput(String::from("crsat blinds seen")));
        }
        // committed
        let ys: Vec<cashu::PublicKey> = payload.inputs.iter().map(|p| p.y).collect();
        self.repo.clean_expired(now).await?;
        let any_committed = self.repo.check_committed_inputs(&ys).await?;
        if any_committed {
            return Err(Error::InvalidInput(String::from("proofs committed")));
        }
        let secrets: Vec<_> = payload.outputs.iter().map(|b| b.blinded_secret).collect();
        let any_committed = self.repo.check_committed_outputs(&secrets).await?;
        if any_committed {
            return Err(Error::InvalidInput(String::from(
                "blinded messages committed",
            )));
        }
        // signing
        let serialized = borsh::to_vec(&payload)?;
        let signature = self.signer.sign(&serialized).await?;
        self.repo
            .store(ys, secrets, payload.expiration, signature)
            .await?;
        let response = wire_swap::CommitmentResponse {
            commitment: signature,
        };
        Ok(response)
    }

    // check if a swap request is ok to go ahead
    // a request can be either made of unseen inputs and outputs
    // or committed inputs and outputs
    pub async fn check_swap(&self, now: TStamp, request: cashu::SwapRequest) -> Result<bool> {
        self.repo.clean_expired(now).await?;
        let inputs = request
            .inputs()
            .iter()
            .map(|p| p.y())
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let outputs: Vec<_> = request.outputs().iter().map(|b| b.blinded_secret).collect();
        // committed swap
        let commitment = self.repo.find(&inputs, &outputs).await?;
        if let Some(commitment) = commitment {
            self.repo.delete(commitment).await?;
            return Ok(true);
        }

        let any_committed_proofs = self.repo.check_committed_inputs(&inputs).await?;
        let any_committed_blinds = self.repo.check_committed_outputs(&outputs).await?;
        Ok(!any_committed_proofs && !any_committed_blinds)
    }
}

async fn validate_commitment_inputs(
    core_cl: &CoreClient,
    inputs: &[wire_keys::ProofFingerprint],
) -> Result<()> {
    //crsat hypothesis
    let ys: Vec<cashu::PublicKey> = inputs.iter().map(|p| p.y).collect();
    let state = core_cl.check_state(ys.clone()).await?;
    let any_spent = state
        .into_iter()
        .any(|state| matches!(state.state, cdk07::State::Spent));
    if any_spent {
        return Err(Error::InvalidInput(String::from("crsat proofs spent")));
    }
    let mut by_kids: HashMap<cashu::Id, Vec<&wire_keys::ProofFingerprint>> = HashMap::new();
    for fp in inputs {
        by_kids.entry(fp.keyset_id).or_default().push(fp);
    }
    for (kid, fps) in by_kids {
        if core_cl.keyset_info(kid).await.is_ok() {
            for fp in fps {
                core_cl.verify_fingerprint(fp).await?;
            }
        }
    }
    Ok(())
}
