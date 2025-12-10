// ----- standard library imports
// ----- extra library imports
use async_trait::async_trait;
use bitcoin::hashes::{sha256::Hash as Sha256, Hash};
use bitcoin::secp256k1 as secp;
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

pub struct LocalSigner {
    kp: secp::Keypair,
}

impl LocalSigner {
    pub fn random() -> Self {
        let mut rng = secp::rand::thread_rng();
        let kp = secp::Keypair::new(secp::global::SECP256K1, &mut rng);
        tracing::info!("LocalSigner public key is {}", kp.public_key());
        Self { kp }
    }
}

#[async_trait]
impl commitment::Signer for LocalSigner {
    async fn sign(&self, content: &[u8]) -> Result<bitcoin::secp256k1::schnorr::Signature> {
        let sha = Sha256::hash(content);
        let secp_msg = secp::Message::from_digest(*sha.as_ref());
        let signature = secp::global::SECP256K1.sign_schnorr(&secp_msg, &self.kp);
        Ok(signature)
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
        let signature = self.0.sign_schnorr_preimage(content).await?;
        Ok(signature)
    }
}
