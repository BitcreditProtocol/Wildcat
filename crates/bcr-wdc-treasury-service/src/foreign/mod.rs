// ----- standard library imports
use std::collections::HashMap;
// ----- extra library imports
use async_trait::async_trait;
use bcr_common::{cashu, client::cdk::MintConnectorExt, wire::keys as wire_keys};
pub use bitcoin::hashes::sha256::Hash as Sha256Hash;
// ----- local modules
pub mod clients;
pub mod crsat;
mod proof;
pub mod sat;
pub mod settle;
// ----- local imports
use crate::error::Result;

// ----- end imports
//
#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait OnlineRepository: Send + Sync {
    async fn store(
        &self,
        mint: (secp256k1::PublicKey, cashu::MintUrl),
        proofs: Vec<cashu::Proof>,
    ) -> Result<()>;
    #[allow(dead_code)]
    async fn list(&self) -> Result<Vec<((secp256k1::PublicKey, cashu::MintUrl), cashu::Proof)>>;

    async fn store_htlc(
        &self,
        mint: (secp256k1::PublicKey, cashu::MintUrl),
        hash: Sha256Hash,
        proofs: Vec<cashu::Proof>,
    ) -> Result<()>;
    async fn search_htlc(
        &self,
        hash: &Sha256Hash,
    ) -> Result<Vec<((secp256k1::PublicKey, cashu::MintUrl), cashu::Proof)>>;
    async fn remove_htlcs(&self, ys: &[cashu::PublicKey]) -> Result<()>;
}

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait OfflineRepository: Send + Sync {
    async fn store_fps(
        &self,
        alpha: (secp256k1::PublicKey, cashu::MintUrl),
        fps: Vec<wire_keys::ProofFingerprint>,
        hash: Vec<Sha256Hash>,
    ) -> Result<()>;
    async fn search_fp(
        &self,
        hash: &Sha256Hash,
    ) -> Result<
        Option<(
            (secp256k1::PublicKey, cashu::MintUrl),
            wire_keys::ProofFingerprint,
        )>,
    >;
    async fn remove_fps(&self, ys: &[cashu::PublicKey]) -> Result<()>;
    async fn store_proofs(
        &self,
        alpha: (secp256k1::PublicKey, cashu::MintUrl),
        proof: Vec<cashu::Proof>,
    ) -> Result<()>;
    #[allow(dead_code)]
    async fn load_proofs(
        &self,
        alpha: &(secp256k1::PublicKey, cashu::MintUrl),
    ) -> Result<Vec<cashu::Proof>>;
    #[allow(dead_code)]
    async fn remove_proofs(&self, ys: &[cashu::PublicKey]) -> Result<()>;
}

#[async_trait]
pub trait ClowderClient: proof::ClowderClient {
    async fn get_mint_url_from_pk(&self, pk: &cashu::PublicKey) -> Result<cashu::MintUrl>;
    async fn get_myself_pk(&self) -> Result<bitcoin::PublicKey>;
    async fn sign_p2pk_proofs(&self, proofs: &[cashu::Proof]) -> Result<Vec<cashu::Proof>>;
    // yes if result is Ok
    async fn can_accept_offline_exchange(
        &self,
        fps: Vec<wire_keys::ProofFingerprint>,
    ) -> Result<(cashu::MintUrl, secp256k1::PublicKey)>;
    async fn get_keyset_info(
        &self,
        alpha_pk: &secp256k1::PublicKey,
        kid: &cashu::Id,
    ) -> Result<cashu::KeySetInfo>;
    async fn get_keyset(
        &self,
        alpha_pk: &secp256k1::PublicKey,
        kid: &cashu::Id,
    ) -> Result<cashu::KeySet>;
    async fn is_offline(&self, pk: secp256k1::PublicKey) -> Result<bool>;
}

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait MintClientFactory: Send + Sync {
    async fn make_client(&self, mint_url: cashu::MintUrl) -> Result<Box<dyn MintConnectorExt>>;
}

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait OfflineSettleHandler: Send + Sync {
    fn monitor(&self, mint: (secp256k1::PublicKey, cashu::MintUrl)) -> Result<()>;
    async fn stop(&self) -> Result<()>;
}

fn proofs_vec_to_map(
    input: Vec<((secp256k1::PublicKey, cashu::MintUrl), cashu::Proof)>,
) -> HashMap<(secp256k1::PublicKey, cashu::MintUrl), Vec<cashu::Proof>> {
    let mut map: HashMap<(secp256k1::PublicKey, cashu::MintUrl), Vec<cashu::Proof>> =
        HashMap::new();
    for (mint, proof) in input {
        map.entry(mint).or_default().push(proof);
    }
    map
}

fn fingerprints_vec_to_map(
    input: Vec<wire_keys::ProofFingerprint>,
    hashes: Vec<Sha256Hash>,
) -> HashMap<cashu::Id, Vec<(wire_keys::ProofFingerprint, Sha256Hash)>> {
    let mut map: HashMap<cashu::Id, Vec<(wire_keys::ProofFingerprint, Sha256Hash)>> =
        HashMap::new();
    for (fp, hash) in input.into_iter().zip(hashes.into_iter()) {
        map.entry(fp.keyset_id).or_default().push((fp, hash));
    }
    map
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::Result;
    use async_trait::async_trait;
    use bcr_common::wire::keys as wire_keys;

    mockall::mock! {
        pub ClowderClient{
        }

        #[async_trait]
        impl super::proof::ClowderClient for ClowderClient {
            async fn check_htlc_proofs(
                &self,
                issuer: cashu::PublicKey,
                proofs: Vec<cashu::Proof>,
            ) -> Result<()>;
        }
        #[async_trait]
        impl super::ClowderClient for ClowderClient {
            async fn get_mint_url_from_pk(&self, pk: &cashu::PublicKey) -> Result<cashu::MintUrl>;
            async fn get_myself_pk(&self) -> Result<bitcoin::PublicKey>;
            async fn sign_p2pk_proofs(&self, proofs: &[cashu::Proof]) -> Result<Vec<cashu::Proof>>;
            async fn can_accept_offline_exchange(
                &self,
                fps: Vec<wire_keys::ProofFingerprint>,
            ) -> Result<(cashu::MintUrl, secp256k1::PublicKey)>;
            async fn get_keyset_info(
                &self,
                alpha_pk: &secp256k1::PublicKey,
                kid: &cashu::Id,
            ) -> Result<cashu::KeySetInfo>;
            async fn get_keyset(
                &self,
                alpha_pk: &secp256k1::PublicKey,
                kid: &cashu::Id,
            ) -> Result<cashu::KeySet>;
            async fn is_offline(&self, pk: secp256k1::PublicKey) -> Result<bool>;
        }
    }
}
