// ----- standard library imports
use std::sync::Arc;
// ----- extra library imports
use async_trait::async_trait;
use bcr_common::{
    cashu,
    client::{
        admin::clowder::Client as ClowderRestClient, clowder::ClowderNatsClient,
        core::Client as CoreClient, ebill::Client as EbillClient,
    },
    clowder::taproot,
    core::{self, BillId},
    ecash,
    wire::{bill as wire_bill, clowder as wire_clowder},
};
use bitcoin::hashes::Hash;
use uuid::Uuid;
// ----- local imports
use crate::{
    ebill,
    error::{Error, Result},
    TStamp,
};

// ----- end imports

pub struct ClwdrCl {
    pub rest: Arc<ClowderRestClient>,
    pub nats: Arc<ClowderNatsClient>,
}

#[async_trait]
impl ebill::ClowderClient for ClwdrCl {
    async fn register_ebill(&self, bid: BillId, amount: cashu::Amount) -> Result<()> {
        let request = wire_clowder::RegisterEbillRequest {
            bill_id: bid,
            amount,
        };
        let response = wire_clowder::RegisterEbillResponse {};
        let _resp = self.nats.register_ebill(request, response).await?;
        Ok(())
    }

    async fn minting_ebill(
        &self,
        keyset_id: cashu::Id,
        quote_id: Uuid,
        amount: cashu::Amount,
        bill_id: core::BillId,
        signatures: Vec<cashu::BlindSignature>,
    ) -> Result<Vec<cashu::BlindSignature>> {
        let request = wire_clowder::MintEbillRequest {
            keyset_id,
            amount,
            bill_id,
            quote_id,
        };
        let response = wire_clowder::MintEbillResponse { signatures };
        let res = self.nats.mint_bill(request, response).await?;
        Ok(res.signatures)
    }

    async fn request_to_pay_ebill(
        &self,
        bid: BillId,
        payment_address: bitcoin::Address,
        block_id: u64,
        previous_block_hash: bitcoin::hashes::sha256::Hash,
        amount: bitcoin::Amount,
    ) -> Result<()> {
        let req = wire_clowder::RequestToPayEbillRequest {
            bill_id: bid,
            payment_address: payment_address.into_unchecked(),
            block_id,
            previous_block_hash,
            amount,
        };
        let resp = wire_clowder::RequestToPayEbillResponse {};
        let _resp = self.nats.request_to_pay_bill(req, resp).await?;
        Ok(())
    }

    async fn request_onchain_ebill_address(
        &self,
        bid: BillId,
        block_id: u64,
        previous_block_hash: bitcoin::hashes::sha256::Hash,
    ) -> Result<bitcoin::Address> {
        let info = self.rest.get_info().await?;
        let network = info.network;
        let frost_agg_key = info.multisig_agg_xonly;
        let derived_address = taproot::derive_ebill_mint_req_to_pay_address(
            &frost_agg_key,
            &bid,
            block_id,
            previous_block_hash.as_byte_array(),
            network,
        )
        .map_err(|e| Error::Internal(e.to_string()))?;
        Ok(derived_address)
    }
}

pub struct WildcatCl {
    /// core-service's public (web) endpoints, e.g. `keyset_info`.
    pub core: Arc<CoreClient>,
    /// core-service's admin-only endpoints, e.g. `sign`, `burn`, `recover`.
    pub core_admin: Arc<CoreClient>,
    pub ebill: Box<EbillClient>,
}

#[async_trait]
impl ebill::WildcatClient for WildcatCl {
    async fn info(&self, kid: cashu::Id) -> Result<ecash::KeySetInfo> {
        let kinfo = self.core.keyset_info(kid).await?;
        Ok(kinfo)
    }

    async fn sign(&self, blinds: &[cashu::BlindedMessage]) -> Result<Vec<cashu::BlindSignature>> {
        let c_blinds: Vec<_> = blinds.iter().cloned().map(From::from).collect();
        let res = self.core_admin.sign(&c_blinds).await?;
        Ok(res.into_iter().map(From::from).collect())
    }

    async fn burn(&self, proofs: Vec<cashu::Proof>) -> Result<()> {
        let c_proofs = proofs.into_iter().map(From::from).collect();
        self.core_admin.burn(c_proofs).await?;
        Ok(())
    }

    async fn recover(&self, proofs: Vec<cashu::Proof>) -> Result<()> {
        let c_proofs = proofs.into_iter().map(From::from).collect();
        self.core_admin.recover(c_proofs).await?;
        Ok(())
    }

