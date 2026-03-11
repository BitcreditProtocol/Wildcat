// ----- standard library imports
// ----- extra library imports
use async_trait::async_trait;
use bcr_common::{cashu, core::BillId};
use uuid::Uuid;
// ----- local modules
mod client;
mod service;
// ----- local imports
use crate::error::Result;

// ----- end imports

pub use client::{new_clowder_client, CoreCl};
pub use service::Service;

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct MintOperation {
    pub uid: Uuid,
    pub kid: cashu::Id,
    pub pub_key: cashu::PublicKey,
    pub target: cashu::Amount,
    pub minted: cashu::Amount,
    pub bill_id: BillId,
}

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait MintOpRepository: Send + Sync {
    async fn store(&self, mint_operation: MintOperation) -> Result<()>;
    async fn load(&self, uid: Uuid) -> Result<MintOperation>;
    async fn list(&self, kid: cashu::Id) -> Result<Vec<MintOperation>>;
    async fn update_minted_field(
        &self,
        uid: Uuid,
        old_minted: cashu::Amount,
        new_minted: cashu::Amount,
    ) -> Result<()>;
}

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait CoreClient: Send + Sync {
    async fn info(&self, kid: cashu::Id) -> Result<cashu::KeySetInfo>;
    async fn sign(&self, blind: cashu::BlindedMessage) -> Result<cashu::BlindSignature>;
}

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait ClowderClient: Send + Sync {
    async fn minting_ebill(
        &self,
        kid: cashu::Id,
        qid: Uuid,
        amount: cashu::Amount,
        bid: BillId,
        signs: Vec<cashu::BlindSignature>,
    ) -> Result<Vec<cashu::BlindSignature>>;
}
