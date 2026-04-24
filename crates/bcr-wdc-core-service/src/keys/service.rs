// ----- standard library imports
use std::{
    collections::HashMap,
    sync::atomic::{AtomicU64, Ordering},
};
// ----- extra library imports
use bcr_common::{
    cashu,
    cdk_common::mint::MintKeySetInfo,
    core::signature::{sign_ecash, verify_ecash_fingerprint, verify_ecash_proof, ProofFingerprint},
};
// ----- local imports
use crate::{
    error::{Error, Result},
    keys::{factory::Factory, ClowderClient},
    persistence::{KeysRepository, SignaturesRepository},
    TStamp,
};

// ----- end imports

#[derive(Default)]
pub struct ListFilters {
    pub unit: Option<cashu::CurrencyUnit>,
    pub min_expiration: Option<chrono::NaiveDate>,
    pub max_expiration: Option<chrono::NaiveDate>,
}

pub struct Service {
    pub keys: Box<dyn KeysRepository>,
    pub signatures: Box<dyn SignaturesRepository>,
    pub clowder: Box<dyn ClowderClient>,
    pub keygen: Factory,
    pub min_keyset_fees_ppk: AtomicU64,
}

impl Service {
    pub fn set_minimum_fees_ppk(&self, fees_ppk: u64) -> Result<()> {
        self.min_keyset_fees_ppk.store(fees_ppk, Ordering::Relaxed);
        Ok(())
    }

    pub async fn create(
        &self,
        unit: cashu::CurrencyUnit,
        now: TStamp,
        expiration: Option<TStamp>,
        fees_ppk: u64,
    ) -> Result<MintKeySetInfo> {
        let fees_ppk = std::cmp::max(fees_ppk, self.min_keyset_fees_ppk.load(Ordering::Relaxed));
        let entry = self.keygen.generate(unit, now, expiration, fees_ppk);
        let kinfo = entry.0.clone();
        let keyset = cashu::KeySet::from(entry.1.clone());
        self.keys.store(entry).await?;
        self.clowder.new_keyset(keyset).await?;
        Ok(kinfo)
    }

    pub async fn info(&self, kid: cashu::Id) -> Result<MintKeySetInfo> {
        self.keys
            .info(kid)
            .await?
            .ok_or(Error::ResourceNotFound(format!("keyset {}", kid)))
    }

    pub async fn keys(&self, kid: cashu::Id) -> Result<cashu::MintKeySet> {
        self.keys
            .keyset(kid)
            .await?
            .ok_or(Error::ResourceNotFound(format!("keyset {}", kid)))
    }

    pub async fn verify_proofs(&self, proofs: &[cashu::Proof]) -> Result<()> {
        let by_kid: HashMap<cashu::Id, Vec<&cashu::Proof>> =
            proofs.iter().fold(HashMap::new(), |mut kmap, p| {
                kmap.entry(p.keyset_id).or_default().push(p);
                kmap
            });
        for (kid, proofs) in by_kid {
            let keyset = self.keys(kid).await?;
            for proof in proofs {
                verify_ecash_proof(&keyset, proof)?;
            }
        }
        Ok(())
    }

    pub async fn verify_fingerprints(&self, fps: &[ProofFingerprint]) -> Result<()> {
        let by_kid: HashMap<cashu::Id, Vec<&ProofFingerprint>> =
            fps.iter().fold(HashMap::new(), |mut kmap, fp| {
                kmap.entry(fp.keyset_id).or_default().push(fp);
                kmap
            });
        for (kid, fps) in by_kid {
            let keyset = self.keys(kid).await?;
            for fp in fps {
                verify_ecash_fingerprint(&keyset, fp)?;
            }
        }
        Ok(())
    }

    pub async fn list_info(&self, filters: ListFilters) -> Result<Vec<MintKeySetInfo>> {
        let min_tstamp = filters
            .min_expiration
            .map(|d| d.and_time(chrono::NaiveTime::MIN).and_utc().timestamp() as u64);
        let max_tstamp = filters
            .max_expiration
            .map(|d| d.and_time(chrono::NaiveTime::MIN).and_utc().timestamp() as u64);
        self.keys
            .list_info(filters.unit, min_tstamp, max_tstamp)
            .await
    }

