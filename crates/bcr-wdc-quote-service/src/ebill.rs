// ----- standard library imports
// ----- extra library imports
use async_trait::async_trait;
use bcr_common::{client::ebill::Client as EbillClient, wire::quotes as wire_quotes};
// ----- local imports
use crate::error::{Error, Result};
use crate::service::EBillNode;

// ----- end imports

#[derive(Debug, Clone, serde::Deserialize)]
pub struct EBillClientConfig {
    base_url: reqwest::Url,
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
        shared_bill: &wire_quotes::SharedBill,
    ) -> Result<wire_quotes::BillInfo> {
        self.0
            .validate_and_decrypt_shared_bill(shared_bill)
            .await
            .map_err(Error::EbillClient)
    }
}

#[cfg(feature = "test-utils")]
pub mod test_utils {
    use super::*;
    use bcr_common::{core_tests, wire::bill as wire_bill};
    use bcr_wdc_webapi::test_utils::{holder_key_pair, random_date, random_identity_public_data};
    use std::str::FromStr;

    #[derive(Clone, Debug, Default)]
    pub struct DummyEbillNode {}

    #[async_trait]
    impl EBillNode for DummyEbillNode {
        async fn validate_and_decrypt_shared_bill(
            &self,
            shared_bill: &wire_quotes::SharedBill,
        ) -> Result<wire_quotes::BillInfo> {
            let mut payee = random_identity_public_data().1;
            payee.node_id = core_tests::node_id_from_pub_key(holder_key_pair().public_key());

            Ok(wire_quotes::BillInfo {
                id: shared_bill.bill_id.clone(),
                drawee: random_identity_public_data().1,
                drawer: random_identity_public_data().1,
                payee: wire_bill::BillParticipant::Ident(payee),
                endorsees: vec![],
                sum: 100,
                maturity_date: chrono::NaiveDate::from_str(random_date().to_string().as_str())
                    .unwrap(),
                file_urls: shared_bill.file_urls.clone(),
            })
        }
    }
}
