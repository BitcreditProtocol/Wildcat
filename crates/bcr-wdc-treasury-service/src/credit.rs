// ----- standard library imports
// ----- extra library imports
use async_trait::async_trait;
use bitcoin::bip32 as btc32;
use cashu::nut02 as cdk02;
use cashu::nuts::nut00 as cdk00;
use cashu::Amount;
use uuid::Uuid;
// ----- local imports
use crate::error::{Error, Result};

#[cfg_attr(test, mockall::automock)]
#[async_trait()]
pub trait Repository: Send + Sync {
    async fn next_counter(&self, kid: cdk02::Id) -> Result<u32>;
    async fn increment_counter(&self, kid: cdk02::Id, inc: u32) -> Result<()>;
    async fn store_secrets(&self, rid: Uuid, premint: cdk00::PreMintSecrets) -> Result<()>;
    async fn store_signatures(
        &self,
        rid: Uuid,
        signatures: Vec<cdk00::BlindSignature>,
    ) -> Result<()>;
}

#[derive(Clone)]
pub struct Service<Repo> {
    pub xpriv: btc32::Xpriv,
    pub repo: Repo,
}

impl<Repo> Service<Repo>
where
    Repo: Repository,
{
    pub async fn generate_blinds(
        &self,
        kid: cdk02::Id,
        total: Amount,
    ) -> Result<(Uuid, Vec<cdk00::BlindedMessage>)> {
        let counter = self.repo.next_counter(kid).await?;
        let premint = cdk00::PreMintSecrets::from_xpriv(
            kid,
            counter,
            self.xpriv,
            total,
            &cashu::amount::SplitTarget::default(),
        )
        .map_err(Error::CDK13)?;
        let request_id = Uuid::new_v4();
        let blinds = premint.blinded_messages();
        self.repo.store_secrets(request_id, premint).await?;
        self.repo
            .increment_counter(kid, blinds.len() as u32)
            .await?;
        Ok((request_id, blinds))
    }

    pub async fn store_signatures(
        &self,
        rid: Uuid,
        signatures: Vec<cdk00::BlindSignature>,
        _expiration: chrono::DateTime<chrono::Utc>,
    ) -> Result<()> {
        self.repo.store_signatures(rid, signatures).await?;
        Ok(())
        //TODO: schedule check operation after expiration
    }
}
