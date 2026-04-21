// ----- standard library imports
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, RwLock},
};
// ----- extra library imports
use async_trait::async_trait;
use bcr_common::{cashu, wire::keys::ProofFingerprint};
use bitcoin::hashes::sha256::Hash as Sha256Hash;
use uuid::Uuid;
// ----- local imports
use crate::{
    credit,
    error::{Error, Result},
    foreign,
};

// ----- end imports

#[allow(dead_code)]
#[derive(Clone, Default, Debug)]
pub struct OnlineRepository {
    proofs: Arc<Mutex<Vec<((secp256k1::PublicKey, cashu::MintUrl), cashu::Proof)>>>,
    htlc: Arc<
        Mutex<HashMap<Sha256Hash, Vec<((secp256k1::PublicKey, cashu::MintUrl), cashu::Proof)>>>,
    >,
}

#[async_trait]
impl foreign::OnlineRepository for OnlineRepository {
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
pub struct OfflineRepository {
    fingerprints:
        Arc<Mutex<HashMap<Sha256Hash, ((secp256k1::PublicKey, cashu::MintUrl), ProofFingerprint)>>>,
    proofs: Arc<Mutex<HashMap<(secp256k1::PublicKey, cashu::MintUrl), Vec<cashu::Proof>>>>,
}

#[async_trait]
impl foreign::OfflineRepository for OfflineRepository {
    async fn store_fps(
        &self,
        alpha: (secp256k1::PublicKey, cashu::MintUrl),
        fps: Vec<ProofFingerprint>,
        hash: Vec<Sha256Hash>,
    ) -> Result<()> {
        let mut locked = self.fingerprints.lock().unwrap();
        for (h, fp) in hash.into_iter().zip(fps) {
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

#[allow(dead_code)]
type MintOperationsReferences = (
    HashMap<uuid::Uuid, credit::MintOperation>,
    HashMap<cashu::Id, Vec<uuid::Uuid>>,
);

#[allow(dead_code)]
#[derive(Default, Debug, Clone)]
pub struct CreditRepository {
    mintops: Arc<RwLock<MintOperationsReferences>>,
    meltops: Arc<RwLock<HashMap<cashu::Id, credit::MeltOperation>>>,
}
#[async_trait]
impl credit::Repository for CreditRepository {
    async fn mint_store(&self, mint_op: credit::MintOperation) -> Result<()> {
        let mut wlocked = self.mintops.write().unwrap();
        let (cs, cs_kid) = &mut *wlocked;
        if cs.contains_key(&mint_op.uid) {
            return Err(Error::InvalidInput(format!(
                "mint_op {}, already exists",
                mint_op.uid
            )));
        }
        let uid = mint_op.uid;
        let kid = mint_op.kid;
        cs.insert(mint_op.uid, mint_op);
        cs_kid
            .entry(kid)
            .and_modify(|conds| conds.push(uid))
            .or_insert(vec![uid]);
        Ok(())
    }
    async fn mint_load(&self, uid: Uuid) -> Result<credit::MintOperation> {
        let rlocked = self.mintops.read().unwrap();
        let (cs, _) = &*rlocked;
        let op = cs
            .get(&uid)
            .ok_or(Error::InvalidInput(format!("mint_op {uid} not found")))?;
        Ok(op.clone())
    }
    async fn mint_list(&self, kid: cashu::Id) -> Result<Vec<credit::MintOperation>> {
        let rlocked = self.mintops.read().unwrap();
        let (cs, cs_kid) = &*rlocked;
        let empty = Vec::default();
        let uids = cs_kid.get(&kid).unwrap_or(&empty);
        let mut a = Vec::with_capacity(uids.len());
        for uid in uids {
            if let Some(condition) = cs.get(uid) {
                a.push(condition.clone())
            }
        }
        Ok(a)
    }
    async fn mint_update_field(
        &self,
        uid: uuid::Uuid,
        old: cashu::Amount,
        new: cashu::Amount,
    ) -> Result<()> {
        let mut wlocked = self.mintops.write().unwrap();
        let (cs, _) = &mut *wlocked;
        let operation = cs.get_mut(&uid).ok_or(Error::Internal(format!(
            "MintOperation internal uuid does not exist {uid}"
        )))?;
        if operation.minted != old {
            return Err(Error::Internal(format!(
                "MintOperation internal minted value changed {} != {}",
                operation.minted, old
            )));
        }
        operation.minted = new;
        Ok(())
    }
    async fn melt_store(&self, melt_operation: credit::MeltOperation) -> Result<()> {
        let mut wlocked = self.meltops.write().unwrap();
        let melt_kid = melt_operation.kid;
        if wlocked.contains_key(&melt_kid) {
            return Err(Error::InvalidInput(format!(
                "melt_op for kid {melt_kid} already exists"
            )));
        }
        wlocked.insert(melt_kid, melt_operation);
        Ok(())
    }
    async fn melt_load(&self, kid: cashu::Id) -> Result<credit::MeltOperation> {
        let rlocked = self.meltops.read().unwrap();
        let melt_op = rlocked.get(&kid).ok_or(Error::UnknownKeyset(kid))?;
        Ok(melt_op.clone())
    }
    async fn melt_update_field(
        &self,
        kid: cashu::Id,
        old_melted: cashu::Amount,
        new_melted: cashu::Amount,
    ) -> Result<()> {
        let mut wlocked = self.meltops.write().unwrap();
        let melt_op = wlocked.get_mut(&kid).ok_or(Error::Internal(format!(
            "MeltOperation internal kid does not exist {kid}"
        )))?;
        if melt_op.melted != old_melted {
            return Err(Error::Internal(format!(
                "MeltOperation internal melted value changed {} != {}",
                melt_op.melted, old_melted
            )));
        }
        melt_op.melted = new_melted;
        Ok(())
    }
}
