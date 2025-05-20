// ----- standard library imports
// ----- extra library imports
use async_trait::async_trait;
use bcr_wdc_ebill_client::{EbillClient, Url};
use bdk_wallet::{
    bitcoin::{Amount, PrivateKey},
    keys::KeyMap,
    miniscript::descriptor,
};
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
    async fn request_to_pay(&self, bill: &str, amount: Amount) -> Result<String> {
        tracing::info!(
            "EBillClient: request_to_pay called with bill: {}, amount: {}",
            bill,
            amount
        );
        let request = bcr_wdc_webapi::bill::RequestToPayBitcreditBillPayload {
            bill_id: String::from(bill),
            currency: String::from("sat"),
        };
        self.0
            .request_to_pay_bill(&request)
            .await
            .map_err(Error::EBillClient)?;
        let bcr_wdc_webapi::bill::BillCombinedBitcoinKey { private_key } = self
            .0
            .get_bitcoin_private_key_for_bill(bill)
            .await
            .map_err(Error::EBillClient)?;
        let priv_key = PrivateKey::from_wif(&private_key).map_err(Error::BTCWif)?;
        let single = descriptor::SinglePriv {
            key: priv_key,
            origin: None,
        };
        let secret_descriptor = descriptor::DescriptorSecretKey::Single(single);
        let pub_descriptor = secret_descriptor
            .to_public(secp256k1::global::SECP256K1)
            .expect("invalid single secret descriptor");
        let kmap = KeyMap::from_iter(std::iter::once((pub_descriptor.clone(), secret_descriptor)));
        let descriptor = descriptor::Descriptor::new_pkh(pub_descriptor).expect("invalid pubkey");
        Ok(descriptor.to_string_with_secret(&kmap))
    }
}
