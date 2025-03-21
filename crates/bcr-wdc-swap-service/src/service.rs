// ----- standard library imports
// ----- extra library imports
use async_trait::async_trait;
use cashu::nuts::nut00 as cdk00;
use cashu::nuts::nut02 as cdk02;
use cashu::Amount;
use futures::future::JoinAll;
use itertools::Itertools;
// ----- local imports
use crate::error::{Error, Result};

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait KeysService: Send + Sync {
    async fn info(&self, id: &cdk02::Id) -> Result<cdk02::KeySetInfo>;
    async fn sign_blind(&self, blind: &cdk00::BlindedMessage) -> Result<cdk00::BlindSignature>;
    async fn verify_proof(&self, proof: &cdk00::Proof) -> Result<()>;
}

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait ProofRepository: Send + Sync {
    /// WARNING: this method should do strict insert.
    /// i.e. it should fail if any of the proofs is already present in the DB
    /// in case of failure, the DB should be in the same state as before the call
    async fn insert(&self, tokens: &[cdk00::Proof]) -> Result<()>;
    async fn remove(&self, tokens: &[cdk00::Proof]) -> Result<()>;
}

#[derive(Clone)]
pub struct Service<KeysSrvc, ProofRepo> {
    pub keys: KeysSrvc,
    pub proofs: ProofRepo,
}

impl<KeysSrvc, ProofRepo> Service<KeysSrvc, ProofRepo>
where
    KeysSrvc: KeysService,
{
    async fn verify_proofs_signatures(&self, proofs: &[cdk00::Proof]) -> Result<()> {
        let joined: JoinAll<_> = proofs.iter().map(|p| self.keys.verify_proof(p)).collect();
        joined.await.into_iter().collect::<Result<()>>()?;
        Ok(())
    }

    async fn are_keysets_active(&self, kids: &[cdk02::Id]) -> Result<Vec<bool>> {
        let joined: JoinAll<_> = kids.iter().map(|kid| self.keys.info(kid)).collect();
        let responses: Vec<_> = joined.await.into_iter().collect::<Result<_>>()?;
        let statuses: Vec<bool> = responses.into_iter().map(|info| info.active).collect();
        Ok(statuses)
    }

    async fn sign_blinds(&self, blinds: &[cdk00::BlindedMessage]) -> Result<Vec<cdk00::BlindSignature>> {
        let joined: JoinAll<_> = blinds.iter().map(|blind| self.keys.sign_blind(blind)).collect();
        let signatures: Vec<cdk00::BlindSignature> = joined.await.into_iter().collect::<Result<_>>()?;
        Ok(signatures)
    }
}

