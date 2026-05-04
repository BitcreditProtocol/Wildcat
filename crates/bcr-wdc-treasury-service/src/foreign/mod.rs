// ----- standard library imports
use std::collections::HashMap;
// ----- extra library imports
use async_trait::async_trait;
use bcr_common::{cashu, wire::keys as wire_keys};
pub use bitcoin::hashes::sha256::Hash as Sha256Hash;
// ----- local modules
pub mod clients;
mod proof;
mod service;
pub mod settle;
// ----- local imports
use crate::{error::Result, TStamp};

// ----- end imports

pub use service::Service;

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait OnlineRepository: Send + Sync {
    async fn store(&self, mint_id: secp256k1::PublicKey, proofs: Vec<cashu::Proof>) -> Result<()>;
    #[allow(dead_code)]
    async fn list(&self) -> Result<Vec<(secp256k1::PublicKey, cashu::Proof)>>;

    async fn store_htlc(
        &self,
        mint_id: secp256k1::PublicKey,
        hash: Sha256Hash,
        proofs: Vec<cashu::Proof>,
    ) -> Result<()>;
    async fn search_htlc(
        &self,
        hash: &Sha256Hash,
    ) -> Result<Vec<(secp256k1::PublicKey, cashu::Proof)>>;
    async fn remove_htlcs(&self, ys: &[cashu::PublicKey]) -> Result<()>;
}

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait OfflineRepository: Send + Sync {
    async fn store_fps(
        &self,
        mint_id: secp256k1::PublicKey,
        fps: Vec<wire_keys::ProofFingerprint>,
        hash: Vec<Sha256Hash>,
    ) -> Result<()>;
    async fn search_fp(
        &self,
        hash: &Sha256Hash,
    ) -> Result<Option<(secp256k1::PublicKey, wire_keys::ProofFingerprint)>>;
    async fn remove_fps(&self, ys: &[cashu::PublicKey]) -> Result<()>;
    async fn store_proofs(
        &self,
        mint_id: secp256k1::PublicKey,
        proof: Vec<cashu::Proof>,
    ) -> Result<()>;
    #[allow(dead_code)]
    async fn load_proofs(&self, mint_id: secp256k1::PublicKey) -> Result<Vec<cashu::Proof>>;
    #[allow(dead_code)]
    async fn remove_proofs(&self, ys: &[cashu::PublicKey]) -> Result<()>;
}

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait KeysClient: Send + Sync {
    async fn get_keyset_with_expiration(
        &self,
        expiration: chrono::NaiveDate,
    ) -> Result<cashu::KeySet>;
    async fn sign(&self, blinds: &[cashu::BlindedMessage]) -> Result<Vec<cashu::BlindSignature>>;
}

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait ClowderClient: Send + Sync {
    async fn get_mint_url_from_pk(&self, pk: &secp256k1::PublicKey) -> Result<reqwest::Url>;
    async fn get_myself_pk(&self) -> Result<secp256k1::PublicKey>;
    async fn sign_p2pk_proofs(&self, proofs: &[cashu::Proof]) -> Result<Vec<cashu::Proof>>;
    // yes if result is Ok
    async fn can_accept_offline_exchange(
        &self,
        fps: Vec<wire_keys::ProofFingerprint>,
    ) -> Result<(reqwest::Url, secp256k1::PublicKey)>;
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
    async fn check_htlc_proofs(
        &self,
        issuer: cashu::PublicKey,
        proofs: Vec<cashu::Proof>,
    ) -> Result<()>;
}

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait ForeignClient: Send + Sync {
    async fn swap(
        &self,
        inputs: Vec<cashu::Proof>,
        outputs: Vec<cashu::BlindedMessage>,
        now: TStamp,
    ) -> Result<Vec<cashu::BlindSignature>>;

    async fn check_state(&self, ys: Vec<cashu::PublicKey>) -> Result<Vec<cashu::ProofState>>;
    async fn get_keyset(&self, kid: cashu::Id) -> Result<cashu::KeySet>;
}

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait MintClientFactory: Send + Sync {
    async fn make_client(
        &self,
        mint_url: reqwest::Url,
        mint_pk: secp256k1::PublicKey,
    ) -> Result<Box<dyn ForeignClient>>;
}

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait OfflineSettleHandler: Send + Sync {
    fn monitor(&self, mint: (secp256k1::PublicKey, reqwest::Url)) -> Result<()>;
    async fn stop(&self) -> Result<()>;
}

fn proofs_vec_to_map(
    input: Vec<(secp256k1::PublicKey, cashu::Proof)>,
) -> HashMap<secp256k1::PublicKey, Vec<cashu::Proof>> {
    let mut map: HashMap<secp256k1::PublicKey, Vec<cashu::Proof>> = HashMap::new();
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
    for (fp, hash) in input.into_iter().zip(hashes) {
        map.entry(fp.keyset_id).or_default().push((fp, hash));
    }
    map
}
