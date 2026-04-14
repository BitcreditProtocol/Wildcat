// ----- standard library imports
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, RwLock},
};
// ----- extra library imports
use async_trait::async_trait;
use bcr_common::{cashu, cdk_common::mint::MintKeySetInfo};
use bcr_wdc_utils::keys::KeysetEntry;
// ----- local imports
use crate::{
    error::{Error, Result},
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

    async fn list_info(
        &self,
        unit: Option<cashu::CurrencyUnit>,
        min_expiration_tstamp: Option<u64>,
        max_expiration_tstamp: Option<u64>,
    ) -> Result<Vec<MintKeySetInfo>> {
        let rlocked = self.keys.read().unwrap();
        let a = rlocked
            .iter()
            .filter_map(|(_, (info, _))| {
                if let Some(unit) = unit.clone() {
                    if info.unit != unit {
                        return None;
                    }
                }
                if info.final_expiry.unwrap_or_default()
                    <= min_expiration_tstamp.unwrap_or_default()
                {
                    return None;
                }
                if info.final_expiry.unwrap_or_default()
                    >= max_expiration_tstamp.unwrap_or_default()
                {
                    return None;
                }
                Some(info)
            })
            .cloned()
            .collect();
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
            .ok_or(Error::ResourceNotFound(format!("keyset {}", new.id)))?;
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

#[derive(Default, Debug, Clone)]
pub struct SignatureMap {
    signs: Arc<RwLock<HashMap<cashu::PublicKey, cashu::BlindSignature>>>,
}

#[async_trait]
impl persistence::SignaturesRepository for SignatureMap {
    async fn store(&self, y: cashu::PublicKey, signature: cashu::BlindSignature) -> Result<()> {
        let mut locked = self.signs.write().unwrap();
        if locked.contains_key(&y) {
            return Err(Error::InvalidInput(format!(
                "signature already exists: {}",
                y
            )));
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
            let y = cashu::dhke::hash_to_curve(&token.secret.to_bytes())?;
            items.push((y, token.clone()));
        }
        let mut locked = self.proofs.lock().unwrap();
        for (y, _) in &items {
            if locked.contains_key(y) {
                return Err(Error::InvalidInput(String::from("proofs already spent")));
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
            let y = cashu::dhke::hash_to_curve(&token.secret.to_bytes())?;
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
