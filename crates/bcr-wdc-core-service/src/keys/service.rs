// ----- standard library imports
// ----- extra library imports
use bcr_common::{cashu, cdk_common::mint::MintKeySetInfo, core};
// ----- local imports
use crate::{
    error::{Error, Result},
    keys::{factory::Factory, ClowderClient},
    persistence::{KeysRepository, SignaturesRepository},
    TStamp,
};

// ----- end imports

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
}

impl Service {
    pub async fn create(
        &self,
        unit: cashu::CurrencyUnit,
        now: TStamp,
        expiration: Option<TStamp>,
        fees_ppk: u64,
    ) -> Result<MintKeySetInfo> {
        let entry = self.keygen.generate(unit, now, expiration, fees_ppk);
        let kinfo = entry.0.clone();
        self.keys.store(entry).await?;
        Ok(kinfo)
    }

    pub async fn info(&self, kid: cashu::Id) -> Result<MintKeySetInfo> {
        self.keys.info(kid).await?.ok_or(Error::KeysetNotFound(kid))
    }

    pub async fn keys(&self, kid: cashu::Id) -> Result<cashu::MintKeySet> {
        self.keys
            .keyset(kid)
            .await?
            .ok_or(Error::KeysetNotFound(kid))
    }

    pub async fn verify_proof(&self, proof: cashu::Proof) -> Result<()> {
        let keyset = self.keys(proof.keyset_id).await?;
        core::signature::verify_ecash_proof(&keyset, &proof)?;
        Ok(())
    }

    pub async fn verify_fingerprint(&self, fp: core::signature::ProofFingerprint) -> Result<()> {
        let keyset = self.keys(fp.keyset_id).await?;
        core::signature::verify_ecash_fingerprint(&keyset, &fp)?;
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
            .ok_or(Error::KeysetNotFound(kid))?;
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

    pub async fn sign_blind(&self, blind: &cashu::BlindedMessage) -> Result<cashu::BlindSignature> {
        let keyset = self.keys(blind.keyset_id).await?;
        let signature = core::signature::sign_ecash(&keyset, blind)?;
        self.signatures
            .store(blind.blinded_secret, signature.clone())
            .await?;
        Ok(signature)
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
        };
        let err = service.deactivate(kid).await.unwrap_err();
        assert!(matches!(err, Error::KeysetNotFound(id) if id == kid));
    }
}