    pub async fn list_keyset(&self) -> Result<Vec<cashu::MintKeySet>> {
        self.keys.list_keyset().await
    }

    pub async fn deactivate(&self, kid: cashu::Id) -> Result<cashu::Id> {
        let mut info = self
            .keys
            .info(kid)
            .await?
            .ok_or(Error::ResourceNotFound(format!("keyset {}", kid)))?;
        info.active = false;
        self.keys.update_info(info.clone()).await?;
        self.clowder.keyset_deactivated(kid).await?;
        Ok(info.id)
    }

    pub async fn search_signature(
        &self,
        blind: &cashu::BlindedMessage,
    ) -> Result<Option<cashu::BlindSignature>> {
        self.signatures.load(blind).await
    }

    pub async fn sign_blinds(
        &self,
        mut blinds: impl Iterator<Item = &cashu::BlindedMessage>,
    ) -> Result<Vec<cashu::BlindSignature>> {
        let Some(first_b) = blinds.next() else {
            return Ok(Vec::new());
        };
        let mut keyset = self.keys(first_b.keyset_id).await?;
        let first_s = sign_ecash(&keyset, first_b)?;
        self.signatures
            .store(first_b.blinded_secret, first_s.clone())
            .await?;
        let mut signatures = vec![first_s];
        for blind in blinds {
            let cur_keyset = if blind.keyset_id == keyset.id {
                &keyset
            } else {
                keyset = self.keys(blind.keyset_id).await?;
                &keyset
            };
            let signature = sign_ecash(cur_keyset, blind)?;
            self.signatures
                .store(blind.blinded_secret, signature.clone())
                .await?;
            signatures.push(signature);
        }
        Ok(signatures)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        btc32::DerivationPath,
        keys::MockClowderClient,
        persistence::{MockKeysRepository, MockSignaturesRepository},
    };
    use bcr_wdc_utils::signatures::test_utils as signature_tests;
    use mockall::predicate::eq;
    use std::str::FromStr;

    fn seed() -> [u8; 64] {
        bip39::Mnemonic::from_str(
            "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
        )
        .unwrap().to_seed("")
    }

    #[tokio::test]
    async fn deactivate_ok() {
        let factory = Factory::new(&seed(), DerivationPath::default());
        let mut keys_repo = MockKeysRepository::new();
        let signatures_repo = MockSignaturesRepository::new();
        let mut clowder_cl = MockClowderClient::new();
        let (kinfo, _keyset) = bcr_common::core_tests::generate_random_ecash_keyset();
        let kid = kinfo.id;
        let mut updated_info = kinfo.clone();
        updated_info.active = false;
        keys_repo
            .expect_info()
            .times(1)
            .with(eq(kid))
            .returning(move |_| Ok(Some(kinfo.clone())));
        keys_repo
            .expect_update_info()
            .times(1)
            .with(eq(updated_info.clone()))
            .returning(|_| Ok(()));
        clowder_cl
            .expect_keyset_deactivated()
            .times(1)
            .with(eq(kid))
            .returning(|_| Ok(()));
        let service = Service {
            keys: Box::new(keys_repo),
            signatures: Box::new(signatures_repo),
            keygen: factory,
            clowder: Box::new(clowder_cl),
            min_keyset_fees_ppk: Default::default(),
        };
        let deactivated = service.deactivate(kid).await.unwrap();
        assert_eq!(deactivated, kid);
    }

