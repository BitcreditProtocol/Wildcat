#![allow(dead_code)]
// ----- standard library imports
// ----- extra library imports
use anyhow::Result as AnyResult;
use async_trait::async_trait;
use cdk::mint::MintKeySetInfo;
use cdk::nuts::nut00::{BlindSignature, BlindedMessage, Proof};
use cdk::nuts::nut02::MintKeySet;
use cdk::nuts::nut07 as cdk07;
use cdk::Amount;
// ----- local imports
use crate::keys::KeysetID;
use crate::swap::error::{Error, Result};

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait KeysRepository {
    async fn keyset(&self, id: &KeysetID) -> AnyResult<Option<MintKeySet>>;
    async fn info(&self, id: &KeysetID) -> AnyResult<Option<MintKeySetInfo>>;
    // in case keyset id is inactive, returns the proper replacement for it
    async fn replacing_id(&self, id: &KeysetID) -> AnyResult<Option<KeysetID>>;
}

#[cfg_attr(test, mockall::automock)]
pub trait ProofRepository {
    fn spend(&self, tokens: &[Proof]) -> AnyResult<()>;
    fn get_state(&self, tokens: &[Proof]) -> AnyResult<Vec<cdk07::State>>;
}

#[derive(Clone)]
pub struct Service<KeysRepo, ProofRepo> {
    pub keys: KeysRepo,
    pub proofs: ProofRepo,
}

