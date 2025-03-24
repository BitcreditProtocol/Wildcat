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

#[derive(Clone)]
pub struct Service<MS> {
    pub mint_service: MS,
}

impl<MS> Service<MS> where MS: MintService {}
