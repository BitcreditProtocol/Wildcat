// ----- standard library imports
// ----- extra library imports
use async_trait::async_trait;
use bcr_wdc_utils::keys as keys_utils;
use cashu::{nut00 as cdk00, nut01 as cdk01, nut02 as cdk02, Amount};
use cdk_common::mint::MintKeySetInfo;
use itertools::Itertools;
// ----- local imports
use crate::error::{Error, Result};
use crate::factory::Factory;
use crate::TStamp;

// ----- end imports

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[cfg_attr(test, derive(PartialEq))]
pub struct MintCondition {
    pub target: Amount,
    pub pub_key: cdk01::PublicKey,
    pub is_minted: bool,
}

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait KeysRepository {
    async fn info(&self, id: &cdk02::Id) -> Result<Option<MintKeySetInfo>>;
    async fn list_info(&self) -> Result<Vec<MintKeySetInfo>>;
    async fn keyset(&self, id: &cdk02::Id) -> Result<Option<cdk02::MintKeySet>>;
    async fn list_keyset(&self) -> Result<Vec<cdk02::MintKeySet>>;
    async fn condition(&self, id: &cdk02::Id) -> Result<Option<MintCondition>>;
    async fn store(&self, keys: keys_utils::KeysetEntry, condition: MintCondition) -> Result<()>;
    // WARNING: it must fail if the keyset is already minted
    async fn mark_as_minted(&self, id: &cdk02::Id) -> Result<()>;
}

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait QuoteKeysRepository {
    async fn entry(&self, qid: &uuid::Uuid) -> Result<Option<keys_utils::KeysetEntry>>;
    async fn info(&self, qid: &uuid::Uuid) -> Result<Option<MintKeySetInfo>>;
    async fn keyset(&self, qid: &uuid::Uuid) -> Result<Option<cdk02::MintKeySet>>;
    async fn condition(&self, qid: &uuid::Uuid) -> Result<Option<MintCondition>>;
    async fn store(
        &self,
        qid: &uuid::Uuid,
        keys: keys_utils::KeysetEntry,
        condition: MintCondition,
    ) -> Result<()>;
}

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait SignaturesRepository {
    async fn store(
        &self,
        blind: &cdk00::BlindedMessage,
        signature: &cdk00::BlindSignature,
    ) -> Result<()>;
    async fn load(&self, blind: &cdk00::BlindedMessage) -> Result<Option<cdk00::BlindSignature>>;
}

#[derive(Clone)]
pub struct Service<QuoteKeysRepo, KeysRepo, SignsRepo> {
    pub quote_keys: QuoteKeysRepo,
    pub keys: KeysRepo,
    pub signatures: SignsRepo,
    pub keygen: Factory,
}

impl<QuoteKeysRepo, KeysRepo, SignsRepo> Service<QuoteKeysRepo, KeysRepo, SignsRepo>
where
    KeysRepo: KeysRepository,
{
    pub async fn info(&self, kid: cdk02::Id) -> Result<MintKeySetInfo> {
        self.keys.info(&kid).await?.ok_or(Error::UnknownKeyset(kid))
    }

    pub async fn keys(&self, kid: cdk02::Id) -> Result<cdk02::MintKeySet> {
        self.keys
            .keyset(&kid)
            .await?
            .ok_or(Error::UnknownKeyset(kid))
    }

    pub async fn verify_proof(&self, proof: cdk00::Proof) -> Result<()> {
        let keyset = self.keys(proof.keyset_id).await?;
        keys_utils::verify_with_keys(&keyset, &proof)?;
        Ok(())
    }

    pub async fn list_info(&self) -> Result<Vec<MintKeySetInfo>> {
        self.keys.list_info().await
    }

    pub async fn list_keyset(&self) -> Result<Vec<cdk02::MintKeySet>> {
        self.keys.list_keyset().await
    }

    pub async fn authorized_public_key_to_mint(&self, kid: cdk02::Id) -> Result<cdk01::PublicKey> {
        let condition = self
            .keys
            .condition(&kid)
            .await?
            .ok_or(Error::UnknownKeyset(kid))?;
        Ok(condition.pub_key)
    }
}

