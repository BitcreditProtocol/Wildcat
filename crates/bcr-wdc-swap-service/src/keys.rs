// ----- standard library imports
// ----- extra library imports
use anyhow::Result as AnyResult;
use async_trait::async_trait;
use cashu::mint::MintKeySetInfo;
use cashu::nuts::nut02 as cdk02;
// ----- local imports
use crate::error::Result;
use crate::service::KeysService;

#[derive(Debug, Default, Clone, serde::Deserialize)]
pub struct KeysClientConfig {}

#[derive(Debug, Clone)]
pub struct DummyKeysService {}

impl DummyKeysService {
    pub async fn new(_cfg: KeysClientConfig) -> AnyResult<Self> {
        Ok(Self {})
    }
}

#[async_trait]
impl KeysService for DummyKeysService {
    async fn keyset(&self, id: &cdk02::Id) -> Result<Option<cdk02::MintKeySet>> {
        log::debug!("DummyKeys keyset({:?})", id);
        Ok(None)
    }
    async fn info(&self, id: &cdk02::Id) -> Result<Option<MintKeySetInfo>> {
        log::debug!("DummyKeys info({:?})", id);
        Ok(None)
    }
}
