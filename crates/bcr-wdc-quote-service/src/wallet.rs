// ----- standard library imports
// ----- extra library imports
use async_trait::async_trait;
use bcr_wdc_treasury_client::TreasuryClient;
use bcr_wdc_utils::id::KeysetID;
use cashu::nuts::nut00 as cdk00;
use cashu::Amount;
use uuid::Uuid;
// ----- local imports
use crate::error::{Error, Result};
use crate::service::Wallet;
use crate::TStamp;

#[derive(Debug, Clone, serde::Deserialize)]
pub struct WalletConfig {
    base_url: bcr_wdc_treasury_client::Url,
}

#[derive(Clone, Debug)]
pub struct Client {
    cl: TreasuryClient,
}

impl Client {
    pub fn new(cfg: WalletConfig) -> Self {
        let WalletConfig { base_url } = cfg;
        let cl = TreasuryClient::new(base_url);
        Self { cl }
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