    #[tokio::test]
    async fn deactivate_no_keysetid() {
        let factory = Factory::new(&seed(), DerivationPath::default());
        let mut keys_repo = MockKeysRepository::new();
        let signatures_repo = MockSignaturesRepository::new();
        let clowder_cl = MockClowderClient::new();
        let kid = bcr_common::core_tests::generate_random_ecash_keyset().0.id;
        keys_repo
            .expect_info()
            .times(1)
            .with(eq(kid))
            .returning(|_| Ok(None));
        let service = Service {
            keys: Box::new(keys_repo),
            signatures: Box::new(signatures_repo),
            keygen: factory,
            clowder: Box::new(clowder_cl),
            min_keyset_fees_ppk: Default::default(),
        };
        let err = service.deactivate(kid).await.unwrap_err();
        assert!(matches!(err, Error::ResourceNotFound(_)));
    }

    #[tokio::test]
    async fn sign_blinds() {
        let factory = Factory::new(&seed(), DerivationPath::default());
        let mut keys_repo = MockKeysRepository::new();
        let mut signatures_repo = MockSignaturesRepository::new();
        let clowder_cl = MockClowderClient::new();
        let (kinfo, keyset) = bcr_common::core_tests::generate_random_ecash_keyset();
        let amounts = vec![
            cashu::Amount::from(64),
            cashu::Amount::from(512),
            cashu::Amount::from(32),
        ];
        keys_repo
            .expect_keyset()
            .times(1)
            .with(eq(kinfo.id))
            .returning(move |_| Ok(Some(keyset.clone())));
        signatures_repo
            .expect_store()
            .times(amounts.len())
            .returning(|_, _| Ok(()));
        let service = Service {
            keys: Box::new(keys_repo),
            signatures: Box::new(signatures_repo),
            keygen: factory,
            clowder: Box::new(clowder_cl),
            min_keyset_fees_ppk: Default::default(),
        };
        let blinds = signature_tests::generate_blinds(kinfo.id, &amounts)
            .into_iter()
            .map(|(b, _, _)| b)
            .collect::<Vec<_>>();
        let signatures = service.sign_blinds(blinds.iter()).await.unwrap();
        assert_eq!(signatures.len(), blinds.len());
        assert_eq!(signatures[0].amount, blinds[0].amount);
        assert_eq!(signatures[1].amount, blinds[1].amount);
        assert_eq!(signatures[2].amount, blinds[2].amount);
    }

    #[tokio::test]
    async fn sign_blinds_different_keysets() {
        let factory = Factory::new(&seed(), DerivationPath::default());
        let mut keys_repo = MockKeysRepository::new();
        let mut signatures_repo = MockSignaturesRepository::new();
        let clowder_cl = MockClowderClient::new();
        let (kinfo1, keyset1) = bcr_common::core_tests::generate_random_ecash_keyset();
        let (kinfo2, keyset2) = bcr_common::core_tests::generate_random_ecash_keyset();
        keys_repo
            .expect_keyset()
            .times(1)
            .with(eq(kinfo1.id))
            .returning(move |_| Ok(Some(keyset1.clone())));
        keys_repo
            .expect_keyset()
            .times(1)
            .with(eq(kinfo2.id))
            .returning(move |_| Ok(Some(keyset2.clone())));
        signatures_repo
            .expect_store()
            .times(4)
            .returning(|_, _| Ok(()));
        let service = Service {
            keys: Box::new(keys_repo),
            signatures: Box::new(signatures_repo),
            keygen: factory,
            clowder: Box::new(clowder_cl),
            min_keyset_fees_ppk: Default::default(),
        };
        let amounts = vec![cashu::Amount::from(64), cashu::Amount::from(32)];
        let blinds1 = signature_tests::generate_blinds(kinfo1.id, &amounts)
            .into_iter()
            .map(|(b, _, _)| b);
        let blinds2 = signature_tests::generate_blinds(kinfo2.id, &amounts)
            .into_iter()
            .map(|(b, _, _)| b);
        let blinds = blinds1.chain(blinds2).collect::<Vec<_>>();
        let result = service.sign_blinds(blinds.iter()).await.unwrap();
        assert_eq!(result.len(), 4);
        assert_eq!(result[0].amount, amounts[0]);
        assert_eq!(result[1].amount, amounts[1]);
        assert_eq!(result[2].amount, amounts[0]);
        assert_eq!(result[3].amount, amounts[1]);
    }
}
