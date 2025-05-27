// ----- standard library imports
// ----- extra library imports
use bcr_wdc_webapi::{signatures as web_signatures, wallet as web_wallet};
use cashu::{nut00 as cdk00, nut02 as cdk02, nut03 as cdk03, Amount};
use thiserror::Error;
use uuid::Uuid;
// ----- local modules
// ----- local imports
pub use reqwest::Url;

pub type Result<T> = std::result::Result<T, Error>;
#[derive(Debug, Error)]
pub enum Error {
    #[error("internal error {0}")]
    Reqwest(#[from] reqwest::Error),
}

#[derive(Debug, Clone)]
pub struct TreasuryClient {
    cl: reqwest::Client,
    base: reqwest::Url,
}

impl TreasuryClient {
    pub fn new(base: reqwest::Url) -> Self {
        Self {
            cl: reqwest::Client::new(),
            base,
        }
    }

    pub async fn generate_blinds(
        &self,
        kid: cdk02::Id,
        amount: Amount,
    ) -> Result<(Uuid, Vec<cdk00::BlindedMessage>)> {
        let request = web_signatures::GenerateBlindedMessagesRequest { kid, total: amount };
        let url = self
            .base
            .join("/v1/credit/generate_blinds")
            .expect("generate_blinds relative path");
        let res = self.cl.post(url).json(&request).send().await?;
        let response: web_signatures::GenerateBlindedMessagesResponse = res.json().await?;
        Ok((response.request_id, response.messages))
    }

    pub async fn store_signatures(
        &self,
        rid: uuid::Uuid,
        expiration: chrono::DateTime<chrono::Utc>,
        signatures: Vec<cdk00::BlindSignature>,
    ) -> Result<()> {
        let request = web_signatures::StoreBlindSignaturesRequest {
            rid,
            expiration,
            signatures,
        };
        let url = self
            .base
            .join("/v1/credit/store_signatures")
            .expect("store_signatures relative path");
        let res = self.cl.post(url).json(&request).send().await?;
        res.error_for_status()?;
        Ok(())
    }

    pub async fn redeem(
        &self,
        inputs: Vec<cdk00::Proof>,
        outputs: Vec<cdk00::BlindedMessage>,
    ) -> Result<Vec<cdk00::BlindSignature>> {
        let request = cdk03::SwapRequest::new(inputs, outputs);
        let url = self
            .base
            .join("/v1/debit/redeem")
            .expect("redeem relative path");
        let res = self.cl.post(url).json(&request).send().await?;
        let response: cdk03::SwapResponse = res.json().await?;
        Ok(response.signatures)
    }

    pub async fn crsat_balance(&self) -> Result<web_wallet::ECashBalance> {
        let url = self
            .base
            .join("/v1/balance/credit")
            .expect("crsat balance relative path");
        let res = self.cl.get(url).send().await?;
        let response: web_wallet::ECashBalance = res.json().await?;
        Ok(response)
    }

    pub async fn sat_balance(&self) -> Result<web_wallet::ECashBalance> {
        let url = self
            .base
            .join("/v1/balance/debit")
            .expect("sat balance relative path");
        let res = self.cl.get(url).send().await?;
        let response: web_wallet::ECashBalance = res.json().await?;
        Ok(response)
    }
}
