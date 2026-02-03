// ----- standard library imports
use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
};
// ----- extra library imports
use async_trait::async_trait;
use bcr_common::cashu;
use bitcoin::secp256k1::schnorr::Signature;
// ----- local imports
use crate::{commitment, error::Result, TStamp};

// ----- end imports

type Value = (Vec<cashu::PublicKey>, Vec<cashu::PublicKey>, TStamp);
#[allow(dead_code)]
#[derive(Clone, Default)]
pub struct InMemoryCommitmentMap {
    commitments: Arc<RwLock<HashMap<Signature, Value>>>,
}

#[async_trait]
impl commitment::Repository for InMemoryCommitmentMap {
    async fn clean_expired(&self, now: TStamp) -> Result<()> {
        let mut commitments = self.commitments.write().unwrap();
        commitments.retain(|_, (_, _, expiration)| *expiration > now);
        Ok(())
    }

    async fn check_committed_inputs(&self, ys: &[cashu::PublicKey]) -> Result<bool> {
        let commitments = self.commitments.read().unwrap();
        for (_, (inputs, _, _)) in commitments.iter() {
            for y in ys {
                if inputs.contains(y) {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }

    async fn check_committed_outputs(&self, secrets: &[cashu::PublicKey]) -> Result<bool> {
        let commitments = self.commitments.read().unwrap();
        for (_, (_, outputs, _)) in commitments.iter() {
            for secret in secrets {
                if outputs.contains(secret) {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }

    async fn store(
        &self,
        mut inputs: Vec<cashu::PublicKey>,
        mut outputs: Vec<cashu::PublicKey>,
        expiration: TStamp,
        signature: Signature,
    ) -> Result<()> {
        let mut commitments = self.commitments.write().unwrap();
        inputs.sort();
        outputs.sort();
        commitments.insert(signature, (inputs, outputs, expiration));
        Ok(())
    }

    async fn find(
        &self,
        inputs: &[cashu::PublicKey],
        outputs: &[cashu::PublicKey],
    ) -> Result<Option<Signature>> {
        let mut inputs = inputs.to_vec();
        inputs.sort();
        let mut outputs = outputs.to_vec();
        outputs.sort();
        let commitments = self.commitments.read().unwrap();
        for (signature, (committed_inputs, committed_outputs, _)) in commitments.iter() {
            if *committed_inputs == inputs && *committed_outputs == outputs {
                return Ok(Some(*signature));
            }
        }
        Ok(None)
    }

    async fn delete(&self, commitment: Signature) -> Result<()> {
        let mut commitments = self.commitments.write().unwrap();
        commitments.remove(&commitment);
        Ok(())
    }
}
