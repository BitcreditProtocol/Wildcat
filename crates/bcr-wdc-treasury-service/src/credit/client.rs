// ----- standard library imports
// ----- extra library imports
use async_trait::async_trait;
use bcr_common::{
    cashu,
    client::core::{Client as CoreClient, Error as CoreError},
    core,
    wire::clowder as wire_clowder,
};
use clwdr_client::ClowderNatsClient;
use uuid::Uuid;
// ----- local imports
use crate::{
    credit,
    error::{Error, Result},
};

// ----- end imports

pub struct DummyClwdr {}
#[async_trait]
impl credit::ClowderClient for DummyClwdr {
    async fn minting_ebill(
        &self,
        _keyset_id: cashu::Id,
        _quote_id: Uuid,
        _amount: cashu::Amount,
        _bill_id: core::BillId,
        signatures: Vec<cashu::BlindSignature>,
    ) -> Result<Vec<cashu::BlindSignature>> {
        Ok(signatures)
    }
}

pub struct ClwdrCl(pub ClowderNatsClient);
#[async_trait]
impl credit::ClowderClient for ClwdrCl {
    async fn minting_ebill(
        &self,
        keyset_id: cashu::Id,
        quote_id: Uuid,
        amount: cashu::Amount,
        bill_id: core::BillId,
        signatures: Vec<cashu::BlindSignature>,
    ) -> Result<Vec<cashu::BlindSignature>> {
        let request = wire_clowder::messages::MintEbillRequest {
            keyset_id,
            amount,
            bill_id,
            quote_id,
        };
        let response = wire_clowder::messages::MintEbillResponse { signatures };
        let res = self
            .0
            .mint_bill(request, response)
            .await
            .map_err(Error::ClowderClient)?;
        Ok(res.signatures)
    }
}
pub async fn new_clowder_client(
    url: Option<reqwest::Url>,
) -> Result<Box<dyn credit::ClowderClient>> {
    let cl: Box<dyn credit::ClowderClient> = match url {
        None => Box::new(DummyClwdr {}),
        Some(url) => {
            let clowder_client = ClowderNatsClient::new(url)
                .await
                .map_err(Error::ClowderClient)?;
            Box::new(ClwdrCl(clowder_client))
        }
    };
    Ok(cl)
}

pub struct CoreCl(pub CoreClient);
#[async_trait]
impl credit::CoreClient for CoreCl {
    async fn info(&self, kid: cashu::Id) -> Result<cashu::KeySetInfo> {
        match self.0.keyset_info(kid).await {
            Ok(info) => Ok(info),
            Err(CoreError::KeysetIdNotFound(kid)) => {
                Err(Error::InvalidInput(format!("Unknown keyset: {kid}")))
            }
            Err(e) => Err(e.into()),
        }
    }

    async fn sign(&self, blind: cashu::BlindedMessage) -> Result<cashu::BlindSignature> {
        let res = self.0.sign(&blind).await?;
        Ok(res)
    }

    async fn burn(&self, proofs: Vec<cashu::Proof>) -> Result<()> {
        self.0.burn(proofs).await?;
        Ok(())
    }

    async fn recover(&self, proofs: Vec<cashu::Proof>) -> Result<()> {
        self.0.recover(proofs).await?;
        Ok(())
    }
}
