// ----- standard library imports
// ----- extra library imports
use async_trait::async_trait;
use bcr_wdc_keys::KeysetID;
use bcr_wdc_treasury_client::TreasuryClient;
use cashu::nuts::nut00 as cdk00;
use cashu::Amount;
use uuid::Uuid;
// ----- local imports
use crate::error::{Error, Result};
use crate::service::Wallet;
use crate::TStamp;

#[derive(Debug, Default, Clone, serde::Deserialize)]
pub struct WalletConfig {
    base_url: String,
}

#[derive(Clone, Debug)]
pub struct Client {
    cl: TreasuryClient,
}

impl Client {
    pub fn new(cfg: &WalletConfig) -> Result<Self> {
        let cl = TreasuryClient::new(&cfg.base_url).map_err(Error::Wallet)?;
        Ok(Self { cl })
    }
}
#[async_trait]
impl Wallet for Client {
    async fn get_blinds(
        &self,
        kid: KeysetID,
        amount: Amount,
    ) -> Result<(Uuid, Vec<cdk00::BlindedMessage>)> {
        self.cl
            .generate_blinds(kid.into(), amount)
            .await
            .map_err(Error::Wallet)
    }

    async fn store_signatures(
        &self,
        rid: Uuid,
        expiration: TStamp,
        signatures: Vec<cdk00::BlindSignature>,
    ) -> Result<()> {
        self.cl
            .store_signatures(rid, expiration, signatures)
            .await
            .map_err(Error::Wallet)
    }
}
