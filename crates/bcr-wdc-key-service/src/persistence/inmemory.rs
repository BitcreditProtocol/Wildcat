// ----- standard library imports
// ----- extra library imports
use async_trait::async_trait;
use bcr_wdc_keys::{persistence::InMemoryMap, KeysetID};
use cashu::mint::MintKeySetInfo;
use cashu::nuts::nut02 as cdk02;
// ----- local imports
use crate::error::{Error, Result};
use crate::service::KeysRepository;

#[async_trait]
impl KeysRepository for InMemoryMap {
    async fn info(&self, id: &cdk02::Id) -> Result<Option<MintKeySetInfo>> {
        let kid = KeysetID::from(*id);
        self.info(&kid).await.map_err(Error::KeysRepository)
    }
    async fn keyset(&self, id: &cdk02::Id) -> Result<Option<cdk02::MintKeySet>> {
        let kid = KeysetID::from(*id);
        self.keyset(&kid).await.map_err(Error::KeysRepository)
    }
}
