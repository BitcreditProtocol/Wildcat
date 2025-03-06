// ----- standard library imports
// ----- extra library imports
use async_trait::async_trait;
use cashu::mint::MintKeySetInfo;
use cashu::nuts::nut00 as cdk00;
use cashu::nuts::nut02 as cdk02;
// ----- local imports
use crate::error::{Error, Result};

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait KeysRepository {
    async fn info(&self, id: &cdk02::Id) -> Result<Option<MintKeySetInfo>>;
    async fn keyset(&self, id: &cdk02::Id) -> Result<Option<cdk02::MintKeySet>>;
}

#[derive(Clone)]
pub struct Service<KeysRepo> {
    pub keys: KeysRepo,
}

impl<KeysRepo> Service<KeysRepo>
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
        bcr_wdc_keys::verify_with_keys(&keyset, proof)?;
        Ok(())
    }
}
