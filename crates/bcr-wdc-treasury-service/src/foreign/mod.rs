// ----- standard library imports
use std::collections::HashMap;
// ----- extra library imports
use async_trait::async_trait;
// ----- local modules
pub mod clients;
pub mod crsat;
mod proof;
pub mod sat;
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

fn proofs_vec_to_map(
    input: Vec<(cashu::MintUrl, cashu::Proof)>,
) -> HashMap<cashu::MintUrl, Vec<cashu::Proof>> {
    let mut map: HashMap<cashu::MintUrl, Vec<cashu::Proof>> = HashMap::new();
    for (mint, proof) in input {
        map.entry(mint).or_default().push(proof);
    }
    map
}
