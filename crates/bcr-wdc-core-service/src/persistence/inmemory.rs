// ----- standard library imports
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, RwLock},
};
// ----- extra library imports
use async_trait::async_trait;
use bcr_common::{cashu, cdk_common::mint::MintKeySetInfo};
use bcr_wdc_utils::keys::KeysetEntry;
use bitcoin::secp256k1::schnorr;
// ----- local imports
use crate::{
    error::{Error, Result},
    persistence, TStamp,
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
        let max_exp = max_expiration_tstamp.unwrap_or(u64::MAX);
        let min_exp = min_expiration_tstamp.unwrap_or(u64::MIN);
        let a = rlocked
            .iter()
            .filter_map(|(_, (info, _))| {
                if let Some(unit) = unit.clone() {
                    if info.unit != unit {
                        return None;
                    }
                }
                let exp = info.final_expiry.unwrap_or_default();
                if exp < min_exp {
                    return None;
                }
                if exp > max_exp {
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

type Commitment = (
    Vec<cashu::PublicKey>,
    Vec<cashu::PublicKey>,
    TStamp,
    cashu::PublicKey,
);
#[allow(dead_code)]
#[derive(Clone, Default)]
pub struct CommitmentMap {
    commitments: Arc<RwLock<HashMap<schnorr::Signature, Commitment>>>,
}

#[async_trait]
impl persistence::CommitmentRepository for CommitmentMap {
    async fn clean_expired(&self, now: TStamp) -> Result<()> {
        let mut commitments = self.commitments.write().unwrap();
        commitments.retain(|_, (_, _, expiration, _)| *expiration > now);
        Ok(())
    }

    async fn contains_inputs(&self, ys: &[cashu::PublicKey]) -> Result<bool> {
        let commitments = self.commitments.read().unwrap();
        for (_, (inputs, _, _, _)) in commitments.iter() {
            for y in ys {
                if inputs.contains(y) {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }

    async fn contains_outputs(&self, secrets: &[cashu::PublicKey]) -> Result<bool> {
        let commitments = self.commitments.read().unwrap();
        for (_, (_, outputs, _, _)) in commitments.iter() {
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
        wallet_key: cashu::PublicKey,
        signature: schnorr::Signature,
    ) -> Result<()> {
        let mut commitments = self.commitments.write().unwrap();
        inputs.sort();
        outputs.sort();
        commitments.insert(signature, (inputs, outputs, expiration, wallet_key));
        Ok(())
    }

    async fn load(
        &self,
        signature: &schnorr::Signature,
    ) -> Result<(Vec<cashu::PublicKey>, Vec<cashu::PublicKey>, TStamp)> {
        let comms = self.commitments.read().unwrap();
        let comm = comms
            .get(signature)
            .ok_or(Error::ResourceNotFound(signature.to_string()))?
            .clone();
        Ok((comm.0, comm.1, comm.2))
    }

    async fn delete(&self, commitment: schnorr::Signature) -> Result<()> {
        let mut commitments = self.commitments.write().unwrap();
        commitments.remove(&commitment);
        Ok(())
    }
}