impl<KeysSrvc, ProofRepo> Service<KeysSrvc, ProofRepo>
where
    KeysSrvc: KeysService,
    ProofRepo: ProofRepository,
{
    pub async fn swap(
        &self,
        inputs: &[cdk00::Proof],
        outputs: &[cdk00::BlindedMessage],
    ) -> Result<Vec<cdk00::BlindSignature>> {
        log::debug!(
            "Received swap request: {} inputs, {} outputs",
            inputs.len(),
            outputs.len(),
        );
        // cheap verifications
        // 1. no empty inputs or outputs
        if inputs.is_empty() || outputs.is_empty() {
            return Err(Error::EmptyInputsOrOutputs);
        }
        // 2. no zero amounts in inputs or outputs
        let zero_outputs = outputs.iter().any(|output| output.amount == Amount::ZERO);
        let zero_inputs = inputs.iter().any(|output| output.amount == Amount::ZERO);
        if zero_outputs || zero_inputs {
            return Err(Error::ZeroAmount);
        }
        // 3. inputs and outputs grouped by keyset ID have equal amounts
        let unique_ids: Vec<_> = inputs.iter().map(|p| p.keyset_id).unique().collect();
        for id in &unique_ids {
            let total_input = inputs
                .iter()
                .filter(|p| p.keyset_id == *id)
                .fold(Amount::ZERO, |total, proof| total + proof.amount);
            let total_output = outputs
                .iter()
                .filter(|p| p.keyset_id == *id)
                .fold(Amount::ZERO, |total, proof| total + proof.amount);
            if total_input != total_output {
                return Err(Error::UnmatchingAmount(total_input, total_output));
            }
        }
        // expensive verifications
        // 1. verify keysets are active
        let statuses = self.are_keysets_active(&unique_ids).await?;
        for (id, status) in unique_ids.iter().zip(statuses.iter()) {
            if !status {
                return Err(Error::InactiveKeyset(*id));
            }
        }
        // 2. verify proofs signatures
        self.verify_proofs_signatures(inputs).await?;
        // generate signatures
        let signatures = self.sign_blinds(outputs).await?;
        self.proofs.insert(inputs).await?;
        Ok(signatures)
    }

    pub async fn burn(&self, proofs: &[cdk00::Proof]) -> Result<()> {
        log::debug!("Received burn request: {} proofs", proofs.len(),);
        // cheap verifications
        // 1. no zero amounts proof
        let zero_amount = proofs.iter().any(|output| output.amount == Amount::ZERO);
        if zero_amount {
            return Err(Error::ZeroAmount);
        }
        // expensive verifications
        let unique_ids: Vec<_> = proofs.iter().map(|p| p.keyset_id).unique().collect();
        // 1. verify keysets are inactive
        let statuses = self.are_keysets_active(&unique_ids).await?;
        for (id, status) in unique_ids.iter().zip(statuses.iter()) {
            if *status {
                return Err(Error::ActiveKeyset(*id));
            }
        }
        // 2. verify proofs signatures
        self.verify_proofs_signatures(proofs).await?;

        self.proofs.insert(proofs).await?;
        Ok(())
    }

    pub async fn recover(&self, proofs: &[cdk00::Proof]) -> Result<()> {
        log::debug!("Received recover request: {} proofs", proofs.len(),);

        self.proofs.remove(proofs).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils;
    use bcr_wdc_keys::test_utils as keys_test;
    use mockall::predicate::*;

    #[tokio::test]
    async fn test_swap_spent_proofs() {
        let (keyinfo, keyset) = keys_test::generate_keyset();
        let inputs = utils::generate_proofs(&keyset, vec![Amount::from(8)].as_slice());
        let signatures = utils::generate_signatures(&keyset, vec![Amount::from(8)].as_slice());
        let outputs: Vec<_> = utils::generate_blinds(&keyset, vec![Amount::from(8)].as_slice())
            .into_iter()
            .map(|a| a.0)
            .collect();

        let mut keysrvc = MockKeysService::new();
        keysrvc
            .expect_info()
            .returning(move |_| Ok(keyinfo.clone().into()));
        keysrvc.expect_verify_proof().returning(|_| Ok(()));
        let sig = signatures[0].clone();
        keysrvc
            .expect_sign_blind()
            .returning(move |_| Ok(sig.clone()));
        let mut proofrepo = MockProofRepository::new();
        proofrepo
            .expect_insert()
            .returning(|_| Err(Error::ProofsAlreadySpent));
        let swaps = Service {
            keys: keysrvc,
            proofs: proofrepo,
        };
        let r = swaps.swap(&inputs, &outputs).await;
        assert!(r.is_err());
        let e = r.unwrap_err();
        assert!(matches!(e, Error::ProofsAlreadySpent));
    }

    #[tokio::test]
    async fn test_swap_unknown_keysetid() {
        let (_, keyset) = keys_test::generate_keyset();
        let kid = keyset.id;
        let inputs = utils::generate_proofs(&keyset, vec![Amount::from(8)].as_slice());
        let outputs: Vec<_> = utils::generate_blinds(&keyset, vec![Amount::from(8)].as_slice())
            .into_iter()
            .map(|a| a.0)
            .collect();

        let mut keysrvc = MockKeysService::new();
        let proofrepo = MockProofRepository::new();
        keysrvc
            .expect_info()
            .with(eq(kid))
            .returning(|kid| Err(Error::UnknownKeyset(*kid)));
        let swaps = Service {
            keys: keysrvc,
            proofs: proofrepo,
        };

        let r = swaps.swap(&inputs, &outputs).await;
        dbg!(&r);
        assert!(r.is_err());
        let e = r.unwrap_err();
        assert!(matches!(e, Error::UnknownKeyset(_)));
    }

    #[tokio::test]
    async fn test_swap_wrong_signatures() {
        let (keyinfo, keyset) = keys_test::generate_keyset();
        let mut inputs = utils::generate_proofs(&keyset, vec![Amount::from(8)].as_slice());
        inputs.get_mut(0).unwrap().c = keys_test::publics()[0];
        let outputs: Vec<_> = utils::generate_blinds(&keyset, vec![Amount::from(8)].as_slice())
            .into_iter()
            .map(|a| a.0)
            .collect();
        let mut keysrvc = MockKeysService::new();
        let proofrepo = MockProofRepository::new();
        let kid = keyset.id;
        keysrvc
            .expect_info()
            .with(eq(kid))
            .returning(move |_| Ok(keyinfo.clone().into()));
        keysrvc
            .expect_verify_proof()
            .returning(move |p| Err(Error::InvalidProof(p.secret.clone())));
        let swaps = Service {
            keys: keysrvc,
            proofs: proofrepo,
        };

        let r = swaps.swap(&inputs, &outputs).await;
        assert!(r.is_err());
        let e = r.unwrap_err();
        assert!(matches!(e, Error::InvalidProof(_)));
    }

    #[tokio::test]
    async fn test_swap_unmatched_amounts() {
        let (keyinfo, keyset) = keys_test::generate_keyset();
        let inputs = utils::generate_proofs(&keyset, vec![Amount::from(8)].as_slice());
        let signatures = utils::generate_signatures(&keyset, vec![Amount::from(8)].as_slice());
        let outputs: Vec<_> = utils::generate_blinds(&keyset, vec![Amount::from(16)].as_slice())
            .into_iter()
            .map(|a| a.0)
            .collect();
        let mut keysrvc = MockKeysService::new();
        let proofrepo = MockProofRepository::new();
        let kid = keyset.id;
        keysrvc
            .expect_info()
            .with(eq(kid))
            .returning(move |_| Ok(keyinfo.clone().into()));
        let sig = signatures[0].clone();
        keysrvc
            .expect_sign_blind()
            .returning(move |_| Ok(sig.clone()));
        let swaps = Service {
            keys: keysrvc,
            proofs: proofrepo,
        };

        let r = swaps.swap(&inputs, &outputs).await;
        assert!(r.is_err());
        let e = r.unwrap_err();
        assert!(matches!(e, Error::UnmatchingAmount(_, _)));
    }

    #[tokio::test]
    async fn test_swap_split_tokens_ok() {
        let (keyinfo, keyset) = keys_test::generate_keyset();
        let amounts = vec![Amount::from(4), Amount::from(4)];
        let inputs = utils::generate_proofs(&keyset, vec![Amount::from(8)].as_slice());
        let signatures = utils::generate_signatures(&keyset, &amounts);
        let outputs: Vec<_> = utils::generate_blinds(&keyset, &amounts)
            .into_iter()
            .map(|a| a.0)
            .collect();
        let mut keysrvc = MockKeysService::new();
        let mut proofrepo = MockProofRepository::new();
        let kid = keyset.id;
        keysrvc
            .expect_info()
            .with(eq(kid))
            .returning(move |_| Ok(keyinfo.clone().into()));
        keysrvc.expect_verify_proof().returning(move |_| Ok(()));
        let sig_clone = signatures[0].clone();
        let blind_clone = outputs[0].clone();
        keysrvc
            .expect_sign_blind()
            .with(eq(blind_clone.clone()))
            .returning(move |_| Ok(sig_clone.clone()));
        let sig_clone = signatures[1].clone();
        let blind_clone = outputs[1].clone();
        keysrvc
            .expect_sign_blind()
            .with(eq(blind_clone.clone()))
            .returning(move |_| Ok(sig_clone.clone()));
        proofrepo
            .expect_insert()
            .with(eq(inputs.clone()))
            .returning(|_| Ok(()));
        let swaps = Service {
            keys: keysrvc,
            proofs: proofrepo,
        };

        let r = swaps.swap(&inputs, &outputs).await;
        assert!(r.is_ok());
        let bs = r.unwrap();
        assert!(utils::verify_signatures_data(
            &keyset,
            outputs.into_iter().zip(bs.into_iter())
        ));
    }

    #[tokio::test]
    async fn test_swap_merge_tokens_ok() {
        let (keyinfo, keyset) = keys_test::generate_keyset();
        let inputs =
            utils::generate_proofs(&keyset, vec![Amount::from(4), Amount::from(4)].as_slice());
        let amounts = vec![Amount::from(8)];
        let signatures = utils::generate_signatures(&keyset, &amounts);
        let outputs: Vec<_> = utils::generate_blinds(&keyset, &amounts)
            .into_iter()
            .map(|a| a.0)
            .collect();
        let mut keysrvc = MockKeysService::new();
        let mut proofrepo = MockProofRepository::new();
        let kid = keyset.id;
        keysrvc
            .expect_info()
            .with(eq(kid))
            .returning(move |_| Ok(keyinfo.clone().into()));
        keysrvc
            .expect_verify_proof()
            .times(2)
            .returning(move |_| Ok(()));
        let sig_clone = signatures[0].clone();
        keysrvc
            .expect_sign_blind()
            .returning(move |_| Ok(sig_clone.clone()));
        proofrepo
            .expect_insert()
            .with(eq(inputs.clone()))
            .returning(|_| Ok(()));
        let swaps = Service {
            keys: keysrvc,
            proofs: proofrepo,
        };

        let r = swaps.swap(&inputs, &outputs).await;
        assert!(r.is_ok());
        let bs = r.unwrap();
        assert!(utils::verify_signatures_data(
            &keyset,
            outputs.into_iter().zip(bs.into_iter())
        ));
    }

    #[tokio::test]
    async fn burn_active_keyset() {
        let (keyinfo, keyset) = keys_test::generate_keyset();
        let inputs = utils::generate_proofs(&keyset, vec![Amount::from(8)].as_slice());

        let mut keysrvc = MockKeysService::new();
        keysrvc
            .expect_info()
            .returning(move |_| Ok(keyinfo.clone().into()));
        keysrvc.expect_verify_proof().returning(|_| Ok(()));
        let mut proofrepo = MockProofRepository::new();
        proofrepo
            .expect_insert()
            .returning(|_| Err(Error::ProofsAlreadySpent));
        let swaps = Service {
            keys: keysrvc,
            proofs: proofrepo,
        };
        let r = swaps.burn(&inputs).await;
        assert!(r.is_err());
        let e = r.unwrap_err();
        assert!(matches!(e, Error::ActiveKeyset(_)));
    }

    #[tokio::test]
    async fn burn_spent_proofs() {
        let (mut keyinfo, keyset) = keys_test::generate_keyset();
        keyinfo.active = false;
        let inputs = utils::generate_proofs(&keyset, vec![Amount::from(8)].as_slice());

        let mut keysrvc = MockKeysService::new();
        keysrvc
            .expect_info()
            .returning(move |_| Ok(keyinfo.clone().into()));
        keysrvc.expect_verify_proof().returning(|_| Ok(()));
        let mut proofrepo = MockProofRepository::new();
        proofrepo
            .expect_insert()
            .returning(|_| Err(Error::ProofsAlreadySpent));
        let swaps = Service {
            keys: keysrvc,
            proofs: proofrepo,
        };
        let r = swaps.burn(&inputs).await;
        assert!(r.is_err());
        let e = r.unwrap_err();
        assert!(matches!(e, Error::ProofsAlreadySpent));
    }

    #[tokio::test]
    async fn burn_wrong_signatures() {
        let (mut keyinfo, keyset) = keys_test::generate_keyset();
        keyinfo.active = false;
        let mut inputs = utils::generate_proofs(&keyset, vec![Amount::from(8)].as_slice());
        inputs.get_mut(0).unwrap().c = keys_test::publics()[0];
        let mut keysrvc = MockKeysService::new();
        let proofrepo = MockProofRepository::new();
        let kid = keyset.id;
        keysrvc
            .expect_info()
            .with(eq(kid))
            .returning(move |_| Ok(keyinfo.clone().into()));
        keysrvc
            .expect_verify_proof()
            .returning(move |p| Err(Error::InvalidProof(p.secret.clone())));
        let swaps = Service {
            keys: keysrvc,
            proofs: proofrepo,
        };

        let r = swaps.burn(&inputs).await;
        assert!(r.is_err());
        let e = r.unwrap_err();
        assert!(matches!(e, Error::InvalidProof(_)));
    }

}
