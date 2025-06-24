// ----- standard library imports
// ----- extra library imports
use async_trait::async_trait;
use bcr_wdc_ebill_client::{EbillClient, Url};
use bcr_wdc_webapi::bill::BillId;
use bdk_wallet::bitcoin::Amount;
// ----- local imports
use crate::error::{Error, Result};
use crate::service::EBillNode;

// ----- end imports

#[derive(Debug, Clone, serde::Deserialize)]
pub struct EBillClientConfig {
    base_url: Url,
}

#[derive(Debug, Clone)]
pub struct EBillClient(EbillClient);
impl EBillClient {
    pub fn new(cfg: EBillClientConfig) -> Self {
        let cl = EbillClient::new(cfg.base_url);
        Self(cl)
    }
}

#[async_trait]
impl EBillNode for EBillClient {
    async fn request_to_pay(&self, bill: &BillId, amount: Amount) -> Result<String> {
        tracing::info!(
            "EBillClient: request_to_pay called with bill: {}, amount: {}",
            bill,
            amount
        );
        let request = bcr_wdc_webapi::bill::RequestToPayBitcreditBillPayload {
            bill_id: bill.to_owned(),
            currency: String::from("sat"),
        };
        self.0
            .request_to_pay_bill(&request)
            .await
            .map_err(Error::EBillClient)?;
        let bcr_wdc_webapi::bill::BillCombinedBitcoinKey { private_descriptor } = self
            .0
            .get_bitcoin_private_descriptor_for_bill(bill)
            .await
            .map_err(Error::EBillClient)?;
        if private_descriptor.is_empty() {
            return Err(Error::InvalidDescriptor(private_descriptor));
        }
        Ok(private_descriptor)
    }
}
