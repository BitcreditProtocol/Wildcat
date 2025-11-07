// ----- standard library imports
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};
// ----- extra library imports
use async_trait::async_trait;
use bcr_common::wire::keys::ProofFingerprint;
use bitcoin::hashes::sha256::Hash as Sha256Hash;
use cashu::{nut00 as cdk00, nut02 as cdk02, Amount};
use uuid::Uuid;
// ----- local imports
use crate::{
    credit::{self, PremintSignatures},
    error::{Error, Result},
    foreign,
};

#[allow(dead_code)]
#[derive(Clone, Default, Debug)]
pub struct InMemoryCreditRepository {
    secrets: Arc<Mutex<HashMap<Uuid, cdk00::PreMintSecrets>>>,
    signatures: Arc<Mutex<HashMap<Uuid, Vec<cdk00::BlindSignature>>>>,
    proofs: Arc<Mutex<HashMap<cdk02::Id, Vec<cdk00::Proof>>>>,
}

#[async_trait]
impl credit::Repository for InMemoryCreditRepository {
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
pub struct InMemoryOnlineRepository {
    proofs: Arc<Mutex<Vec<((secp256k1::PublicKey, cashu::MintUrl), cdk00::Proof)>>>,
    htlc: Arc<
        Mutex<HashMap<Sha256Hash, Vec<((secp256k1::PublicKey, cashu::MintUrl), cdk00::Proof)>>>,
    >,
}

#[async_trait]
impl foreign::OnlineRepository for InMemoryOnlineRepository {
    async fn store(
        &self,
        mint: (secp256k1::PublicKey, cashu::MintUrl),
        proofs: Vec<cashu::Proof>,
    ) -> Result<()> {
        let mut locked = self.proofs.lock().unwrap();
        for proof in proofs {
            locked.push((mint.clone(), proof));
        }
        Ok(())
    }
    async fn list(&self) -> Result<Vec<((secp256k1::PublicKey, cashu::MintUrl), cashu::Proof)>> {
        Ok(self.proofs.lock().unwrap().clone())
    }

    async fn store_htlc(
        &self,
        mint: (secp256k1::PublicKey, cashu::MintUrl),
        hash: Sha256Hash,
        proofs: Vec<cashu::Proof>,
    ) -> Result<()> {
        let mut locked = self.htlc.lock().unwrap();
        let entry = locked.entry(hash).or_default();
        for proof in proofs {
            entry.push((mint.clone(), proof));
        }
        Ok(())
    }
    async fn search_htlc(
        &self,
        hash: &Sha256Hash,
    ) -> Result<Vec<((secp256k1::PublicKey, cashu::MintUrl), cashu::Proof)>> {
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

#[allow(dead_code)]
#[derive(Clone, Default, Debug)]
pub struct InMemoryOfflineRepository {
    fingerprints:
        Arc<Mutex<HashMap<Sha256Hash, ((secp256k1::PublicKey, cashu::MintUrl), ProofFingerprint)>>>,
    proofs: Arc<Mutex<HashMap<(secp256k1::PublicKey, cashu::MintUrl), Vec<cashu::Proof>>>>,
}

#[async_trait]
impl foreign::OfflineRepository for InMemoryOfflineRepository {
    async fn store_fps(
        &self,
        alpha: (secp256k1::PublicKey, cashu::MintUrl),
        fps: Vec<ProofFingerprint>,
        hash: Vec<Sha256Hash>,
    ) -> Result<()> {
        let mut locked = self.fingerprints.lock().unwrap();
        for (h, fp) in hash.into_iter().zip(fps.into_iter()) {
            locked.insert(h, (alpha.clone(), fp));
        }
        Ok(())
    }

    async fn search_fp(
        &self,
        hash: &Sha256Hash,
    ) -> Result<Option<((secp256k1::PublicKey, cashu::MintUrl), ProofFingerprint)>> {
        let locked = self.fingerprints.lock().unwrap();
        let val = locked.get(hash).cloned();
        Ok(val)
    }

    async fn remove_fps(&self, y: &[cashu::PublicKey]) -> Result<()> {
        let mut locked = self.fingerprints.lock().unwrap();
        locked.retain(|_, (_, fp)| !y.contains(&fp.y));
        Ok(())
    }
    async fn store_proofs(
        &self,
        alpha: (secp256k1::PublicKey, cashu::MintUrl),
        proof: Vec<cashu::Proof>,
    ) -> Result<()> {
        let mut locked = self.proofs.lock().unwrap();
        locked.entry(alpha).or_default().extend(proof);
        Ok(())
    }
    async fn load_proofs(
        &self,
        alpha: &(secp256k1::PublicKey, cashu::MintUrl),
    ) -> Result<Vec<cashu::Proof>> {
        let locked = self.proofs.lock().unwrap();
        Ok(locked.get(alpha).cloned().unwrap_or_default())
    }
    async fn remove_proofs(&self, ys: &[cashu::PublicKey]) -> Result<()> {
        let mut locked = self.proofs.lock().unwrap();
        for proofs in locked.values_mut() {
            proofs.retain(|p| !ys.contains(&p.y().expect("proof should have y")));
        }
        Ok(())
    }
}
