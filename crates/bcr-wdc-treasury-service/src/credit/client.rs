// ----- standard library imports
use std::sync::Arc;
// ----- extra library imports
use async_trait::async_trait;
use bcr_common::{
    cashu,
    client::{
        core::{Client as CoreClient, Error as CoreError},
        ebill::Client as EbillClient,
    },
    clowder::taproot,
    clwdr_client::{ClowderNatsClient, ClowderRestClient},
    core::{self, BillId},
    wire::{bill as wire_bill, clowder as wire_clowder},
};
use bitcoin::hashes::Hash;
use uuid::Uuid;
// ----- local imports
use crate::{
    credit,
    error::{Error, Result},
    TStamp,
};

// ----- end imports

#[allow(dead_code)]
pub struct DummyClwdr {}
#[async_trait]
impl credit::ClowderClient for DummyClwdr {
    async fn minting_ebill(
        &self,
        _keyset_id: cashu::Id,
        _quote_id: Uuid,
        _amount: cashu::Amount,
        _bill_id: core::BillId,
        signatures: Vec<cashu::BlindSignature>,
    ) -> Result<Vec<cashu::BlindSignature>> {
        Ok(signatures)
    }
    async fn request_to_pay_ebill(
        &self,
        bid: BillId,
        _payment_address: bitcoin::Address,
        _block_id: u64,
        _previous_block_hash: bitcoin::hashes::sha256::Hash,
        _amount: bitcoin::Amount,
    ) -> Result<()> {
        tracing::debug!("DummyClwdr: request_to_pay_ebill called for bid={bid}");
        Ok(())
    }
    async fn request_onchain_ebill_address(
        &self,
        _bid: BillId,
        _block_id: u64,
        _previous_block_hash: bitcoin::hashes::sha256::Hash,
    ) -> Result<bitcoin::Address> {
        return Err(Error::ClowderUnavailable);
    }
}

pub struct ClwdrCl {
    pub rest: Arc<ClowderRestClient>,
    pub nats: Arc<ClowderNatsClient>,
}

#[async_trait]
impl credit::ClowderClient for ClwdrCl {
    async fn minting_ebill(
        &self,
        keyset_id: cashu::Id,
        quote_id: Uuid,
        amount: cashu::Amount,
        bill_id: core::BillId,
        signatures: Vec<cashu::BlindSignature>,
    ) -> Result<Vec<cashu::BlindSignature>> {
        let request = wire_clowder::messages::MintEbillRequest {
            keyset_id,
            amount,
            bill_id,
            quote_id,
        };
        let response = wire_clowder::messages::MintEbillResponse { signatures };
        let res = self
            .nats
            .mint_bill(request, response)
            .await
            .map_err(Error::ClowderClient)?;
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
        let req = wire_clowder::messages::RequestToPayEbillRequest {
            bill_id: bid,
            payment_address: payment_address.into_unchecked(),
            block_id,
            previous_block_hash,
            amount,
        };
        let resp = wire_clowder::messages::RequestToPayEbillResponse {};
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
        let alpha_key = info.node_id.x_only_public_key();
        let frost_agg_key = info.multisig_agg_xonly;
        let derived_address = taproot::derive_ebill_mint_req_to_pay_address(
            &frost_agg_key,
            &alpha_key,
            144, // will be removed in the future
            &bid,
            block_id,
            previous_block_hash.as_byte_array(),
            network,
        )
        .map_err(|e| Error::Internal(e.to_string()))?;
        Ok(derived_address)
    }
}
pub fn new_clowder_client(
    nats_cl: Arc<ClowderNatsClient>,
    rest_cl: Arc<ClowderRestClient>,
) -> Box<dyn credit::ClowderClient> {
    let cl: Box<dyn credit::ClowderClient> = Box::new(ClwdrCl {
        nats: nats_cl,
        rest: rest_cl,
    });
    cl
}

pub struct WildcatCl {
    pub core: Arc<CoreClient>,
    pub ebill: Box<EbillClient>,
}

#[async_trait]
impl credit::WildcatClient for WildcatCl {
    async fn info(&self, kid: cashu::Id) -> Result<cashu::KeySetInfo> {
        match self.core.keyset_info(kid).await {
            Ok(info) => Ok(info),
            Err(CoreError::KeysetIdNotFound(kid)) => {
                Err(Error::InvalidInput(format!("Unknown keyset: {kid}")))
            }
            Err(e) => Err(e.into()),
        }
    }

    async fn sign(&self, blinds: &[cashu::BlindedMessage]) -> Result<Vec<cashu::BlindSignature>> {
        let res = self.core.sign(blinds).await?;
        Ok(res)
    }

    async fn burn(&self, proofs: Vec<cashu::Proof>) -> Result<()> {
        self.core.burn(proofs).await?;
        Ok(())
    }

    async fn recover(&self, proofs: Vec<cashu::Proof>) -> Result<()> {
        self.core.recover(proofs).await?;
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
