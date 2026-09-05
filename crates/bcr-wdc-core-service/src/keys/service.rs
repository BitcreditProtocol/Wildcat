// ----- standard library imports
use std::{
    collections::HashMap,
    sync::atomic::{AtomicU64, Ordering},
    sync::Arc,
};
// ----- extra library imports
use bcr_common::{
    cashu,
    client::admin::core::RNFError,
    core::signature::{sign_ecash, verify_ecash_fingerprint, verify_ecash_proof, ProofFingerprint},
    ecash,
};
// ----- local imports
use crate::{
    error::{Error, Result},
    keys::{factory::Factory, ClowderClient},
    persistence::Repository,
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
    pub repository: Arc<dyn Repository>,
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
    ) -> Result<ecash::MintKeySetInfo> {
        let fees_ppk = std::cmp::max(fees_ppk, self.min_keyset_fees_ppk.load(Ordering::Relaxed));
        let entry = self.keygen.generate(unit, now, expiration, fees_ppk);
        let (kinfo, keyset) = bcr_wdc_utils::keys::from_entry(entry.clone());
        self.clowder.new_keyset(ecash::KeySet::from(keyset)).await?;
        self.repository.keys_store(entry).await?;
        Ok(kinfo)
    }

    pub async fn info(&self, kid: cashu::Id) -> Result<ecash::MintKeySetInfo> {
        self.repository
            .keys_info(kid)
            .await?
            .ok_or(Error::ResourceNotFound(RNFError::KeysetId(kid)))
    }

    pub async fn keys(&self, kid: cashu::Id) -> Result<ecash::MintKeySet> {
        self.repository
            .keys_load(kid)
            .await?
            .ok_or(Error::ResourceNotFound(RNFError::KeysetId(kid)))
    }

    pub async fn verify_proofs(&self, proofs: &[cashu::Proof]) -> Result<()> {
        let by_kid: HashMap<cashu::Id, Vec<&cashu::Proof>> =
            proofs.iter().fold(HashMap::new(), |mut kmap, p| {
                kmap.entry(p.keyset_id).or_default().push(p);
                kmap
            });
        for (kid, proofs) in by_kid {
            let keyset = self.keys(kid).await?.into();
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
            let keyset = self.keys(kid).await?.into();
            for fp in fps {
                verify_ecash_fingerprint(&keyset, fp)?;
            }
        }
        Ok(())
    }

    pub async fn list_info(&self, filters: ListFilters) -> Result<Vec<ecash::MintKeySetInfo>> {
        let min_tstamp = filters
            .min_expiration
            .map(|d| d.and_time(chrono::NaiveTime::MIN).and_utc().timestamp() as u64);
        let max_tstamp = filters
            .max_expiration
            .map(|d| d.and_time(chrono::NaiveTime::MIN).and_utc().timestamp() as u64);
        self.repository
            .keys_list_info(filters.unit, min_tstamp, max_tstamp)
            .await
    }

    pub async fn search_signature(
        &self,
        blind: &cashu::BlindedMessage,
    ) -> Result<Option<cashu::BlindSignature>> {
        self.repository.signature_load(blind).await
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
        self.repository
            .signature_store(first_b.blinded_secret, first_s.clone())
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
            self.repository
                .signature_store(blind.blinded_secret, signature.clone())
                .await?;
            signatures.push(signature);
        }
        Ok(signatures)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{keys::MockClowderClient, persistence::MockRepository};
    use bcr_wdc_utils::signatures::test_utils as signature_tests;
    use bitcoin::bip32::DerivationPath;
    use mockall::predicate::eq;
    use std::str::FromStr;

    fn seed() -> [u8; 64] {
        bip39::Mnemonic::from_str(
            "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
        )
        .unwrap().to_seed("")
    }

    #[tokio::test]
    async fn sign_blinds() {
        let factory = Factory::new(&seed(), DerivationPath::default());
        let mut repository = MockRepository::new();
        let clowder_cl = MockClowderClient::new();
        let (kinfo, keyset) = bcr_common::core_tests::generate_random_ecash_keyset();
        let amounts = vec![
            cashu::Amount::from(64),
            cashu::Amount::from(512),
            cashu::Amount::from(32),
        ];
        repository
            .expect_keys_load()
            .times(1)
            .with(eq(kinfo.id))
            .returning(move |_| Ok(Some(keyset.clone().into())));
        repository
            .expect_signature_store()
            .times(amounts.len())
            .returning(|_, _| Ok(()));
        let service = Service {
            repository: Arc::new(repository),
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
        let mut repository = MockRepository::new();
        let clowder_cl = MockClowderClient::new();
        let (kinfo1, keyset1) = bcr_common::core_tests::generate_random_ecash_keyset();
        let (kinfo2, keyset2) = bcr_common::core_tests::generate_random_ecash_keyset();
        repository
            .expect_keys_load()
            .times(1)
            .with(eq(kinfo1.id))
            .returning(move |_| Ok(Some(keyset1.clone().into())));
        repository
            .expect_keys_load()
            .times(1)
            .with(eq(kinfo2.id))
            .returning(move |_| Ok(Some(keyset2.clone().into())));
        repository
            .expect_signature_store()
            .times(4)
            .returning(|_, _| Ok(()));
        let service = Service {
            repository: Arc::new(repository),
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
