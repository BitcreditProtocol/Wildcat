// ----- standard library imports
// ----- extra library imports
use async_trait::async_trait;
use bcr_common::{cashu, wire::swap as wire_swap};
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
    pub max_expiry: chrono::Duration,
}

impl Service {
    pub async fn commit(
        &self,
        now: TStamp,
        request: &wire_swap::SwapCommitmentRequest,
    ) -> Result<(Vec<cashu::PublicKey>, Vec<cashu::PublicKey>, TStamp)> {
        // expiry validation
        let expiry = chrono::DateTime::from_timestamp(request.expiry as i64, 0)
            .ok_or_else(|| Error::InvalidInput("invalid expiry timestamp".into()))?;
        if expiry <= now {
            return Err(Error::InvalidInput("commitment already expired".into()));
        }
        let max_allowed = now + self.max_expiry;
        let expiry = expiry.min(max_allowed);

        // amount validation
        let input_amount: u64 = request.inputs.iter().map(|fp| fp.amount).sum();
        let output_amount: u64 = request.outputs.iter().map(|b| u64::from(b.amount)).sum();
        if input_amount != output_amount {
            return Err(Error::InvalidInput(format!(
                "amount mismatch: inputs={input_amount}, outputs={output_amount}"
            )));
        }

        // check not already committed
        let ys: Vec<cashu::PublicKey> = request.inputs.iter().map(|fp| fp.y).collect();
        self.repo.clean_expired(now).await?;
        let any_committed = self.repo.check_committed_inputs(&ys).await?;
        if any_committed {
            return Err(Error::InvalidInput(String::from("proofs committed")));
        }
        let secrets: Vec<_> = request.outputs.iter().map(|b| b.blinded_secret).collect();
        let any_committed = self.repo.check_committed_outputs(&secrets).await?;
        if any_committed {
            return Err(Error::InvalidInput(String::from(
                "blinded messages committed",
            )));
        }

        Ok((ys, secrets, expiry))
    }

    pub async fn store_commitment(
        &self,
        ys: Vec<cashu::PublicKey>,
        secrets: Vec<cashu::PublicKey>,
        wallet_key: cashu::PublicKey,
        commitment: schnorr::Signature,
        expiry: TStamp,
    ) -> Result<()> {
        self.repo
            .store(ys, secrets, expiry, wallet_key, commitment)
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
