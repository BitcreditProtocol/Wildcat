// ----- standard library imports
// ----- extra library imports
use async_trait::async_trait;
use bcr_wdc_key_client::Error as KeyClientError;
use bcr_wdc_key_client::KeyClient;
use cashu::nuts::nut00 as cdk00;
use cashu::nuts::nut02 as cdk02;
// ----- local imports
use crate::error::{Error, Result};
use crate::service::KeysService;

#[derive(Debug, Clone, serde::Deserialize)]
pub struct KeysClientConfig {
    base_url: bcr_wdc_key_client::Url,
}

#[derive(Debug, Clone)]
pub struct RESTClient(KeyClient);
impl RESTClient {
    pub fn new(cfg: KeysClientConfig) -> Self {
        let cl = KeyClient::new(cfg.base_url);
        Self(cl)
    }
}

#[async_trait]
impl KeysService for RESTClient {
    async fn info(&self, id: &cdk02::Id) -> Result<cdk02::KeySetInfo> {
        let response = self.0.keyset_info(*id).await;
        match response {
            Ok(info) => Ok(info),
            Err(KeyClientError::ResourceNotFound(kid)) => Err(Error::UnknownKeyset(kid)),
            Err(e) => Err(Error::KeysClient(e)),
        }
    }
    async fn sign_blind(&self, blind: &cdk00::BlindedMessage) -> Result<cdk00::BlindSignature> {
        let response = self.0.sign(blind).await;
        match response {
            Ok(signature) => Ok(signature),
            Err(KeyClientError::ResourceNotFound(kid)) => Err(Error::UnknownKeyset(kid)),
            Err(KeyClientError::InvalidRequest) => {
                Err(Error::InvalidBlindedMessage(blind.blinded_secret))
            }
            Err(e) => Err(Error::KeysClient(e)),
        }
    }
    async fn verify_proof(&self, proof: &cdk00::Proof) -> Result<()> {
        let response = self.0.verify(proof).await;
        match response {
            Ok(()) => Ok(()),
            Err(KeyClientError::ResourceNotFound(kid)) => Err(Error::UnknownKeyset(kid)),
            Err(KeyClientError::InvalidRequest) => Err(Error::InvalidProof(proof.secret.clone())),
            Err(e) => Err(Error::KeysClient(e)),
        }
    }
}
