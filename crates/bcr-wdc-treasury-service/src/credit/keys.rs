// ----- standard library imports
// ----- extra library imports
use async_trait::async_trait;
use bcr_wdc_key_client::{Error as KeyClientError, KeyClient};
use cashu::nut02 as cdk02;
// ----- local imports
use crate::error::{Error, Result};

// ----- end imports
use crate::credit::KeyService;

#[derive(Clone, Debug, serde::Deserialize)]
pub struct KeySrvcConfig {
    pub base: bcr_wdc_key_client::Url,
}

#[derive(Clone)]
pub struct KeySrvc(KeyClient);

impl KeySrvc {
    pub fn new(cfg: KeySrvcConfig) -> Self {
        let client = KeyClient::new(cfg.base);
        Self(client)
    }
}

#[async_trait]
impl KeyService for KeySrvc {
    async fn info(&self, kid: cdk02::Id) -> Result<cdk02::KeySetInfo> {
        match self.0.keyset_info(kid).await {
            Ok(info) => Ok(info),
            Err(KeyClientError::ResourceNotFound(kid)) => Err(Error::UnknownKeyset(kid)),
            Err(e) => Err(Error::KeyClient(e)),
        }
    }

    async fn keys(&self, kid: cdk02::Id) -> Result<cdk02::KeySet> {
        match self.0.keys(kid).await {
            Ok(keys) => Ok(keys),
            Err(KeyClientError::ResourceNotFound(kid)) => Err(Error::UnknownKeyset(kid)),
            Err(e) => Err(Error::KeyClient(e)),
        }
    }
}
