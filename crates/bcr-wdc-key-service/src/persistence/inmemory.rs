// ----- standard library imports
use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
};
// ----- extra library imports
use async_trait::async_trait;
use bcr_wdc_utils::keys::KeysetEntry;
use cdk_common::mint::MintKeySetInfo;
use uuid::Uuid;
// ----- local imports
use crate::{
    error::{Error, Result},
    service::{KeysRepository, MintOperation, SignaturesRepository},
    TStamp,
};

// ----- end imports

type MintOperationsReferences = (
    HashMap<uuid::Uuid, MintOperation>,
    HashMap<cashu::Id, Vec<uuid::Uuid>>,
);
#[derive(Default, Debug, Clone)]
pub struct InMemoryKeyMap {
    keys: Arc<RwLock<HashMap<cashu::Id, KeysetEntry>>>,
    conditions: Arc<RwLock<MintOperationsReferences>>,
}

#[async_trait]
impl KeysRepository for InMemoryKeyMap {
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
        let (info, _) = wlocked.get_mut(&new.id).ok_or(Error::UnknownKeyset(new.id))?;
        *info = new;
        Ok(())
    }
    async fn infos_for_expiration_date(&self, expire: TStamp) -> Result<Vec<MintKeySetInfo>> {
        let rlocked = self.keys.read().unwrap();
        let tstamp = expire.timestamp() as u64;
        let infos = rlocked
            .values()
            .filter_map(|(info, _)| {
                if info.final_expiry.unwrap_or_default() > tstamp {
                    Some(info)
                } else {
                    None
                }
            })
            .cloned()
            .collect();
        Ok(infos)
    }
    async fn store_mintop(&self, mint_op: MintOperation) -> Result<()> {
        let rlocked = self.keys.read().unwrap();
        if !rlocked.contains_key(&mint_op.kid) {
            return Err(Error::UnknownKeyset(mint_op.kid));
        }
        let mut wlocked = self.conditions.write().unwrap();
        let (cs, cs_kid) = &mut *wlocked;
        if cs.contains_key(&mint_op.uid) {
            return Err(Error::Internal(format!(
                "MintCondition internal uid already exists {}",
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
    async fn load_mintop(&self, uid: Uuid) -> Result<MintOperation> {
        let rlocked = self.conditions.read().unwrap();
        let (cs, _) = &*rlocked;
        let op = cs
            .get(&uid)
            .ok_or(Error::InvalidMintRequest(format!("request unknown {uid}")))?;
        Ok(op.clone())
    }
    async fn list_mintops(&self, kid: cashu::Id) -> Result<Vec<MintOperation>> {
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
    async fn update_mintop(&self, uid: uuid::Uuid, minted: cashu::Amount) -> Result<()> {
        let mut wlocked = self.conditions.write().unwrap();
        let (cs, _) = &mut *wlocked;
        let condition = cs.get_mut(&uid).ok_or(Error::Internal(format!(
            "MintCondition internal uid does not exist {}",
            uid
        )))?;
        condition.minted = minted;
        Ok(())
    }
}
#[derive(Default, Debug, Clone)]
pub struct InMemorySignatureMap {
    signs: Arc<RwLock<HashMap<cashu::PublicKey, cashu::BlindSignature>>>,
}

#[async_trait]
impl SignaturesRepository for InMemorySignatureMap {
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
