use async_trait::async_trait;
use cashu::KeysResponse;
// ----- standard library imports
// ----- extra library imports
use crate::service::KeysService;
use bcr_wdc_key_client::Error::InvalidRequest;
use bcr_wdc_key_client::KeyClient;
// ----- local imports
use crate::error::{Error, Result};

#[derive(Debug, Clone, serde::Deserialize)]
pub struct KeysClientConfig {
    base_url: bcr_wdc_key_client::Url,
}

#[derive(Debug, Clone)]
pub struct RESTClient(KeyClient);
impl RESTClient {
    pub async fn new(cfg: KeysClientConfig) -> Result<Self> {
        let cl = KeyClient::new(cfg.base_url).map_err(Error::KeysClient)?;
        Ok(Self(cl))
    }
}

#[async_trait]
impl KeysService for RESTClient {
    async fn keys(&self) -> Result<KeysResponse> {
        Err(Error::KeysClient(InvalidRequest))
    }
}
