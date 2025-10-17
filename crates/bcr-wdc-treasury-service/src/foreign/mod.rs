// ----- standard library imports
// ----- extra library imports
use async_trait::async_trait;
// ----- local modules
pub mod clients;
pub mod crsat;
mod proof;
// ----- local imports
use crate::error::Result;

// ----- end imports
//
#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait Repository: Send + Sync {
    async fn store(&self, mint: cashu::MintUrl, proofs: Vec<cashu::Proof>) -> Result<()>;
    #[allow(dead_code)]
    async fn list(&self) -> Result<Vec<(cashu::MintUrl, cashu::Proof)>>;

    async fn store_htlc(
        &self,
        mint: cashu::MintUrl,
        hash: &str,
        proofs: Vec<cashu::Proof>,
    ) -> Result<()>;
    async fn search_htlc(&self, hash: &str) -> Result<Vec<(cashu::MintUrl, cashu::Proof)>>;
    async fn remove_htlcs(&self, ys: &[cashu::PublicKey]) -> Result<()>;
}

#[async_trait]
pub trait ClowderClient: proof::ClowderClient {
    async fn get_mint_url_from_pk(&self, pk: &cashu::PublicKey) -> Result<cashu::MintUrl>;
    async fn get_myself_pk(&self) -> Result<bitcoin::PublicKey>;
    async fn sign_p2pk_proofs(&self, proofs: &[cashu::Proof]) -> Result<Vec<cashu::Proof>>;
}

#[async_trait]
pub trait KeysClient: proof::KeysClient {
    async fn get_keyset_with_expiration(
        &self,
        expiration: chrono::NaiveDate,
    ) -> Result<cashu::KeySet>;
}
