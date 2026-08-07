// ----- standard library imports
use std::sync::Arc;
// ----- extra library imports
use async_trait::async_trait;
use bcr_common::{
    cashu, cdk_common, client::clowder::ClowderNatsClient, core::BillId, ecash,
    wire::clowder as wire_clowder,
};
// ----- local imports
use crate::error::Result;
// ----- local modules
pub mod factory;
pub mod service;

// ----- end imports

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MintKeySetInfo {
    pub id: cashu::Id,
    pub unit: cashu::CurrencyUnit,
    pub active: bool,
    pub valid_from: u64,
    pub derivation_path: bitcoin::bip32::DerivationPath,
    pub derivation_path_index: Option<u32>,
    pub amounts: Vec<u64>,
    pub input_fee_ppk: u64,
    pub final_expiry: Option<u64>,
}

impl std::convert::From<cdk_common::mint::MintKeySetInfo> for MintKeySetInfo {
    fn from(info: cdk_common::mint::MintKeySetInfo) -> Self {
        Self {
            id: info.id,
            unit: info.unit,
            active: info.active,
            valid_from: info.valid_from,
            derivation_path: info.derivation_path,
            derivation_path_index: info.derivation_path_index,
            amounts: info.amounts,
            input_fee_ppk: info.input_fee_ppk,
            final_expiry: info.final_expiry,
        }
    }
}

impl std::convert::From<MintKeySetInfo> for ecash::KeySetInfo {
    fn from(info: MintKeySetInfo) -> Self {
        Self {
            id: info.id,
            unit: info.unit,
            active: info.active,
            input_fee_ppk: info.input_fee_ppk,
            final_expiry: info.final_expiry,
        }
    }
}

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait ClowderClient: Send + Sync {
    async fn mint_ebill(
        &self,
        keyset_id: cashu::Id,
        quote_id: uuid::Uuid,
        amount: cashu::Amount,
        bill_id: BillId,
        signatures: Vec<cashu::BlindSignature>,
    ) -> Result<Vec<cashu::BlindSignature>>;
    async fn new_keyset(&self, keyset: cashu::KeySet) -> Result<()>;
}

pub struct ClowderCl {
    pub nats: Arc<ClowderNatsClient>,
}

#[async_trait]
impl ClowderClient for ClowderCl {
    async fn new_keyset(&self, keyset: cashu::KeySet) -> Result<()> {
        let req = wire_clowder::KeysetCreationRequest {
            id: keyset.id,
            expiry: keyset.final_expiry.unwrap_or(0_u64),
            unit: keyset.unit.clone(),
        };
        let resp = wire_clowder::KeysetCreationResponse {
            public_keys: keyset.keys.keys().clone(),
            id: keyset.id,
            expiry: keyset.final_expiry.unwrap_or(0_u64),
            unit: keyset.unit,
        };
        self.nats.new_keyset(req, resp).await?;
        Ok(())
    }

    async fn mint_ebill(
        &self,
        keyset_id: cashu::Id,
        quote_id: uuid::Uuid,
        amount: cashu::Amount,
        bill_id: BillId,
        signatures: Vec<cashu::BlindSignature>,
    ) -> Result<Vec<cashu::BlindSignature>> {
        let resp = self
            .nats
            .mint_bill(
                wire_clowder::MintEbillRequest {
                    amount,
                    keyset_id,
                    quote_id,
                    bill_id,
                },
                wire_clowder::MintEbillResponse { signatures },
            )
            .await?;
        Ok(resp.signatures)
    }
}

pub struct DummyClowderClient;

#[async_trait]
impl ClowderClient for DummyClowderClient {
    async fn new_keyset(&self, keyset: cashu::KeySet) -> Result<()> {
        tracing::debug!("DummyClowderClient::new_keyset for kid {}", keyset.id);

        Ok(())
    }
    async fn mint_ebill(
        &self,
        _keyset_id: cashu::Id,
        _quote_id: uuid::Uuid,
        _amount: cashu::Amount,
        _bill_id: BillId,
        signatures: Vec<cashu::BlindSignature>,
    ) -> Result<Vec<cashu::BlindSignature>> {
        Ok(signatures)
    }
}
