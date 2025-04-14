// ----- standard library imports
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
// ----- extra library imports
use async_trait::async_trait;
use cashu::{nut00 as cdk00, nut02 as cdk02, Amount};
use uuid::Uuid;
// ----- local imports
use crate::credit::{PremintSignatures, Repository};
use crate::error::{Error, Result};

#[derive(Clone, Default, Debug)]
pub struct InMemoryRepository {
    counters: Arc<Mutex<HashMap<cdk02::Id, u32>>>,
    secrets: Arc<Mutex<HashMap<Uuid, cdk00::PreMintSecrets>>>,
    signatures: Arc<Mutex<HashMap<Uuid, Vec<cdk00::BlindSignature>>>>,
    proofs: Arc<Mutex<HashMap<cdk02::Id, Vec<cdk00::Proof>>>>,
}

#[async_trait]
impl Repository for InMemoryRepository {
    async fn next_counter(&self, kid: cdk02::Id) -> Result<u32> {
        let val = self
            .counters
            .lock()
            .unwrap()
            .get(&kid)
            .copied()
            .unwrap_or_default();
        Ok(val)
    }

    async fn increment_counter(&self, kid: cdk02::Id, inc: u32) -> Result<()> {
        let mut map = self.counters.lock().unwrap();
        let val = map.get(&kid).copied().unwrap_or_default() + inc;
        map.insert(kid, val);
        Ok(())
    }

    async fn store_secrets(&self, rid: Uuid, premint: cdk00::PreMintSecrets) -> Result<()> {
        self.secrets.lock().unwrap().insert(rid, premint);
        Ok(())
    }

    async fn load_secrets(&self, rid: Uuid) -> Result<cdk00::PreMintSecrets> {
        self.secrets
            .lock()
            .unwrap()
            .get(&rid)
            .cloned()
            .ok_or_else(|| Error::RequestIDNotFound(rid))
    }

    async fn delete_secrets(&self, rid: Uuid) -> Result<()> {
        self.secrets.lock().unwrap().remove(&rid);
        Ok(())
    }

    async fn store_premint_signatures(&self, (rid, signatures): PremintSignatures) -> Result<()> {
        self.signatures.lock().unwrap().insert(rid, signatures);
        Ok(())
    }

    async fn list_premint_signatures(&self) -> Result<Vec<(Uuid, Vec<cdk00::BlindSignature>)>> {
        let cloned = self
            .signatures
            .lock()
            .unwrap()
            .iter()
            .map(|(k, v)| (*k, v.clone()))
            .collect();
        Ok(cloned)
    }

    async fn delete_premint_signatures(&self, rid: Uuid) -> Result<()> {
        self.signatures.lock().unwrap().remove(&rid);
        Ok(())
    }

    async fn store_proofs(&self, proofs: Vec<cdk00::Proof>) -> Result<()> {
        for proof in proofs {
            let kid = proof.keyset_id;
            let mut locked = self.proofs.lock().unwrap();
            locked.entry(kid).or_default().push(proof);
        }
        Ok(())
    }

    async fn list_balance_by_keyset_id(&self) -> Result<Vec<(cdk02::Id, Amount)>> {
        let mut map = HashMap::new();
        for (kid, proofs) in self.proofs.lock().unwrap().iter() {
            let mut total = Amount::ZERO;
            for proof in proofs.iter() {
                total += proof.amount;
            }
            map.insert(*kid, total);
        }
        Ok(map.into_iter().collect())
    }
}
