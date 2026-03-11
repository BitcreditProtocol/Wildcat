// ----- standard library imports
// ----- extra library imports
use async_trait::async_trait;
use bcr_common::{
    cashu, cdk,
    wire::{clowder::messages as wire_clowder, mint as wire_mint, signatures as wire_signatures},
};
use uuid::Uuid;
// ----- local modules
mod clowder;
mod service;
mod wallet;
mod wildcat;
// ----- local imports
use crate::{error::Result, TStamp};

// ----- end imports

pub use clowder::ClowderCl;
pub use service::{MintQuote, OnchainMeltQuote, Service};
pub use wallet::{CDKWallet, CDKWalletConfig};
pub use wildcat::{WildcatCl, WildcatClientConfig};

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait Wallet: Send + Sync {
    async fn mint_quote(
        &self,
        amount: cashu::Amount,
        signed_request: wire_signatures::SignedRequestToMintFromEBillDesc,
    ) -> Result<cdk::wallet::MintQuote>;
    async fn mint(&self, quote: String) -> Result<cashu::MintQuoteState>;
    async fn keysets_info(&self, kids: &[cashu::Id]) -> Result<Vec<cashu::KeySetInfo>>;
    async fn swap_to_messages(
        &self,
        outputs: &[cashu::BlindedMessage],
    ) -> Result<Vec<cashu::BlindSignature>>;
    async fn balance(&self) -> Result<cashu::Amount>;
}

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait WildcatClient: Send + Sync {
    async fn sign(&self, blinds: Vec<cashu::BlindedMessage>) -> Result<Vec<cashu::BlindSignature>>;
    async fn burn(&self, inputs: &[cashu::Proof]) -> Result<()>;
    async fn keyset_info(&self, kid: cashu::Id) -> Result<cashu::KeySetInfo>;
    async fn get_active_keyset(&self) -> Result<cashu::Id>;
}

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait ClowderClient: Send + Sync {
    async fn get_sweep(&self, qid: uuid::Uuid) -> Result<bitcoin::Address>;
    async fn pay_bill(
        &self,
        req: wire_clowder::BillPaymentRequest,
        resp: wire_clowder::BillPaymentResponse,
    ) -> Result<()>;
    async fn request_onchain_mint_address(
        &self,
        qid: Uuid,
        kid: cashu::Id,
    ) -> Result<bitcoin::Address>;
    async fn verify_onchain_mint_payment(
        &self,
        qid: Uuid,
        kid: cashu::Id,
    ) -> Result<bitcoin::Amount>;
    async fn mint_onchain(
        &self,
        qid: Uuid,
        kid: cashu::Id,
        signatures: Vec<cashu::BlindSignature>,
    ) -> Result<Vec<cashu::BlindSignature>>;
    async fn sign_onchain_mint_response(
        &self,
        msg: &wire_mint::OnchainMintQuoteResponseBody,
    ) -> Result<(String, secp256k1::schnorr::Signature)>;
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum MintStatus {
    Pending(Vec<cashu::BlindedMessage>),
    Paid(Vec<cashu::BlindSignature>),
}
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct OnChainMintOperation {
    pub qid: Uuid,
    pub recipient: bitcoin::Address<bitcoin::address::NetworkUnchecked>,
    pub target: cashu::Amount,
    pub expiry: TStamp,
    pub status: MintStatus,
}

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait Repository: Send + Sync {
    async fn store_quote(&self, quote: MintQuote) -> Result<()>;
    async fn update_quote(&self, quote: MintQuote) -> Result<()>;
    async fn list_quotes(&self) -> Result<Vec<MintQuote>>;
    async fn store_onchain_melt(&self, quote_id: uuid::Uuid, data: OnchainMeltQuote) -> Result<()>;
    async fn load_onchain_melt(&self, quote_id: uuid::Uuid) -> Result<OnchainMeltQuote>;

    async fn store_onchain_mintop(&self, op: OnChainMintOperation) -> Result<()>;
    async fn load_onchain_mintop(&self, qid: Uuid) -> Result<OnChainMintOperation>;
    async fn update_onchain_mintop_status(&self, qid: Uuid, status: MintStatus) -> Result<()>;
}
