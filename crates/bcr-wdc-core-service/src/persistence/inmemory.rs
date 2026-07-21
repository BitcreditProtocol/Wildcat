// ----- standard library imports
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex, MutexGuard, RwLock},
};
// ----- extra library imports
use async_trait::async_trait;
use bcr_common::{
    cashu,
    cdk_common::mint::MintKeySetInfo,
    client::admin::core::{BRError, RNFError},
};
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
    async fn infos_for_expiration_date(&self, expire: u64) -> Result<Vec<MintKeySetInfo>> {
        let rlocked = self.keys.read().unwrap();
        let mut infos: Vec<_> = rlocked
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
        infos.sort_by_key(|info| info.final_expiry.unwrap_or_default());
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
            return Err(Error::Conflict(format!("signature already exists: {}", y)));
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
    proofs: Arc<RwLock<HashMap<cashu::PublicKey, cashu::Proof>>>,
}

#[async_trait()]
impl persistence::ProofRepository for ProofMap {
    async fn insert(&self, tokens: Vec<cashu::Proof>) -> Result<()> {
        let mut items = Vec::with_capacity(tokens.len());
        let mut ys = HashSet::with_capacity(tokens.len());
        for token in tokens {
            let y = cashu::dhke::hash_to_curve(&token.secret.to_bytes())?;
            if !ys.insert(y) {
                return Err(Error::InvalidInput(BRError::Generic(String::from(
                    "proofs already spent",
                ))));
            }
            items.push((y, token.clone()));
        }
        let mut locked = self.proofs.write().unwrap();
        for (y, _) in &items {
            if locked.contains_key(y) {
                return Err(Error::InvalidInput(BRError::Generic(String::from(
                    "proofs already spent",
                ))));
            }
        }
        for (y, token) in items.into_iter() {
            locked.insert(y, token);
        }
        Ok(())
    }

    async fn remove(&self, tokens: &[cashu::PublicKey]) -> Result<()> {
        let mut locked = self.proofs.write().unwrap();
        for token in tokens {
            locked.remove(token);
        }
        Ok(())
    }

    async fn contains(&self, y: cashu::PublicKey) -> Result<Option<cashu::ProofState>> {
        let locked = self.proofs.read().unwrap();
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
    [u8; 32],
    persistence::SignatureOwner,
);
#[allow(dead_code)]
#[derive(Clone, Default)]
pub struct CommitmentMap {
    commitments: Arc<Mutex<HashMap<schnorr::Signature, Commitment>>>,
}

impl CommitmentMap {
    fn _contains_inputs(
        locked: &MutexGuard<HashMap<schnorr::Signature, Commitment>>,
        ys: &[cashu::PublicKey],
    ) -> Result<bool> {
        for (_, (inputs, _, _, _, _, _)) in locked.iter() {
            for y in ys {
                if inputs.contains(y) {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }

    fn _contains_outputs(
        locked: &MutexGuard<HashMap<schnorr::Signature, Commitment>>,
        secrets: &[cashu::PublicKey],
    ) -> Result<bool> {
        for (_, (_, outputs, _, _, _, _)) in locked.iter() {
            for secret in secrets {
                if outputs.contains(secret) {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }
}

#[async_trait]
impl persistence::CommitmentRepository for CommitmentMap {
    async fn clean_expired(&self, now: TStamp) -> Result<()> {
        let mut locked = self.commitments.lock().unwrap();
        locked.retain(|_, (_, _, expiration, _, _, _)| *expiration >= now);
        Ok(())
    }

    async fn contains_inputs(&self, ys: &[cashu::PublicKey]) -> Result<bool> {
        let locked = self.commitments.lock().unwrap();
        Self::_contains_inputs(&locked, ys)
    }

    async fn contains_outputs(&self, secrets: &[cashu::PublicKey]) -> Result<bool> {
        let locked = self.commitments.lock().unwrap();
        Self::_contains_outputs(&locked, secrets)
    }

    async fn store(
        &self,
        mut inputs: Vec<cashu::PublicKey>,
        mut outputs: Vec<cashu::PublicKey>,
        expiration: TStamp,
        wallet_key: cashu::PublicKey,
        signature: schnorr::Signature,
        fp_digest: [u8; 32],
        signed: persistence::SignatureOwner,
    ) -> Result<()> {
        inputs.sort();
        outputs.sort();
        let mut locked = self.commitments.lock().unwrap();
        if locked.contains_key(&signature) {
            return Err(Error::Conflict(format!(
                "commitment already exists: {signature}"
            )));
        }
        if Self::_contains_inputs(&locked, &inputs)? {
            return Err(Error::Conflict(String::from("inputs already used")));
        }
        if Self::_contains_outputs(&locked, &outputs)? {
            return Err(Error::Conflict(String::from("outputs already used")));
        }
        locked.insert(
            signature,
            (inputs, outputs, expiration, wallet_key, fp_digest, signed),
        );
        Ok(())
    }

    async fn load(&self, signature: &schnorr::Signature) -> Result<persistence::StoredCommitment> {
        let locked = self.commitments.lock().unwrap();
        let comm = locked
            .get(signature)
            .ok_or(Error::ResourceNotFound(RNFError::Generic(
                signature.to_string(),
            )))?
            .clone();
        Ok(persistence::StoredCommitment {
            inputs: comm.0.clone(),
            outputs: comm.1.clone(),
            expiration: comm.2,
            fp_digest: comm.4,
            signed: comm.5,
        })
    }

    async fn delete(&self, commitment: schnorr::Signature) -> Result<()> {
        let mut locked = self.commitments.lock().unwrap();
        locked.remove(&commitment);
        Ok(())
    }
}

#[derive(Default, Clone)]
pub struct ReservedYsMap {
    reserved: Arc<RwLock<HashMap<cashu::PublicKey, TStamp>>>,
}

#[async_trait]
impl persistence::ReservedYsRepository for ReservedYsMap {
    async fn store(&self, inputs: Vec<cashu::PublicKey>, deadline: TStamp) -> Result<()> {
        let mut locked = self.reserved.write().unwrap();
        for input in inputs {
            locked.insert(input, deadline);
        }
        Ok(())
    }

    async fn contains(&self, inputs: &[cashu::PublicKey]) -> Result<Vec<bool>> {
        let locked = self.reserved.read().unwrap();
        let results: Vec<bool> = inputs.iter().map(|y| locked.contains_key(y)).collect();
        Ok(results)
    }

    async fn clean_expired(&self, now: TStamp) -> Result<()> {
        let mut locked = self.reserved.write().unwrap();
        locked.retain(|_, deadline| *deadline > now);
        Ok(())
    }
}
