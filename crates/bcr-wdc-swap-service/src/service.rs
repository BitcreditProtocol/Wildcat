// ----- standard library imports
// ----- extra library imports
use async_trait::async_trait;
use cashu::Amount;
use cashu::dhke as cdk_dhke;
use cashu::mint::MintKeySetInfo;
use cashu::nuts::nut00 as cdk00;
use cashu::nuts::nut02 as cdk02;
use itertools::Itertools;
// ----- local imports
use crate::error::{Error, Result};

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait KeysService: Send + Sync {
    async fn keyset(&self, id: &cdk02::Id) -> Result<Option<cdk02::MintKeySet>>;
    async fn info(&self, id: &cdk02::Id) -> Result<Option<MintKeySetInfo>>;
}

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait ProofRepository: Send + Sync {
    /// WARNING: this method should do strict insert.
    /// i.e. it should fail if any of the proofs is already present in the DB
    /// in case of failure, the DB should be in the same state as before the call
    async fn insert(&self, tokens: &[cdk00::Proof]) -> Result<()>;
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
        for proof in proofs {
            let id = proof.keyset_id;
            let keyset = self
                .keys
                .keyset(&id)
                .await?
                .ok_or_else(|| Error::UnknownKeyset(id))?;
            let key = keyset
                .keys
                .get(&proof.amount)
                .ok_or_else(|| Error::UnknownAmountForKeyset(id, proof.amount))?;
            cdk_dhke::verify_message(&key.secret_key, proof.c, proof.secret.as_bytes())
                .map_err(Error::CdkDhke)?;
        }
        Ok(())
    }
    async fn verify_keys_are_active(&self, keys: &[cdk02::Id]) -> Result<()> {
        for id in keys {
            let info = self.keys.info(id).await?;
            if let Some(info) = info {
                if !info.active {
                    return Err(Error::InactiveKeyset(*id));
                }
            } else {
                return Err(Error::UnknownKeyset(*id));
            }
        }
        Ok(())
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
        if inputs.is_empty() || outputs.is_empty() {
            return Err(Error::EmptyInputsOrOutputs);
        }
        // 1. no zero amounts in inputs or outputs
        let zero_outputs = outputs.iter().any(|output| output.amount == Amount::ZERO);
        let zero_inputs = inputs.iter().any(|output| output.amount == Amount::ZERO);
        if zero_outputs || zero_inputs {
            return Err(Error::ZeroAmount);
        }
        // 2. inputs and outputs grouped by keyset ID have equal amounts
        let ids: Vec<_> = inputs.iter().map(|p| p.keyset_id).unique().collect();
        for id in &ids {
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
        self.verify_keys_are_active(&ids).await?;
        // 2. verify proofs signatures
        self.verify_proofs_signatures(inputs).await?;
        let mut signatures = Vec::with_capacity(outputs.len());
        for output in outputs {
            let keys = self
                .keys
                .keyset(&output.keyset_id)
                .await?
                .ok_or(Error::UnknownKeyset(output.keyset_id))?;
            let keypair = keys
                .keys
                .get(&output.amount)
                .ok_or(Error::UnknownAmountForKeyset(keys.id, output.amount))?;
            let c = cdk_dhke::sign_message(&keypair.secret_key, &output.blinded_secret)?;
            let signature = cdk00::BlindSignature::new(
                output.amount,
                c,
                keys.id,
                &output.blinded_secret,
                keypair.secret_key.clone(),
            )?;
            signatures.push(signature);
        }
        self.proofs.insert(inputs).await?;
        Ok(signatures)
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
        let outputs: Vec<_> = utils::generate_blinds(&keyset, vec![Amount::from(8)].as_slice())
            .into_iter()
            .map(|a| a.0)
            .collect();

        let mut keysrvc = MockKeysService::new();
        keysrvc
            .expect_info()
            .returning(move |_| Ok(Some(keyinfo.clone())));
        keysrvc
            .expect_keyset()
            .returning(move |_| Ok(Some(keyset.clone())));
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
        keysrvc.expect_info().with(eq(kid)).returning(|_| Ok(None));
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
            .returning(move |_| Ok(Some(keyinfo.clone())));
        keysrvc
            .expect_keyset()
            .with(eq(kid))
            .returning(move |_| Ok(Some(keyset.clone())));
        let swaps = Service {
            keys: keysrvc,
            proofs: proofrepo,
        };

        let r = swaps.swap(&inputs, &outputs).await;
        assert!(r.is_err());
        let e = r.unwrap_err();
        assert!(matches!(e, Error::CdkDhke(_)));
    }

    #[tokio::test]
    async fn test_swap_unmatched_amounts() {
        let (keyinfo, keyset) = keys_test::generate_keyset();
        let inputs = utils::generate_proofs(&keyset, vec![Amount::from(8)].as_slice());
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
            .returning(move |_| Ok(Some(keyinfo.clone())));
        keysrvc
            .expect_keyset()
            .with(eq(kid))
            .returning(move |_| Ok(Some(keyset.clone())));
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
        let inputs = utils::generate_proofs(&keyset, vec![Amount::from(8)].as_slice());
        let outputs: Vec<_> =
            utils::generate_blinds(&keyset, vec![Amount::from(4), Amount::from(4)].as_slice())
                .into_iter()
                .map(|a| a.0)
                .collect();
        let mut keysrvc = MockKeysService::new();
        let mut proofrepo = MockProofRepository::new();
        let kid = keyset.id;
        let keyset_clone = keyset.clone();
        keysrvc
            .expect_info()
            .with(eq(kid))
            .returning(move |_| Ok(Some(keyinfo.clone())));
        keysrvc
            .expect_keyset()
            .with(eq(kid))
            .returning(move |_| Ok(Some(keyset_clone.clone())));
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
        let outputs: Vec<_> = utils::generate_blinds(&keyset, vec![Amount::from(8)].as_slice())
            .into_iter()
            .map(|a| a.0)
            .collect();
        let mut keysrvc = MockKeysService::new();
        let mut proofrepo = MockProofRepository::new();
        let kid = keyset.id;
        let keyset_clone = keyset.clone();
        keysrvc
            .expect_info()
            .with(eq(kid))
            .returning(move |_| Ok(Some(keyinfo.clone())));
        keysrvc
            .expect_keyset()
            .with(eq(kid))
            .returning(move |_| Ok(Some(keyset_clone.clone())));
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
}
