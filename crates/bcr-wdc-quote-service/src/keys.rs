// ----- standard library imports
// ----- extra library imports
use async_trait::async_trait;
use bcr_wdc_key_client::KeyClient;
use cashu::{nut00 as cdk00, nut01 as cdk01, nut02 as cdk02};
use uuid::Uuid;
// ----- local modules
// ----- local imports
use crate::error::{Error, Result};
use crate::service::KeysHandler;
use crate::TStamp;

#[derive(Debug, Clone, serde::Deserialize)]
pub struct KeysRestConfig {
    pub base_url: bcr_wdc_key_client::Url,
}

#[derive(Debug, Clone)]
pub struct KeysRestHandler(KeyClient);

impl KeysRestHandler {
    pub fn new(cfg: KeysRestConfig) -> Self {
        let cl = KeyClient::new(cfg.base_url);
        Self(cl)
    }
}

#[async_trait]
impl KeysHandler for KeysRestHandler {
    async fn generate(
        &self,
        qid: Uuid,
        amount: bitcoin::Amount,
        pk: cdk01::PublicKey,
        maturity_date: TStamp,
    ) -> Result<cdk02::Id> {
        let amount = cashu::Amount::from(amount.to_sat());
        self.0
            .generate_keyset(qid, amount, pk, maturity_date)
            .await
            .map_err(Error::KeysHandler)
    }
    async fn sign(&self, qid: Uuid, msg: &cdk00::BlindedMessage) -> Result<cdk00::BlindSignature> {
        self.0.pre_sign(qid, msg).await.map_err(Error::KeysHandler)
    }
}
