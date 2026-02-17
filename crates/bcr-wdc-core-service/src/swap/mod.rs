// ----- standard library imports
// ----- extra library imports
use async_trait::async_trait;
use bcr_common::cashu;
// ----- local imports
use crate::{error::Result, keys::service::Service as KeysService};
// ----- local modules
pub mod service;

// ----- end imports

#[async_trait]
pub trait SigningService: Send + Sync {
    async fn info(&self, id: &cashu::Id) -> Result<cashu::KeySetInfo>;
    async fn sign_blind(&self, blind: &cashu::BlindedMessage) -> Result<cashu::BlindSignature>;
    async fn verify_proof(&self, proof: &cashu::Proof) -> Result<()>;
}

#[async_trait]
impl SigningService for KeysService {
    async fn info(&self, id: &cashu::Id) -> Result<cashu::KeySetInfo> {
        self.info(*id).await.map(cashu::KeySetInfo::from)
    }

    async fn sign_blind(&self, blind: &cashu::BlindedMessage) -> Result<cashu::BlindSignature> {
        self.sign_blind(blind).await
    }

    async fn verify_proof(&self, proof: &cashu::Proof) -> Result<()> {
        self.verify_proof(proof.clone()).await
    }
}
