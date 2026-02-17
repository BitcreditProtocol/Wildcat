// ----- standard library imports
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, RwLock},
};
// ----- extra library imports
use async_trait::async_trait;
use bcr_common::{cashu, cdk_common::mint::MintKeySetInfo};
use bcr_wdc_utils::keys::KeysetEntry;
use uuid::Uuid;
// ----- local imports
use crate::{
    error::{Error, Result},
    keys::service::MintOperation,
    persistence,
};

// ----- end imports

#[derive(Default, Debug, Clone)]
pub struct KeyMap {
    keys: Arc<RwLock<HashMap<cashu::Id, KeysetEntry>>>,
}

#[async_trait]
impl persistence::KeysRepository for KeyMap {
    async fn store(&self, entry: KeysetEntry) -> Result<()> {
        let mut wlocked = self.keys.write().unwrap();
        wlocked.insert(entry.0.id, entry);
        Ok(())
    }
    async fn info(&self, kid: cashu::Id) -> Result<Option<MintKeySetInfo>> {
        let rlocked = self.keys.read().unwrap();
        let a = rlocked.get(&kid).map(|(info, _)| info).cloned();
        Ok(a)
    }
    async fn keyset(&self, kid: cashu::Id) -> Result<Option<cashu::MintKeySet>> {
        let rlocked = self.keys.read().unwrap();
        let a = rlocked.get(&kid).map(|(_, keyset)| keyset).cloned();
        Ok(a)
    }
    async fn list_info(&self) -> Result<Vec<MintKeySetInfo>> {
        let rlocked = self.keys.read().unwrap();
        let a = rlocked.iter().map(|(_, (info, _))| info).cloned().collect();
        Ok(a)
    }
    async fn list_keyset(&self) -> Result<Vec<cashu::MintKeySet>> {
        let rlocked = self.keys.read().unwrap();
        let a = rlocked
            .iter()
            .map(|(_, (_, keyset))| keyset)
            .cloned()
            .collect();
        Ok(a)
    }
    async fn update_info(&self, new: MintKeySetInfo) -> Result<()> {
        let mut wlocked = self.keys.write().unwrap();
        let (info, _) = wlocked
            .get_mut(&new.id)
            .ok_or(Error::KeysetNotFound(new.id))?;
        *info = new;
        Ok(())
    }
    async fn infos_for_expiration_date(&self, expire: u64) -> Result<Vec<MintKeySetInfo>> {
        let rlocked = self.keys.read().unwrap();
        let infos = rlocked
            .values()
            .filter_map(|(info, _)| {
                if info.final_expiry.unwrap_or_default() >= expire {
                    Some(info)
                } else {
                    None
                }
            })
            .cloned()
            .collect();
        Ok(infos)
    }
}

type MintOperationsReferences = (
    HashMap<uuid::Uuid, MintOperation>,
    HashMap<cashu::Id, Vec<uuid::Uuid>>,
);

#[derive(Default, Debug, Clone)]
pub struct MintOpMap {
    conditions: Arc<RwLock<MintOperationsReferences>>,
}
#[async_trait]
impl persistence::MintOpRepository for MintOpMap {
    async fn store(&self, mint_op: MintOperation) -> Result<()> {
        let mut wlocked = self.conditions.write().unwrap();
        let (cs, cs_kid) = &mut *wlocked;
        if cs.contains_key(&mint_op.uid) {
            return Err(Error::MintOpAlreadyExist(mint_op.uid));
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
    async fn load(&self, uid: Uuid) -> Result<MintOperation> {
        let rlocked = self.conditions.read().unwrap();
        let (cs, _) = &*rlocked;
        let op = cs.get(&uid).ok_or(Error::MintOpNotFound(uid))?;
        Ok(op.clone())
    }
    async fn list(&self, kid: cashu::Id) -> Result<Vec<MintOperation>> {
        let rlocked = self.conditions.read().unwrap();
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
    async fn update(&self, uid: uuid::Uuid, old: cashu::Amount, new: cashu::Amount) -> Result<()> {
        let mut wlocked = self.conditions.write().unwrap();
        let (cs, _) = &mut *wlocked;
        let condition = cs.get_mut(&uid).ok_or(Error::Internal(format!(
            "MintCondition internal uid does not exist {uid}"
        )))?;
        if condition.minted != old {
            return Err(Error::Internal(format!(
                "MintCondition internal minted value changed {} != {}",
                condition.minted, old
            )));
        }
        condition.minted = new;
        Ok(())
    }
}

#[derive(Default, Debug, Clone)]
pub struct SignatureMap {
    signs: Arc<RwLock<HashMap<cashu::PublicKey, cashu::BlindSignature>>>,
}

#[async_trait]
impl persistence::SignaturesRepository for SignatureMap {
    async fn store(&self, y: cashu::PublicKey, signature: cashu::BlindSignature) -> Result<()> {
        let mut locked = self.signs.write().unwrap();
        if locked.contains_key(&y) {
            return Err(Error::SignatureAlreadyExists(y));
        }
        locked.insert(y, signature);
        Ok(())
    }
    async fn load(&self, blind: &cashu::BlindedMessage) -> Result<Option<cashu::BlindSignature>> {
        let a = self
            .signs
            .read()
            .unwrap()
            .get(&blind.blinded_secret)
            .cloned();
        Ok(a)
    }
}

#[derive(Default, Clone)]
pub struct ProofMap {
    proofs: Arc<Mutex<HashMap<cashu::PublicKey, cashu::Proof>>>,
}

#[async_trait()]
impl persistence::ProofRepository for ProofMap {
    async fn insert(&self, tokens: &[cashu::Proof]) -> Result<()> {
        let mut items = Vec::with_capacity(tokens.len());
        for token in tokens {
            let y = cashu::dhke::hash_to_curve(&token.secret.to_bytes()).map_err(Error::CdkDhke)?;
            items.push((y, token.clone()));
        }
        let mut locked = self.proofs.lock().unwrap();
        for (y, _) in &items {
            if locked.contains_key(y) {
                return Err(Error::ProofsAlreadySpent);
            }
        }
        for (y, token) in items.into_iter() {
            locked.insert(y, token);
        }
        Ok(())
    }
    async fn remove(&self, tokens: &[cashu::Proof]) -> Result<()> {
        let mut locked = self.proofs.lock().unwrap();
        for token in tokens {
            let y = cashu::dhke::hash_to_curve(&token.secret.to_bytes()).map_err(Error::CdkDhke)?;
            locked.remove(&y);
        }
        Ok(())
    }

    async fn contains(&self, y: cashu::PublicKey) -> Result<Option<cashu::ProofState>> {
        let locked = self.proofs.lock().unwrap();
        if locked.get(&y).is_some() {
            let ret_v = cashu::ProofState {
                y,
                state: cashu::State::Spent,
                witness: None,
            };
            return Ok(Some(ret_v));
        }
        Ok(None)
    }
}
