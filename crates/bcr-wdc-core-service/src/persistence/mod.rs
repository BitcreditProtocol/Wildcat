// ----- standard library imports
// ----- extra library imports
use async_trait::async_trait;
use bcr_common::{cashu, cdk_common::mint::MintKeySetInfo};
use bcr_wdc_utils::keys as keys_utils;
use bitcoin::secp256k1::schnorr;
// ----- local imports
use crate::{error::Result, TStamp};
// ----- local modules
pub mod inmemory;
pub mod surreal;

// ----- end imports

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait KeysRepository: Send + Sync {
    async fn store(&self, keys: keys_utils::KeysetEntry) -> Result<()>;
    async fn info(&self, id: cashu::Id) -> Result<Option<MintKeySetInfo>>;
    async fn keyset(&self, id: cashu::Id) -> Result<Option<cashu::MintKeySet>>;
    async fn list_info(
        &self,
        currency: Option<cashu::CurrencyUnit>,
        min_expiration_tstamp: Option<u64>,
        max_expiration_tstamp: Option<u64>,
    ) -> Result<Vec<MintKeySetInfo>>;
    async fn list_keyset(&self) -> Result<Vec<cashu::MintKeySet>>;
    async fn update_info(&self, info: MintKeySetInfo) -> Result<()>;
    async fn infos_for_expiration_date(&self, expire: u64) -> Result<Vec<MintKeySetInfo>>;
}

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait SignaturesRepository: Send + Sync {
    async fn store(&self, y: cashu::PublicKey, signature: cashu::BlindSignature) -> Result<()>;
    async fn load(&self, blind: &cashu::BlindedMessage) -> Result<Option<cashu::BlindSignature>>;
}

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait ProofRepository: Send + Sync {
    /// WARNING: this method should do strict insert.
    /// i.e. it should fail if any of the proofs is already present in the DB
    /// in case of failure, the DB should be in the same state as before the call
    async fn insert(&self, tokens: &[cashu::Proof]) -> Result<()>;
    async fn remove(&self, tokens: &[cashu::Proof]) -> Result<()>;
    async fn contains(&self, y: cashu::PublicKey) -> Result<Option<cashu::ProofState>>;
}

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait CommitmentRepository: Send + Sync {
    async fn store(
        &self,
        inputs: Vec<cashu::PublicKey>,
        outputs: Vec<cashu::PublicKey>,
        expiration: TStamp,
        wallet_key: cashu::PublicKey,
        wallet_signature: schnorr::Signature,
        commitment: schnorr::Signature,
    ) -> Result<()>;
    async fn load(
        &self,
        signature: &schnorr::Signature,
    ) -> Result<(Vec<cashu::PublicKey>, Vec<cashu::PublicKey>, TStamp)>;
    async fn contains_inputs(&self, inputs: &[cashu::PublicKey]) -> Result<bool>;
    async fn contains_outputs(&self, outputs: &[cashu::PublicKey]) -> Result<bool>;
    async fn delete(&self, commitment: schnorr::Signature) -> Result<()>;
    async fn clean_expired(&self, now: TStamp) -> Result<()>;
}