impl<QuoteKeysRepo, KeysRepo, SignsRepo> Service<QuoteKeysRepo, KeysRepo, SignsRepo>
where
    QuoteKeysRepo: QuoteKeysRepository,
{
    pub async fn pre_sign(
        &self,
        qid: uuid::Uuid,
        msg: &cdk00::BlindedMessage,
    ) -> Result<cdk00::BlindSignature> {
        let keyset = self
            .quote_keys
            .keyset(&qid)
            .await?
            .ok_or(Error::UnknownKeysetFromId(qid))?;
        let signature = keys_utils::sign_with_keys(&keyset, msg)?;
        Ok(signature)
    }

    pub async fn generate_keyset(
        &self,
        qid: uuid::Uuid,
        target: Amount,
        pub_key: cdk01::PublicKey,
        expire: TStamp,
    ) -> Result<cdk02::Id> {
        let mint_condition = MintCondition {
            target,
            pub_key,
            is_minted: false,
        };
        let info = self.quote_keys.info(&qid).await?;
        let id = match info {
            Some(info) => {
                let condition = self
                    .quote_keys
                    .condition(&qid)
                    .await?
                    .expect("info with not condition");
                if condition.pub_key != mint_condition.pub_key
                    || condition.target != mint_condition.target
                {
                    return Err(Error::InvalidGenerateRequest(qid));
                }
                info.id
            }
            None => {
                let keys_entry = self.keygen.generate(qid, expire);
                let id = keys_entry.1.id;
                self.quote_keys
                    .store(&qid, keys_entry, mint_condition)
                    .await?;
                id
            }
        };
        Ok(id)
    }
}

impl<QuoteKeysRepo, KeysRepo, SignsRepo> Service<QuoteKeysRepo, KeysRepo, SignsRepo>
where
    SignsRepo: SignaturesRepository,
{
    pub async fn search_signature(
        &self,
        blind: &cdk00::BlindedMessage,
    ) -> Result<Option<cdk00::BlindSignature>> {
        self.signatures.load(blind).await
    }
}

impl<QuoteKeysRepo, KeysRepo, SignsRepo> Service<QuoteKeysRepo, KeysRepo, SignsRepo>
where
    KeysRepo: KeysRepository,
    SignsRepo: SignaturesRepository,
{
    pub async fn sign_blind(&self, blind: &cdk00::BlindedMessage) -> Result<cdk00::BlindSignature> {
        let keyset = self.keys(blind.keyset_id).await?;
        let signature = keys_utils::sign_with_keys(&keyset, blind)?;
        self.signatures.store(blind, &signature).await?;
        Ok(signature)
    }

    pub async fn mint(
        &self,
        _qid: uuid::Uuid,
        outputs: Vec<cdk00::BlindedMessage>,
    ) -> Result<Vec<cdk00::BlindSignature>> {
        // basic checks
        bcr_wdc_utils::signatures::basic_blinds_checks(&outputs)
            .map_err(|e| Error::InvalidMintRequest(e.to_string()))?;
        //  check if the ids of the outputs are all the same
        let unique_ids: Vec<_> = outputs.iter().map(|p| p.keyset_id).unique().collect();
        if unique_ids.len() != 1 {
            return Err(Error::InvalidMintRequest(String::from(
                "multiple keyset IDs",
            )));
        }
        let kid = unique_ids[0];

        let MintCondition {
            target, is_minted, ..
        } = self
            .keys
            .condition(&kid)
            .await?
            .ok_or(Error::UnknownKeyset(kid))?;
        //  check if the keyset id has been minted already
        if is_minted {
            return Err(Error::InvalidMintRequest(String::from(
                "keyset already minted",
            )));
        }
        let blinds_sum = outputs.iter().fold(Amount::ZERO, |acc, b| acc + b.amount);
        if blinds_sum != target {
            return Err(Error::InvalidMintRequest(String::from("invalid amount")));
        }

        let mut signatures = Vec::with_capacity(outputs.len());
        for blind in &outputs {
            let signature = self.sign_blind(blind).await?;
            self.signatures.store(blind, &signature).await?;
            signatures.push(signature);
        }
        self.keys.mark_as_minted(&kid).await?;
        Ok(signatures)
    }
}

