// ----- standard library imports
// ----- extra library imports
use async_trait::async_trait;
use bcr_wdc_key_client::KeyClient;
use bcr_wdc_keys::KeysetID;
use cashu::nuts::nut00 as cdk00;
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
    pub fn new(cfg: KeysRestConfig) -> Result<Self> {
        let cl = KeyClient::new(cfg.base_url).map_err(Error::KeysHandler)?;
        Ok(Self(cl))
    }
}

#[async_trait]
impl KeysHandler for KeysRestHandler {
    async fn sign(
        &self,
        kid: KeysetID,
        qid: Uuid,
        maturity_date: TStamp,
        msg: &cdk00::BlindedMessage,
    ) -> Result<cdk00::BlindSignature> {
        self.0
            .pre_sign(kid.into(), qid, maturity_date, msg)
            .await
            .map_err(Error::KeysHandler)
    }
}
