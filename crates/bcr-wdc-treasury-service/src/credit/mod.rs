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

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct MeltOperation {
    pub kid: cashu::Id,
    pub melted: cashu::Amount,
}

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait Repository: Send + Sync {
    async fn mint_store(&self, mint_operation: MintOperation) -> Result<()>;
    async fn mint_load(&self, uid: Uuid) -> Result<MintOperation>;
    async fn mint_list(&self, kid: cashu::Id) -> Result<Vec<MintOperation>>;
    async fn mint_update_field(
        &self,
        uid: Uuid,
        old_minted: cashu::Amount,
        new_minted: cashu::Amount,
    ) -> Result<()>;

    async fn melt_store(&self, melt_operation: MeltOperation) -> Result<()>;
    async fn melt_load(&self, kid: cashu::Id) -> Result<MeltOperation>;
    async fn melt_update_field(
        &self,
        kid: cashu::Id,
        old_melted: cashu::Amount,
        new_melted: cashu::Amount,
    ) -> Result<()>;
}

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait CoreClient: Send + Sync {
    async fn info(&self, kid: cashu::Id) -> Result<cashu::KeySetInfo>;
    async fn sign(&self, blind: cashu::BlindedMessage) -> Result<cashu::BlindSignature>;
    async fn burn(&self, proofs: Vec<cashu::Proof>) -> Result<()>;
    async fn recover(&self, proofs: Vec<cashu::Proof>) -> Result<()>;
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
