// ----- standard library imports
// ----- extra library imports
use cashu::nuts::nut01 as cdk01;
use cashu::nuts::nut06 as cdk06;

use async_trait::async_trait;

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait MintService {
    async fn info(&self) -> crate::error::Result<cdk06::MintInfo>;
    async fn keys(&self) -> crate::error::Result<cdk01::KeysResponse>;
}

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait KeysService: Send + Sync {
    async fn keys(&self) -> crate::error::Result<cdk01::KeysResponse>;
}

#[derive(Clone)]
pub struct Service<MS, KS> {
    pub mint_service: MS,
    pub key_service: KS,
}

impl<MS, KS> Service<MS, KS>
where
    MS: MintService,
    KS: KeysService,
{
    pub async fn keys(&self) -> crate::error::Result<cdk01::KeysResponse> {
        let mint_keys = self.mint_service.keys().await;
        // TODO: merge with key service response
        // let key_keys = self.key_service.keys().await;
        Ok(mint_keys?)
    }
}