    async fn prepare_request_to_pay(
        &self,
        bid: core::BillId,
    ) -> Result<(u64, bitcoin::hashes::sha256::Hash)> {
        let request = wire_bill::PrepareRequestToPayBitcreditBillPayload { bill_id: bid };
        let resp: wire_bill::PrepareRequestToPayBitcreditBillResponse =
            self.ebill.prepare_request_to_pay_bill(&request).await?;

        Ok((resp.block_id, resp.previous_block_hash))
    }

    async fn request_to_pay(
        &self,
        bill_id: core::BillId,
        deadline: TStamp,
        payment_address: bitcoin::Address,
    ) -> Result<secp256k1::SecretKey> {
        let request = wire_bill::RequestToPayBitcreditBillPayload {
            bill_id,
            deadline,
            currency: CoreClient::currency_unit().to_string(),
            payment_address: payment_address.into_unchecked(),
        };
        let resp: wire_bill::RequestToPayBitcreditBillResponse =
            self.ebill.request_to_pay_bill(&request).await?;
        Ok(resp.bill_private_key)
    }

    async fn is_bill_paid(&self, bill_id: core::BillId) -> Result<bool> {
        let status = self.ebill.get_payment_status(bill_id).await?;
        Ok(status.payment_status.paid)
    }
}

#[cfg(feature = "test-utils")]
pub struct DummyWildcatClient;

#[cfg(feature = "test-utils")]
#[async_trait]
impl ebill::WildcatClient for DummyWildcatClient {
    async fn info(&self, _kid: cashu::Id) -> Result<ecash::KeySetInfo> {
        Err(Error::Internal(String::from("test_utils dummy: not implemented")))
    }
    async fn sign(&self, _blinds: &[cashu::BlindedMessage]) -> Result<Vec<cashu::BlindSignature>> {
        Err(Error::Internal(String::from("test_utils dummy: not implemented")))
    }
    async fn burn(&self, _proofs: Vec<cashu::Proof>) -> Result<()> {
        Err(Error::Internal(String::from("test_utils dummy: not implemented")))
    }
    async fn recover(&self, _proofs: Vec<cashu::Proof>) -> Result<()> {
        Err(Error::Internal(String::from("test_utils dummy: not implemented")))
    }
    async fn prepare_request_to_pay(
        &self,
        _bid: BillId,
    ) -> Result<(u64, bitcoin::hashes::sha256::Hash)> {
        Err(Error::Internal(String::from("test_utils dummy: not implemented")))
    }
    async fn request_to_pay(
        &self,
        _bid: BillId,
        _expire: TStamp,
        _payment_address: bitcoin::Address,
    ) -> Result<secp256k1::SecretKey> {
        Err(Error::Internal(String::from("test_utils dummy: not implemented")))
    }
    async fn is_bill_paid(&self, _bid: BillId) -> Result<bool> {
        Err(Error::Internal(String::from("test_utils dummy: not implemented")))
    }
}

#[cfg(feature = "test-utils")]
pub struct DummyClowderClient;

#[cfg(feature = "test-utils")]
#[async_trait]
impl ebill::ClowderClient for DummyClowderClient {
    async fn register_ebill(&self, _bid: BillId, _amount: cashu::Amount) -> Result<()> {
        Err(Error::Internal(String::from("test_utils dummy: not implemented")))
    }
    async fn minting_ebill(
        &self,
        _kid: cashu::Id,
        _qid: Uuid,
        _amount: cashu::Amount,
        _bid: BillId,
        _signs: Vec<cashu::BlindSignature>,
    ) -> Result<Vec<cashu::BlindSignature>> {
        Err(Error::Internal(String::from("test_utils dummy: not implemented")))
    }
    async fn request_to_pay_ebill(
        &self,
        _bid: BillId,
        _payment_address: bitcoin::Address,
        _block_id: u64,
        _previous_block_hash: bitcoin::hashes::sha256::Hash,
        _amount: bitcoin::Amount,
    ) -> Result<()> {
        Err(Error::Internal(String::from("test_utils dummy: not implemented")))
    }
    async fn request_onchain_ebill_address(
        &self,
        _bid: BillId,
        _block_id: u64,
        _previous_block_hash: bitcoin::hashes::sha256::Hash,
    ) -> Result<bitcoin::Address> {
        Err(Error::Internal(String::from("test_utils dummy: not implemented")))
    }
}
