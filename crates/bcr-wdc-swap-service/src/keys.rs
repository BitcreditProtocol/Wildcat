// ----- standard library imports
// ----- extra library imports
use async_trait::async_trait;
use bcr_common::client::keys::{Client as KeysClient, Error as KeysError};
// ----- local imports
use crate::error::{Error, Result};
use crate::service::KeysService;

#[derive(Debug, Clone, serde::Deserialize)]
pub struct KeysClientConfig {
    base_url: reqwest::Url,
}

#[derive(Debug, Clone)]
pub struct RESTClient(KeysClient);
impl RESTClient {
    pub fn new(cfg: KeysClientConfig) -> Self {
        let cl = KeysClient::new(cfg.base_url);
        Self(cl)
    }
}

#[async_trait]
impl KeysService for RESTClient {
    async fn info(&self, id: &cashu::Id) -> Result<cashu::KeySetInfo> {
        let response = self.0.keyset_info(*id).await;
        match response {
            Ok(info) => Ok(info),
            Err(KeysError::ResourceNotFound(kid)) => Err(Error::UnknownKeyset(kid)),
            Err(e) => Err(Error::KeysClient(e)),
        }
    }
    async fn sign_blind(&self, blind: &cashu::BlindedMessage) -> Result<cashu::BlindSignature> {
        let response = self.0.sign(blind).await;
        match response {
            Ok(signature) => Ok(signature),
            Err(KeysError::ResourceNotFound(kid)) => Err(Error::UnknownKeyset(kid)),
            Err(KeysError::InvalidRequest) => {
                Err(Error::InvalidBlindedMessage(blind.blinded_secret))
            }
            Err(e) => Err(Error::KeysClient(e)),
        }
    }
    async fn verify_proof(&self, proof: &cashu::Proof) -> Result<()> {
        let response = self.0.verify(proof).await;
        match response {
            Ok(()) => Ok(()),
            Err(KeysError::ResourceNotFound(kid)) => Err(Error::UnknownKeyset(kid)),
            Err(KeysError::InvalidRequest) => Err(Error::InvalidProof(proof.secret.clone())),
            Err(e) => Err(Error::KeysClient(e)),
        }
    }
}
