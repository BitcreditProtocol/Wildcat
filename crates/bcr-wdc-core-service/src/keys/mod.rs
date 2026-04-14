// ----- standard library imports
use std::sync::Arc;
// ----- extra library imports
use async_trait::async_trait;
use bcr_common::{cashu, core::BillId, wire::clowder::messages as wire_clowder};
use clwdr_client::ClowderNatsClient;
// ----- local imports
use crate::error::Result;
// ----- local modules
pub mod factory;
pub mod service;

// ----- end imports

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
    async fn keyset_deactivated(&self, kid: cashu::Id) -> Result<()>;
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

    async fn keyset_deactivated(&self, kid: cashu::Id) -> Result<()> {
        self.nats
            .deactivate_keyset(wire_clowder::KeysetDeactivationRequest { id: kid })
            .await?;
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
    async fn keyset_deactivated(&self, kid: cashu::Id) -> Result<()> {
        tracing::debug!("DummyClowderClient::keyset_deactivated for kid {}", kid);

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
