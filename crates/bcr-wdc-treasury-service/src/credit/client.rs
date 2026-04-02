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
    core,
    wire::{bill as wire_bill, clowder as wire_clowder},
};
use clwdr_client::ClowderNatsClient;
use uuid::Uuid;
// ----- local imports
use crate::{
    credit,
    error::{Error, Result},
    TStamp,
};

// ----- end imports

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
    async fn request_to_pay_ebill(&self, rqid: Uuid, bid: core::BillId) -> Result<()> {
        tracing::debug!("DummyClwdr: request_to_pay_ebill called with rqid={rqid} and bid={bid}");
        Ok(())
    }
}

pub struct ClwdrCl(Arc<ClowderNatsClient>);
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
            .0
            .mint_bill(request, response)
            .await
            .map_err(Error::ClowderClient)?;
        Ok(res.signatures)
    }

    async fn request_to_pay_ebill(&self, rqid: Uuid, bid: core::BillId) -> Result<()> {
        let req = wire_clowder::messages::BillPaymentRequest {
            bill_id: bid,
            payment_clowder_quote: rqid,
        };
        let resp = wire_clowder::messages::BillPaymentResponse {};
        let _resp = self.0.pay_bill(req, resp).await?;
        Ok(())
    }
}
pub fn new_clowder_client(cl: Option<Arc<ClowderNatsClient>>) -> Box<dyn credit::ClowderClient> {
    let cl: Box<dyn credit::ClowderClient> = match cl {
        None => Box::new(DummyClwdr {}),
        Some(cl) => Box::new(ClwdrCl(cl)),
    };
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

    async fn request_to_pay(&self, bill_id: core::BillId, deadline: TStamp) -> Result<()> {
        let request = wire_bill::RequestToPayBitcreditBillPayload {
            bill_id,
            deadline,
            currency: CoreClient::debit_unit().to_string(),
            payment_address: todo!("payment_address not yet plumbed through"),
        };
        self.ebill.request_to_pay_bill(&request).await?;
        Ok(())
    }

    async fn is_bill_paid(&self, bill_id: core::BillId) -> Result<bool> {
        let status = self.ebill.get_payment_status(bill_id).await?;
        Ok(status.payment_status.paid)
    }
}