impl<QuoteKeysRepo, KeysRepo, SignsRepo> Service<QuoteKeysRepo, KeysRepo, SignsRepo>
where
    QuoteKeysRepo: QuoteKeysRepository,
    KeysRepo: KeysRepository,
{
    pub async fn activate(&self, qid: &uuid::Uuid) -> Result<()> {
        let mut entry = self
            .quote_keys
            .entry(qid)
            .await?
            .ok_or(Error::UnknownKeysetFromId(*qid))?;
        entry.0.active = true;
        let condition = self
            .quote_keys
            .condition(qid)
            .await?
            .ok_or(Error::UnknownKeysetFromId(*qid))?;

        self.keys.store(entry, condition).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::btc32::DerivationPath;
    use crate::test_utils::{
        TestKeysRepository, TestKeysService, TestQuoteKeysRepository, TestSignaturesRepository,
    };
    use bcr_wdc_utils::signatures::test_utils::generate_blinds;
    use std::str::FromStr;
    use uuid::Uuid;

    // Helper function to set up test service
    async fn setup_test_service() -> (TestKeysService, cdk02::Id, Uuid) {
        let seed = bip39::Mnemonic::from_str("abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about")
            .unwrap()
            .to_seed("");
        let maturity = chrono::DateTime::parse_from_rfc3339("2029-01-01T00:00:00Z")
            .unwrap()
            .to_utc();
        let factory = Factory::new(&seed, DerivationPath::default());

        let service = TestKeysService {
            quote_keys: TestQuoteKeysRepository::default(),
            keys: TestKeysRepository::default(),
            signatures: TestSignaturesRepository::default(),
            keygen: factory,
        };

        let kp = bcr_wdc_utils::keys::test_utils::generate_random_keypair();
        let pub_key = kp.public_key();
        let target = Amount::from(192);

        let qid = Uuid::new_v4();
        let kid = service
            .generate_keyset(qid, target, pub_key.into(), maturity)
            .await
            .unwrap();

        service.activate(&qid).await.unwrap();

        (service, kid, qid)
    }

    #[tokio::test]
    async fn test_mint_success() {
        let (service, kid, qid) = setup_test_service().await;

        let outputs = generate_blinds(kid, &[Amount::from(128), Amount::from(64)]);
        let blinds = outputs.iter().map(|o| o.0.clone()).collect::<Vec<_>>();

        let signatures = service.mint(qid, blinds).await.unwrap();

        assert_eq!(signatures.len(), 2, "Should have 2 signatures");
        assert_eq!(
            signatures.iter().map(|s| u64::from(s.amount)).sum::<u64>(),
            192,
            "Total amount should match target"
        );
    }

    #[tokio::test]
    async fn test_mint_more() {
        let (service, kid, qid) = setup_test_service().await;
        let outputs = generate_blinds(kid, &[Amount::from(128), Amount::from(64), Amount::from(1)]);
        let blinds = outputs.iter().map(|o| o.0.clone()).collect::<Vec<_>>();

        assert!(
            service.mint(qid, blinds).await.is_err(),
            "Mint should fail with invalid amount"
        );
    }

    #[tokio::test]
    async fn test_mint_less() {
        let (service, kid, qid) = setup_test_service().await;
        let outputs = generate_blinds(kid, &[Amount::from(128), Amount::from(32)]);
        let blinds = outputs.iter().map(|o| o.0.clone()).collect::<Vec<_>>();

        assert!(
            service.mint(qid, blinds).await.is_err(),
            "Mint should fail with invalid amount"
        );
    }
}
