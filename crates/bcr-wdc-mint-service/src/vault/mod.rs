// ----- standard library imports
// ----- extra library imports
use async_trait::async_trait;
use bcr_common::cashu;
// ----- local modules
mod clients;
mod service;
// ----- local imports
use crate::error::Result;

// ----- end imports

pub use clients::WildcatCl;
pub use service::Service;

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait Repository: Send + Sync {
    async fn store_proofs(&self, proofs: Vec<cashu::Proof>) -> Result<()>;
    async fn load_proofs(&self, ys: Vec<cashu::PublicKey>) -> Result<Vec<cashu::Proof>>;
    async fn list_ys(&self) -> Result<Vec<cashu::PublicKey>>;
    async fn delete_proofs(&self, ys: &[cashu::PublicKey]) -> Result<()>;
}

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait WildcatClient: Send + Sync {
    async fn check_spent(&self, proofs: Vec<cashu::PublicKey>) -> Result<Vec<cashu::ProofState>>;
    fn unit(&self) -> cashu::CurrencyUnit;
}
