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

type Commitment = (
    Vec<cashu::PublicKey>,
    Vec<cashu::PublicKey>,
    TStamp,
    cashu::PublicKey,
    [u8; 32],
    persistence::SignatureOwner,
);

#[derive(Default, Clone)]
pub struct Repository {
    keys: Arc<RwLock<HashMap<cashu::Id, KeysetEntry>>>,
    signatures: Arc<RwLock<HashMap<cashu::PublicKey, cashu::BlindSignature>>>,
    proofs: Arc<RwLock<HashMap<cashu::PublicKey, cashu::Proof>>>,
    commitments: Arc<Mutex<HashMap<schnorr::Signature, Commitment>>>,
    reserved_ys: Arc<RwLock<HashMap<cashu::PublicKey, TStamp>>>,
}

impl Repository {
    fn commitment_contains_inputs_locked(
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

    fn commitment_contains_outputs_locked(
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
impl persistence::Repository for Repository {
    async fn keys_store(&self, entry: KeysetEntry) -> Result<()> {
        let mut wlocked = self.keys.write().unwrap();
        wlocked.insert(entry.0.id, entry);
        Ok(())
    }

    async fn keys_info(&self, kid: cashu::Id) -> Result<Option<MintKeySetInfo>> {
        let rlocked = self.keys.read().unwrap();
        let a = rlocked.get(&kid).map(|(info, _)| info).cloned();
        Ok(a)
    }

    async fn keys_load(&self, kid: cashu::Id) -> Result<Option<cashu::MintKeySet>> {
        let rlocked = self.keys.read().unwrap();
        let a = rlocked.get(&kid).map(|(_, keyset)| keyset).cloned();
        Ok(a)
    }

    async fn keys_list_info(
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

    async fn keys_infos_for_expiration_date(&self, expire: u64) -> Result<Vec<MintKeySetInfo>> {
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

    async fn signature_store(
        &self,
        y: cashu::PublicKey,
        signature: cashu::BlindSignature,
    ) -> Result<()> {
        let mut locked = self.signatures.write().unwrap();
        if locked.contains_key(&y) {
            return Err(Error::Conflict(format!("signature already exists: {}", y)));
        }
        locked.insert(y, signature);
        Ok(())
    }

    async fn signature_load(
        &self,
        blind: &cashu::BlindedMessage,
    ) -> Result<Option<cashu::BlindSignature>> {
        let a = self
            .signatures
            .read()
            .unwrap()
            .get(&blind.blinded_secret)
            .cloned();
        Ok(a)
    }

    async fn proofs_insert(&self, tokens: Vec<cashu::Proof>) -> Result<()> {
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

    async fn proofs_remove(&self, tokens: &[cashu::PublicKey]) -> Result<()> {
        let mut locked = self.proofs.write().unwrap();
        for token in tokens {
            locked.remove(token);
        }
        Ok(())
    }

    async fn proofs_contains(&self, y: cashu::PublicKey) -> Result<Option<cashu::ProofState>> {
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

    async fn commitment_store(
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
        if Self::commitment_contains_inputs_locked(&locked, &inputs)? {
            return Err(Error::Conflict(String::from("inputs already used")));
        }
        if Self::commitment_contains_outputs_locked(&locked, &outputs)? {
            return Err(Error::Conflict(String::from("outputs already used")));
        }
        locked.insert(
            signature,
            (inputs, outputs, expiration, wallet_key, fp_digest, signed),
        );
        Ok(())
    }

    async fn commitment_load(
        &self,
        signature: &schnorr::Signature,
    ) -> Result<persistence::StoredCommitment> {
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

    async fn commitment_contains_inputs(&self, ys: &[cashu::PublicKey]) -> Result<bool> {
        let locked = self.commitments.lock().unwrap();
        Self::commitment_contains_inputs_locked(&locked, ys)
    }

    async fn commitment_contains_outputs(&self, secrets: &[cashu::PublicKey]) -> Result<bool> {
        let locked = self.commitments.lock().unwrap();
        Self::commitment_contains_outputs_locked(&locked, secrets)
    }

    async fn commitment_delete(&self, commitment: schnorr::Signature) -> Result<()> {
        let mut locked = self.commitments.lock().unwrap();
        locked.remove(&commitment);
        Ok(())
    }

    async fn commitment_clean_expired(&self, now: TStamp) -> Result<()> {
        let mut locked = self.commitments.lock().unwrap();
        locked.retain(|_, (_, _, expiration, _, _, _)| *expiration >= now);
        Ok(())
    }

    async fn ys_store(&self, inputs: Vec<cashu::PublicKey>, deadline: TStamp) -> Result<()> {
        let duplicate_check = inputs.iter().collect::<HashSet<_>>();
        if duplicate_check.len() != inputs.len() {
            let err_msg = String::from("ys already reserved");
            return Err(Error::Conflict(err_msg));
        }
        let mut locked = self.reserved_ys.write().unwrap();
        for input in &inputs {
            if locked.contains_key(input) {
                let err_msg = String::from("ys already reserved");
                return Err(Error::Conflict(err_msg));
            }
        }
        for input in inputs {
            locked.insert(input, deadline);
        }
        Ok(())
    }

    async fn ys_contains(&self, inputs: &[cashu::PublicKey]) -> Result<Vec<bool>> {
        let locked = self.reserved_ys.read().unwrap();
        let results: Vec<bool> = inputs.iter().map(|y| locked.contains_key(y)).collect();
        Ok(results)
    }

    async fn ys_clean_expired(&self, now: TStamp) -> Result<()> {
        let mut locked = self.reserved_ys.write().unwrap();
        locked.retain(|_, deadline| *deadline >= now);
        Ok(())
    }
}
