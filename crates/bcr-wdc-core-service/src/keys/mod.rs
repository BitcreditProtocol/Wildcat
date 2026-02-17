// ----- standard library imports
// ----- extra library imports
use async_trait::async_trait;
use bcr_common::{cashu, core::BillId};
// ----- local imports
use crate::error::Result;
// ----- local modules
pub mod clowder;
pub mod factory;
pub mod service;

// ----- end imports

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait ClowderClient: Send + Sync {
    async fn mint_ebill(
        &self,
        keyset_id: cashu::Id,
        quote_id: uuid::Uuid,
        amount: cashu::Amount,
        bill_id: BillId,
        signatures: Vec<cashu::BlindSignature>,
    ) -> Result<Vec<cashu::BlindSignature>>;
    async fn new_keyset(&self, keyset: cashu::KeySet) -> Result<()>;
    async fn keyset_deactivated(&self, kid: cashu::Id) -> Result<()>;
}
