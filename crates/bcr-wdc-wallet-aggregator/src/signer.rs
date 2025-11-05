// ----- standard library imports
// ----- extra library imports
use async_trait::async_trait;
use clwdr_client::SignatoryNatsClient;
// ----- local imports
use crate::{
    commitment,
    error::{Error, Result},
};

// ----- end imports

#[allow(dead_code)]
#[derive(Default)]
pub struct DummySigner {}

#[async_trait]
impl commitment::Signer for DummySigner {
    async fn sign(&self, _content: &[u8]) -> Result<bitcoin::secp256k1::schnorr::Signature> {
        Err(Error::NotYet(
            "DummySigner does not implement signing".to_string(),
        ))
    }
}

pub struct ClowderSigner(SignatoryNatsClient);
impl ClowderSigner {
    pub async fn new(url: reqwest::Url) -> Result<Self> {
        let inner = SignatoryNatsClient::new(url, None).await?;
        Ok(Self(inner))
    }
}

#[async_trait]
impl commitment::Signer for ClowderSigner {
    async fn sign(&self, content: &[u8]) -> Result<bitcoin::secp256k1::schnorr::Signature> {
        let signature = self.0.sign_bytes(content).await?;
        Ok(signature)
    }
}
