// ----- standard library imports
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};
// ----- extra library imports
use async_trait::async_trait;
use cashu::{nut00 as cdk00, nut02 as cdk02, Amount};
use uuid::Uuid;
// ----- local imports
use crate::{
    credit::{self, PremintSignatures},
    crsat,
    error::{Error, Result},
};

#[allow(dead_code)]
#[derive(Clone, Default, Debug)]
pub struct InMemoryCreditRepository {
    counters: Arc<Mutex<HashMap<cdk02::Id, u32>>>,
    secrets: Arc<Mutex<HashMap<Uuid, cdk00::PreMintSecrets>>>,
    signatures: Arc<Mutex<HashMap<Uuid, Vec<cdk00::BlindSignature>>>>,
    proofs: Arc<Mutex<HashMap<cdk02::Id, Vec<cdk00::Proof>>>>,
}

#[async_trait]
impl credit::Repository for InMemoryCreditRepository {
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

#[allow(dead_code)]
#[derive(Clone, Default, Debug)]
pub struct InMemoryCrsatRepository {
    proofs: Arc<Mutex<Vec<(cashu::MintUrl, cdk00::Proof)>>>,
    htlc: Arc<Mutex<HashMap<String, Vec<(cashu::MintUrl, cdk00::Proof)>>>>,
}

#[async_trait]
impl crsat::Repository for InMemoryCrsatRepository {
    async fn store(&self, mint: cashu::MintUrl, proofs: Vec<cashu::Proof>) -> Result<()> {
        let mut locked = self.proofs.lock().unwrap();
        for proof in proofs {
            locked.push((mint.clone(), proof));
        }
        Ok(())
    }
    async fn list(&self) -> Result<Vec<(cashu::MintUrl, cashu::Proof)>> {
        Ok(self.proofs.lock().unwrap().clone())
    }

    async fn store_htlc(
        &self,
        mint: cashu::MintUrl,
        hash: &str,
        proofs: Vec<cashu::Proof>,
    ) -> Result<()> {
        let mut locked = self.htlc.lock().unwrap();
        let entry = locked.entry(hash.to_string()).or_default();
        for proof in proofs {
            entry.push((mint.clone(), proof));
        }
        Ok(())
    }
    async fn search_htlc(&self, hash: &str) -> Result<Vec<(cashu::MintUrl, cashu::Proof)>> {
        let locked = self.htlc.lock().unwrap();
        Ok(locked.get(hash).cloned().unwrap_or_default())
    }
    async fn remove_htlcs(&self, y: &[cashu::PublicKey]) -> Result<()> {
        let mut locked = self.htlc.lock().unwrap();
        for vals in locked.values_mut() {
            vals.retain(|(_, p)| !y.contains(&p.y().expect("proof should have y")));
        }
        Ok(())
    }
}