impl<KeysRepo, ProofRepo> Service<KeysRepo, ProofRepo>
where
    KeysRepo: KeysRepository,
    ProofRepo: ProofRepository,
{
    fn verify_proofs_are_unspent(&self, proofs: &[Proof]) -> Result<bool> {
        let result = self
            .proofs
            .get_state(proofs)
            .map_err(Error::ProofRepository)?
            .into_iter()
            .all(|state| state == cdk07::State::Unspent);
        Ok(result)
    }

    async fn verify_proofs_signatures(&self, proofs: &[Proof]) -> Result<bool> {
        for proof in proofs {
            let id = proof.keyset_id;
            let keyset = self
                .keys
                .keyset(&id.into())
                .await
                .map_err(Error::KeysetRepository)?
                .ok_or_else(|| Error::UnknownKeyset(id.into()))?;
            let key = keyset
                .keys
                .get(&proof.amount)
                .ok_or_else(|| Error::UnknownAmountForKeyset(id.into(), proof.amount))?;
            let ok = cdk::dhke::verify_message(&key.secret_key, proof.c, proof.secret.as_bytes());
            if ok.is_err() {
                return Ok(false);
            }
        }
        Ok(true)
    }

    pub async fn swap(
        &self,
        inputs: &[Proof],
        outputs: &[BlindedMessage],
    ) -> Result<Vec<BlindSignature>> {
        if inputs.is_empty() {
            return Err(Error::ZeroAmount);
        }
        // first step: zero-cost verifications
        let no_zero_amount = outputs.iter().all(|output| output.amount != Amount::ZERO);
        if !no_zero_amount {
            return Err(Error::ZeroAmount);
        }
        let total_input: Amount = inputs
            .iter()
            .fold(Amount::ZERO, |total, proof| total + proof.amount);
        let total_output: Amount = outputs
            .iter()
            .fold(Amount::ZERO, |total, output| total + output.amount);
        log::debug!(
            "Received swap request: {} inputs totaling {}, {} outputs totaling {}",
            inputs.len(),
            total_input,
            outputs.len(),
            total_output
        );
        if total_input != total_output {
            return Err(Error::UnmatchingAmount(total_input, total_output));
        }
        // second step: costly verifications
        let proofs_are_unspent = self.verify_proofs_are_unspent(inputs)?;
        if !proofs_are_unspent {
            return Err(Error::ProofsAlreadySpent);
        }
        let proofs_signatures_are_ok = self.verify_proofs_signatures(inputs).await?;
        if !proofs_signatures_are_ok {
            return Err(Error::UnknownProofs);
        }

        let mut ids: Vec<KeysetID> = Vec::new();
        for i in inputs {
            let o = self
                .keys
                .replacing_id(&i.keyset_id.into())
                .await
                .map_err(Error::KeysetRepository)?
                .ok_or(Error::UnknownKeyset(i.keyset_id.into()))?;
            ids.push(o);
        }
        let first = ids.first().expect("first is None");
        if ids.iter().any(|id| *id != *first) {
            return Err(Error::UnmergeableProofs);
        }

        let keys = self
            .keys
            .keyset(first)
            .await
            .map_err(Error::KeysetRepository)?
            .expect("Keyset from first not found");
        let mut signatures = Vec::new();
        for output in outputs {
            let keypair = keys
                .keys
                .get(&output.amount)
                .ok_or(Error::UnknownAmountForKeyset(*first, output.amount))?;
            let c = cdk::dhke::sign_message(&keypair.secret_key, &output.blinded_secret)?;
            let signature = BlindSignature::new(
                output.amount,
                c,
                keys.id,
                &output.blinded_secret,
                keypair.secret_key.clone(),
            )?;
            signatures.push(signature);
        }
        self.proofs.spend(inputs).map_err(Error::ProofRepository)?;
        Ok(signatures)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keys::tests as keys;
    use crate::utils::tests as utils;
    use mockall::predicate::*;

    #[tokio::test]
    async fn test_swap_spent_proofs() {
        let keys = keys::generate_keyset();
        let inputs = utils::generate_proofs(&keys, vec![Amount::from(8)].as_slice());
        let outputs: Vec<_> = utils::generate_blinds(&keys, vec![Amount::from(8)].as_slice())
            .into_iter()
            .map(|a| a.0)
            .collect();

        let keyrepo = MockKeysRepository::new();
        let mut proofrepo = MockProofRepository::new();
        proofrepo
            .expect_get_state()
            .returning(|_| Ok(vec![cdk07::State::Spent]));
        let swaps = Service {
            keys: keyrepo,
            proofs: proofrepo,
        };
        let r = swaps.swap(&inputs, &outputs).await;
        assert!(r.is_err());
        let e = r.unwrap_err();
        assert!(matches!(e, Error::ProofsAlreadySpent));
    }

    #[tokio::test]
    async fn test_swap_unknown_keysetid() {
        let kid = keys::generate_random_keysetid();
        let id = kid.into();

        let keys = keys::generate_keyset();
        let mut inputs = utils::generate_proofs(&keys, vec![Amount::from(8)].as_slice());
        inputs.get_mut(0).unwrap().keyset_id = id;
        let outputs: Vec<_> = utils::generate_blinds(&keys, vec![Amount::from(8)].as_slice())
            .into_iter()
            .map(|a| a.0)
            .collect();

        let mut keyrepo = MockKeysRepository::new();
        let mut proofrepo = MockProofRepository::new();
        proofrepo
            .expect_get_state()
            .returning(|_| Ok(vec![cdk07::State::Unspent]));
        keyrepo
            .expect_keyset()
            .with(eq(kid))
            .returning(|_| Ok(None));
        let swaps = Service {
            keys: keyrepo,
            proofs: proofrepo,
        };

        let r = swaps.swap(&inputs, &outputs).await;
        assert!(r.is_err());
        let e = r.unwrap_err();
        assert!(matches!(e, Error::UnknownKeyset(_)));
    }

    #[tokio::test]
    async fn test_swap_wrong_signatures() {
        let keys = keys::generate_keyset();
        let mut inputs = utils::generate_proofs(&keys, vec![Amount::from(8)].as_slice());
        inputs.get_mut(0).unwrap().c = utils::publics()[0];
        let outputs: Vec<_> = utils::generate_blinds(&keys, vec![Amount::from(8)].as_slice())
            .into_iter()
            .map(|a| a.0)
            .collect();
        let mut keyrepo = MockKeysRepository::new();
        let mut proofrepo = MockProofRepository::new();
        proofrepo
            .expect_get_state()
            .returning(|_| Ok(vec![cdk07::State::Unspent]));
        let kid = KeysetID::from(keys.id);
        keyrepo
            .expect_keyset()
            .with(eq(kid))
            .returning(move |_| Ok(Some(keys.clone())));
        let swaps = Service {
            keys: keyrepo,
            proofs: proofrepo,
        };

        let r = swaps.swap(&inputs, &outputs).await;
        assert!(r.is_err());
        let e = r.unwrap_err();
        assert!(matches!(e, Error::UnknownProofs));
    }

    #[tokio::test]
    async fn test_swap_unmatched_amounts() {
        let keys = keys::generate_keyset();
        let inputs = utils::generate_proofs(&keys, vec![Amount::from(8)].as_slice());
        let outputs: Vec<_> = utils::generate_blinds(&keys, vec![Amount::from(16)].as_slice())
            .into_iter()
            .map(|a| a.0)
            .collect();
        let mut keyrepo = MockKeysRepository::new();
        let mut proofrepo = MockProofRepository::new();
        proofrepo
            .expect_get_state()
            .returning(|_| Ok(vec![cdk07::State::Unspent]));
        let kid = KeysetID::from(keys.id);
        keyrepo
            .expect_keyset()
            .with(eq(kid))
            .returning(move |_| Ok(Some(keys.clone())));
        let swaps = Service {
            keys: keyrepo,
            proofs: proofrepo,
        };

        let r = swaps.swap(&inputs, &outputs).await;
        assert!(r.is_err());
        let e = r.unwrap_err();
        assert!(matches!(e, Error::UnmatchingAmount(_, _)));
    }

    #[tokio::test]
    async fn test_swap_split_tokens_ok() {
        let keys = keys::generate_keyset();
        let inputs = utils::generate_proofs(&keys, vec![Amount::from(8)].as_slice());
        let outputs: Vec<_> =
            utils::generate_blinds(&keys, vec![Amount::from(4), Amount::from(4)].as_slice())
                .into_iter()
                .map(|a| a.0)
                .collect();
        let mut keyrepo = MockKeysRepository::new();
        let mut proofrepo = MockProofRepository::new();
        proofrepo
            .expect_get_state()
            .returning(|_| Ok(vec![cdk07::State::Unspent]));
        let kid = KeysetID::from(keys.id);
        let ex_keys = keys.clone();
        keyrepo
            .expect_keyset()
            .with(eq(kid))
            .returning(move |_| Ok(Some(ex_keys.clone())));
        keyrepo
            .expect_replacing_id()
            .returning(move |_| Ok(Some(kid)));
        proofrepo
            .expect_spend()
            .with(eq(inputs.clone()))
            .returning(|_| Ok(()));
        let swaps = Service {
            keys: keyrepo,
            proofs: proofrepo,
        };

        let r = swaps.swap(&inputs, &outputs).await;
        assert!(r.is_ok());
        let bs = r.unwrap();
        assert!(utils::verify_signatures_data(
            &keys,
            outputs.into_iter().zip(bs.into_iter())
        ));
    }

    #[tokio::test]
    async fn test_swap_merge_tokens_ok() {
        let keys = keys::generate_keyset();
        let inputs =
            utils::generate_proofs(&keys, vec![Amount::from(4), Amount::from(4)].as_slice());
        let outputs: Vec<_> = utils::generate_blinds(&keys, vec![Amount::from(8)].as_slice())
            .into_iter()
            .map(|a| a.0)
            .collect();
        let mut keyrepo = MockKeysRepository::new();
        let mut proofrepo = MockProofRepository::new();
        proofrepo
            .expect_get_state()
            .returning(|_| Ok(vec![cdk07::State::Unspent]));
        let kid = KeysetID::from(keys.id);
        let ex_keys = keys.clone();
        keyrepo
            .expect_keyset()
            .with(eq(kid))
            .returning(move |_| Ok(Some(ex_keys.clone())));
        keyrepo
            .expect_replacing_id()
            .returning(move |_| Ok(Some(kid)));
        proofrepo
            .expect_spend()
            .with(eq(inputs.clone()))
            .returning(|_| Ok(()));
        let swaps = Service {
            keys: keyrepo,
            proofs: proofrepo,
        };

        let r = swaps.swap(&inputs, &outputs).await;
        assert!(r.is_ok());
        let bs = r.unwrap();
        assert!(utils::verify_signatures_data(
            &keys,
            outputs.into_iter().zip(bs.into_iter())
        ));
    }
}
