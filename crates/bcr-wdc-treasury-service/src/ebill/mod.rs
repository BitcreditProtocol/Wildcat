// ----- standard library imports
// ----- extra library imports
use async_trait::async_trait;
use bcr_common::{cashu, core::BillId};
use uuid::Uuid;
// ----- local modules
mod client;
mod service;
// ----- local imports
use crate::{error::Result, TStamp};

// ----- end imports

pub use client::{ClwdrCl, WildcatCl};
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
}

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait WildcatClient: Send + Sync {
    async fn info(&self, kid: cashu::Id) -> Result<cashu::KeySetInfo>;
    async fn sign(&self, blinds: &[cashu::BlindedMessage]) -> Result<Vec<cashu::BlindSignature>>;
    async fn burn(&self, proofs: Vec<cashu::Proof>) -> Result<()>;
    async fn recover(&self, proofs: Vec<cashu::Proof>) -> Result<()>;

    /// Fetches metadata for deriving the request to pay address from E-Bill, returns block id and previous block hash
    async fn prepare_request_to_pay(
        &self,
        bid: BillId,
    ) -> Result<(u64, bitcoin::hashes::sha256::Hash)>;
    /// Calls request to pay on E-Bill and returns the bill private key
    async fn request_to_pay(
        &self,
        bid: BillId,
        expire: TStamp,
        payment_address: bitcoin::Address,
    ) -> Result<secp256k1::SecretKey>;
    async fn is_bill_paid(&self, bid: BillId) -> Result<bool>;
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

    async fn request_to_pay_ebill(
        &self,
        bid: BillId,
        payment_address: bitcoin::Address,
        block_id: u64,
        previous_block_hash: bitcoin::hashes::sha256::Hash,
        amount: bitcoin::Amount,
    ) -> Result<()>;

    async fn request_onchain_ebill_address(
        &self,
        bid: BillId,
        block_id: u64,
        previous_block_hash: bitcoin::hashes::sha256::Hash,
    ) -> Result<bitcoin::Address>;
}
