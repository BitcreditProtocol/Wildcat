// ----- standard library imports
// ----- extra library imports
use async_trait::async_trait;
use bcr_common::client::keys::{Client as KeysClient, Error as KeysError};
use cashu::nut02 as cdk02;
// ----- local imports
use crate::{
    credit::KeyService,
    error::{Error, Result},
};

// ----- end imports

#[derive(Clone)]
pub struct KeySrvc(KeysClient);

impl KeySrvc {
    pub fn new(url: reqwest::Url) -> Self {
        let client = KeysClient::new(url);
        Self(client)
    }
}

#[async_trait]
impl KeyService for KeySrvc {
    async fn info(&self, kid: cdk02::Id) -> Result<cdk02::KeySetInfo> {
        match self.0.keyset_info(kid).await {
            Ok(info) => Ok(info),
            Err(KeysError::ResourceNotFound(kid)) => Err(Error::UnknownKeyset(kid)),
            Err(e) => Err(Error::KeyClient(e)),
        }
    }

    async fn keys(&self, kid: cdk02::Id) -> Result<cdk02::KeySet> {
        match self.0.keys(kid).await {
            Ok(keys) => Ok(keys),
            Err(KeysError::ResourceNotFound(kid)) => Err(Error::UnknownKeyset(kid)),
            Err(e) => Err(Error::KeyClient(e)),
        }
    }
}
