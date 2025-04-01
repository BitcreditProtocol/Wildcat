// ----- standard library imports
// ----- extra library imports
use async_trait::async_trait;
use bcr_wdc_keys::KeysetEntry;
use cashu::nuts::nut00 as cdk00;
use cashu::nuts::nut02 as cdk02;
use cdk_common::mint::MintKeySetInfo;
// ----- local imports
use crate::error::{Error, Result};
use crate::factory::Factory;
use crate::TStamp;

// ----- end imports

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait KeysRepository {
    async fn info(&self, id: &cdk02::Id) -> Result<Option<MintKeySetInfo>>;
    async fn keyset(&self, id: &cdk02::Id) -> Result<Option<cdk02::MintKeySet>>;
    async fn store(&self, keys: KeysetEntry) -> Result<()>;
}

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait QuoteKeysRepository {
    async fn info(&self, kid: &cdk02::Id, qid: &uuid::Uuid) -> Result<Option<MintKeySetInfo>>;
    async fn keyset(&self, kid: &cdk02::Id, qid: &uuid::Uuid) -> Result<Option<cdk02::MintKeySet>>;
    async fn store(&self, qid: &uuid::Uuid, keys: KeysetEntry) -> Result<()>;
}

#[derive(Clone)]
pub struct Service<QuoteKeysRepo, KeysRepo> {
    pub quote_keys: QuoteKeysRepo,
    pub keys: KeysRepo,
    pub keygen: Factory,
}

impl<QuoteKeysRepo, KeysRepo> Service<QuoteKeysRepo, KeysRepo>
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

    pub async fn sign_blind(&self, blind: cdk00::BlindedMessage) -> Result<cdk00::BlindSignature> {
        let keyset = self.keys(blind.keyset_id).await?;
        let signature = bcr_wdc_keys::sign_with_keys(&keyset, &blind)?;
        Ok(signature)
    }

    pub async fn verify_proof(&self, proof: cdk00::Proof) -> Result<()> {
        let keyset = self.keys(proof.keyset_id).await?;
        bcr_wdc_keys::verify_with_keys(&keyset, &proof)?;
        Ok(())
    }
}

impl<QuoteKeysRepo, KeysRepo> Service<QuoteKeysRepo, KeysRepo>
where
    QuoteKeysRepo: QuoteKeysRepository,
{
    pub async fn pre_sign(
        &self,
        kid: cdk02::Id,
        qid: uuid::Uuid,
        expire: TStamp,
        msg: &cdk00::BlindedMessage,
    ) -> Result<cdk00::BlindSignature> {
        let keyset = self.quote_keys.keyset(&kid, &qid).await?;
        let keyset = match keyset {
            Some(keyset) => keyset,
            None => {
                let keys = self.keygen.generate(kid, qid, expire);
                let keyset = keys.1.clone();
                self.quote_keys.store(&qid, keys).await?;
                keyset
            }
        };
        let signature = bcr_wdc_keys::sign_with_keys(&keyset, msg)?;
        Ok(signature)
    }
}

impl<QuoteKeysRepo, KeysRepo> Service<QuoteKeysRepo, KeysRepo>
where
    QuoteKeysRepo: QuoteKeysRepository,
    KeysRepo: KeysRepository,
{
    pub async fn activate(&self, kid: &cdk02::Id, qid: &uuid::Uuid) -> Result<()> {
        let (info, keyset) = futures::join!(
            self.quote_keys.info(kid, qid),
            self.quote_keys.keyset(kid, qid),
        );
        let mut info = info?.ok_or(Error::UnknownKeyset(*kid))?;
        let keyset = keyset?.ok_or(Error::UnknownKeyset(*kid))?;
        info.active = true;

        self.keys.store((info, keyset)).await?;
        Ok(())
    }
}
