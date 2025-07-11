// ----- standard library imports
// ----- extra library imports
use async_trait::async_trait;
use bcr_wdc_ebill_client::{EbillClient, Url};
use bcr_wdc_webapi::quotes::SharedBill;
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
    async fn validate_and_decrypt_shared_bill(
        &self,
        shared_bill: &SharedBill,
    ) -> Result<bcr_wdc_webapi::quotes::BillInfo> {
        self.0
            .validate_and_decrypt_shared_bill(shared_bill)
            .await
            .map_err(Error::EbillClient)
    }
}

#[cfg(feature = "test-utils")]
pub mod test_utils {
    use bcr_wdc_webapi::test_utils::{
        holder_key_pair, node_id_from_pub_key, random_date, random_identity_public_data,
    };

    use super::*;

    #[derive(Clone, Debug, Default)]
    pub struct DummyEbillNode {}

    #[async_trait]
    impl EBillNode for DummyEbillNode {
        async fn validate_and_decrypt_shared_bill(
            &self,
            shared_bill: &SharedBill,
        ) -> Result<bcr_wdc_webapi::quotes::BillInfo> {
            let mut payee = random_identity_public_data().1;
            payee.node_id = node_id_from_pub_key(holder_key_pair().public_key());

            Ok(bcr_wdc_webapi::quotes::BillInfo {
                id: shared_bill.bill_id.clone(),
                drawee: random_identity_public_data().1,
                drawer: random_identity_public_data().1,
                payee: bcr_wdc_webapi::bill::BillParticipant::Ident(payee),
                endorsees: vec![],
                sum: 100,
                maturity_date: random_date(),
                file_urls: shared_bill.file_urls.clone(),
            })
        }
    }
}
