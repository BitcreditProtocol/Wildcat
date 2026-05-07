// ----- standard library imports
// ----- extra library imports
use async_trait::async_trait;
use bcr_common::{
    cashu,
    wire::{
        attestation::IssuanceAttestation, clowder::messages as wire_clowder, melt as wire_melt,
        mint as wire_mint,
    },
};
use bitcoin::secp256k1::PublicKey;
use uuid::Uuid;
// ----- local modules
mod clowder;
mod monitor;
mod service;
mod wildcat;
// ----- local imports
use crate::{error::Result, TStamp};

// ----- end imports

pub use clowder::ClowderCl;
pub use monitor::MintOpMonitor;
pub use service::{MintQuote, Service};
pub use wildcat::WildcatCl;

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait WildcatClient: Send + Sync {
    async fn sign(&self, blinds: Vec<cashu::BlindedMessage>) -> Result<Vec<cashu::BlindSignature>>;
    async fn burn(&self, inputs: Vec<cashu::Proof>) -> Result<()>;
    async fn recover(&self, inputs: Vec<cashu::Proof>) -> Result<()>;
    async fn keyset_info(&self, kid: cashu::Id) -> Result<cashu::KeySetInfo>;
    async fn get_active_keyset(&self) -> Result<cashu::Id>;
}

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait ClowderClient: Send + Sync {
    async fn get_sweep(&self, qid: uuid::Uuid) -> Result<bitcoin::Address>;
    async fn request_to_pay_bill(
        &self,
        req: wire_clowder::RequestToPayEbillRequest,
        resp: wire_clowder::RequestToPayEbillResponse,
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
    async fn sign_onchain_melt_response(
        &self,
        msg: &wire_melt::MeltQuoteOnchainResponseBody,
    ) -> Result<(String, secp256k1::schnorr::Signature)>;
    async fn verify_onchain_address(
        &self,
        address: bitcoin::Address<bitcoin::address::NetworkUnchecked>,
    ) -> Result<bitcoin::Address>;
    async fn melt_onchain(
        &self,
        qid: Uuid,
        amount: bitcoin::Amount,
        address: bitcoin::Address,
        inputs: Vec<cashu::Proof>,
        commitment: secp256k1::schnorr::Signature,
        attestation: IssuanceAttestation,
    ) -> Result<wire_melt::MeltTx>;
    async fn fetch_mint_signatures(
        &self,
        qid: Uuid,
        mint_id: secp256k1::PublicKey,
    ) -> Result<Vec<cashu::BlindSignature>>;
    async fn verify_attestation(
        &self,
        alpha_id: &PublicKey,
        inputs: &[cashu::Proof],
        attestation: &IssuanceAttestation,
    ) -> Result<()>;
}

#[derive(Debug, Clone, strum::EnumDiscriminants, serde::Serialize, serde::Deserialize)]
#[strum_discriminants(derive(serde::Serialize, serde::Deserialize, strum::Display))]
#[serde(tag = "status")]
pub enum MintStatus {
    Pending {
        blinds: Vec<cashu::BlindedMessage>,
    },
    Paid {
        signatures: Vec<cashu::BlindSignature>,
    },
    Expired,
}
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct OnChainMintOperation {
    pub qid: Uuid,
    pub kid: cashu::Id,
    pub recipient: bitcoin::Address<bitcoin::address::NetworkUnchecked>,
    pub target: bitcoin::Amount,
    pub expiry: TStamp,
    pub status: MintStatus,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum MeltStatus {
    Pending,
    Paid { tx: wire_melt::MeltTx },
}
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct OnchainMeltOperation {
    pub qid: Uuid,
    pub address: String,
    pub amount: bitcoin::Amount,
    pub expiry: TStamp,
    pub fees: bitcoin::Amount,
    pub wallet_key: cashu::PublicKey,
    pub commitment: secp256k1::schnorr::Signature,
    pub status: MeltStatus,
}

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait Repository: Send + Sync {
    async fn store_quote(&self, quote: MintQuote) -> Result<()>;
    async fn update_quote(&self, quote: MintQuote) -> Result<()>;
    async fn list_quotes(&self) -> Result<Vec<MintQuote>>;

    async fn store_onchain_mintop(&self, op: OnChainMintOperation) -> Result<()>;
    async fn load_onchain_mintop(&self, qid: Uuid) -> Result<OnChainMintOperation>;
    async fn update_onchain_mintop_status(&self, qid: Uuid, status: MintStatus) -> Result<()>;
    async fn list_onchain_pending_mintops(&self) -> Result<Vec<Uuid>>;
    async fn store_onchain_meltop(&self, op: OnchainMeltOperation) -> Result<()>;
    async fn load_onchain_meltop(&self, qid: Uuid) -> Result<OnchainMeltOperation>;
    async fn update_onchain_meltop_status(&self, qid: Uuid, status: MeltStatus) -> Result<()>;
}
