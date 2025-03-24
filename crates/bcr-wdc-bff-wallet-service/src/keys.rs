// ----- standard library imports
// ----- extra library imports
use bcr_wdc_key_client::Error as KeyClientError;
use bcr_wdc_key_client::KeyClient;
use thiserror::Error;
// ----- local imports

#[allow(dead_code)]
#[derive(Debug, Error)]
pub enum Error {
    // external errors wrappers
    #[error("Keyset Client error: {0}")]
    KeysClient(KeyClientError),
}

pub type Result<T> = std::result::Result<T, Error>;

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
