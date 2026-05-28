// ----- standard library imports
// ----- extra library imports
use async_trait::async_trait;
use bcr_common::{
    cashu,
    wire::{
        attestation::IssuanceAttestation, clowder as wire_clowder, keys as wire_keys,
        melt as wire_melt, mint as wire_mint,
    },
};
use bitcoin::secp256k1::PublicKey;
use uuid::Uuid;
// ----- local modules
mod clients;
mod monitor;
mod service;
// ----- local imports
use crate::{error::Result, TStamp};

// ----- end imports

pub use clients::ClowderCl;
pub use clients::VaultSrvc;
pub use clients::WildcatCl;
pub use monitor::MintOpMonitor;
pub use service::Service;

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait WildcatClient: Send + Sync {
    async fn verify_fingerprints(&self, fps: &[wire_keys::ProofFingerprint]) -> Result<()>;
    async fn verify_proofs(&self, proofs: &[cashu::Proof]) -> Result<()>;
    async fn check_spendable(
        &self,
        proofs: Vec<cashu::PublicKey>,
    ) -> Result<Vec<cashu::ProofState>>;
    async fn sign(&self, blinds: Vec<cashu::BlindedMessage>) -> Result<Vec<cashu::BlindSignature>>;
    async fn burn(&self, inputs: Vec<cashu::Proof>) -> Result<()>;
    async fn recover(&self, inputs: Vec<cashu::Proof>) -> Result<()>;
    async fn keyset_info(&self, kid: cashu::Id) -> Result<cashu::KeySetInfo>;
    async fn keyset(&self, kid: cashu::Id) -> Result<cashu::KeySet>;
    async fn get_active_keyset(&self) -> Result<cashu::Id>;
}

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait ClowderClient: Send + Sync {
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
        admin_fees: bitcoin::Amount,
        network_fees: bitcoin::Amount,
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
        fees: Vec<cashu::BlindSignature>,
        commitment: secp256k1::schnorr::Signature,
        attestation: IssuanceAttestation,
    ) -> Result<bitcoin::Txid>;
    async fn fetch_mint_signatures(
        &self,
        qid: Uuid,
        mint_id: secp256k1::PublicKey,
    ) -> Result<Vec<cashu::BlindSignature>>;
    async fn estimate_onchain_fees(&self, amount: bitcoin::Amount) -> Result<bitcoin::Amount>;
    async fn get_onchain_reserve(&self) -> Result<bitcoin::Amount>;
    async fn verify_attestation(
        &self,
        alpha_id: &PublicKey,
        inputs: &[cashu::Proof],
        attestation: &IssuanceAttestation,
    ) -> Result<()>;
}

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait VaultService: Send + Sync {
    async fn store_proofs(&self, proofs: Vec<cashu::Proof>) -> Result<()>;
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
#[derive(Debug, Clone)]
pub struct MintOperation {
    pub qid: Uuid,
    pub kid: cashu::Id,
    pub recipient: bitcoin::Address<bitcoin::address::NetworkUnchecked>,
    pub target: bitcoin::Amount,
    pub expiry: TStamp,
    pub status: MintStatus,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, strum::EnumDiscriminants)]
#[serde(tag = "status")]
#[strum_discriminants(derive(serde::Serialize, serde::Deserialize, strum::Display))]
pub enum MeltStatus {
    Pending,
    Paid { tx: bitcoin::Txid },
    Expired,
}
#[derive(Debug, Clone)]
pub struct MeltOperation {
    pub qid: Uuid,
    pub address: String,
    pub target: bitcoin::Amount,
    pub available: cashu::Amount,
    pub fees: cashu::Amount,
    // network fees = available - target - fees
    pub expiry: TStamp,
    pub wallet_key: cashu::PublicKey,
    pub input_ys: Vec<cashu::PublicKey>,
    pub commitment: secp256k1::schnorr::Signature,
    pub status: MeltStatus,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DeniedMeltOperation {
    pub qid: Uuid,
    pub inputs: bitcoin::Amount,
    pub created: TStamp,
}

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait Repository: Send + Sync {
    async fn store_mintop(&self, op: MintOperation) -> Result<()>;
    async fn load_mintop(&self, qid: Uuid) -> Result<MintOperation>;
    async fn update_mintop_status(&self, qid: Uuid, status: MintStatus) -> Result<()>;
    async fn list_pending_mintops(&self, now: TStamp) -> Result<Vec<Uuid>>;
    async fn store_meltop(&self, op: MeltOperation, now: TStamp) -> Result<()>;
    async fn load_meltop(&self, qid: Uuid) -> Result<MeltOperation>;
    async fn update_meltop_status(&self, qid: Uuid, status: MeltStatus) -> Result<()>;
    async fn list_pending_meltops(&self, now: TStamp) -> Result<Vec<Uuid>>;
    async fn store_denied_meltop(&self, op: DeniedMeltOperation) -> Result<()>;
    async fn list_denied_meltops(&self) -> Result<Vec<DeniedMeltOperation>>;
    async fn delete_denied_meltop(&self, qid: Uuid) -> Result<()>;
}
