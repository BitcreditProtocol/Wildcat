// ----- standard library imports
use std::collections::HashMap;
// ----- extra library imports
use async_trait::async_trait;
use bcr_common::{
    cashu::{self, nut07 as cdk07},
    client::core::Client as CoreClient,
    core::signature::{deserialize_borsh_msg, schnorr_verify_b64},
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
        wallet_key: cashu::PublicKey,
        commitment: schnorr::Signature,
    ) -> Result<()>;
    async fn find(
        &self,
        inputs: &[cashu::PublicKey],
        outputs: &[cashu::PublicKey],
    ) -> Result<Option<schnorr::Signature>>;
    async fn delete(&self, commitment: schnorr::Signature) -> Result<()>;
}

pub struct Service {
    pub repo: Box<dyn Repository>,
}

impl Service {
    pub const MIN_EXPIRATION: chrono::Duration = chrono::Duration::seconds(90);

    pub async fn commit(
        &self,
        now: TStamp,
        request: &wire_swap::SwapCommitmentRequest,
        core_cl: &CoreClient,
        cdk_mint_cl: &cdk::wallet::HttpClient,
    ) -> Result<()> {
        // verify wallet signature
        let xonly = request.wallet_key.x_only_public_key();
        schnorr_verify_b64(&request.content, &request.wallet_signature, &xonly)
            .map_err(|e| Error::InvalidSignature(e.to_string()))?;

        let body: wire_swap::SwapCommitmentRequestBody =
            deserialize_borsh_msg(&request.content)?;

        // amount validation
        let input_amount: u64 = body.inputs.iter().map(|fp| fp.amount).sum();
        let output_amount: u64 = body
            .outputs
            .iter()
            .map(|b| u64::from(b.amount))
            .sum();
        if input_amount != output_amount {
            return Err(Error::InvalidInput(format!(
                "amount mismatch: inputs={input_amount}, outputs={output_amount}"
            )));
        }

        // validate inputs (check unspent, verify fingerprints)
        validate_commitment_inputs(core_cl, cdk_mint_cl, &body.inputs).await?;

        // check outputs not seen (crsat)
        let signatures = core_cl.restore(body.outputs.clone()).await?;
        if !signatures.is_empty() {
            return Err(Error::InvalidInput(String::from("crsat blinds seen")));
        }
        // check outputs not seen (sat)
        let restore_request = cashu::RestoreRequest {
            outputs: body.outputs.clone(),
        };
        let restore_response = cdk_mint_cl.post_restore(restore_request).await?;
        if !restore_response.signatures.is_empty() {
            return Err(Error::InvalidInput(String::from("sat blinds seen")));
        }

        // check not already committed
        let ys: Vec<cashu::PublicKey> = body.inputs.iter().map(|fp| fp.y).collect();
        self.repo.clean_expired(now).await?;
        let any_committed = self.repo.check_committed_inputs(&ys).await?;
        if any_committed {
            return Err(Error::InvalidInput(String::from("proofs committed")));
        }
        let secrets: Vec<_> = body.outputs.iter().map(|b| b.blinded_secret).collect();
        let any_committed = self.repo.check_committed_outputs(&secrets).await?;
        if any_committed {
            return Err(Error::InvalidInput(String::from(
                "blinded messages committed",
            )));
        }

        Ok(())
    }

    pub async fn store_commitment(
        &self,
        request: &wire_swap::SwapCommitmentRequest,
        commitment: schnorr::Signature,
    ) -> Result<()> {
        let body: wire_swap::SwapCommitmentRequestBody =
            deserialize_borsh_msg(&request.content)?;
        let ys: Vec<cashu::PublicKey> = body.inputs.iter().map(|fp| fp.y).collect();
        let secrets: Vec<_> = body.outputs.iter().map(|b| b.blinded_secret).collect();
        let expiration = chrono::Utc::now() + Self::MIN_EXPIRATION;
        self.repo
            .store(ys, secrets, expiration, request.wallet_key, commitment)
            .await?;
        Ok(())
    }

    pub async fn check_swap(
        &self,
        now: TStamp,
        inputs: &[cashu::Proof],
        outputs: &[cashu::BlindedMessage],
        commitment: &schnorr::Signature,
    ) -> Result<()> {
        self.repo.clean_expired(now).await?;
        let ys = inputs
            .iter()
            .map(|p| p.y())
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let secrets: Vec<_> = outputs.iter().map(|b| b.blinded_secret).collect();

        let found = self.repo.find(&ys, &secrets).await?;
        match found {
            Some(stored) if stored == *commitment => {
                self.repo.delete(stored).await?;
                Ok(())
            }
            Some(_) => Err(Error::CommitmentMismatch),
            None => Err(Error::CommitmentNotFound),
        }
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
