// ----- standard library imports
// ----- extra library imports
use bcr_wdc_webapi::signatures as web_signatures;
use cashu::amount::Amount;
use cashu::nut00 as cdk00;
use cashu::nut02 as cdk02;
use thiserror::Error;
use uuid::Uuid;
// ----- local modules
// ----- local imports

pub type Result<T> = std::result::Result<T, Error>;
#[derive(Debug, Error)]
pub enum Error {
    #[error("URL parse error {0}")]
    Url(#[from] url::ParseError),

    #[error("internal error {0}")]
    Reqwest(#[from] reqwest::Error),
}

#[derive(Debug, Clone)]
pub struct TreasuryClient {
    cl: reqwest::Client,
    base: reqwest::Url,
}

impl TreasuryClient {
    pub fn new(base: &str) -> Result<Self> {
        let url = reqwest::Url::parse(base)?;
        Ok(Self {
            cl: reqwest::Client::new(),
            base: url,
        })
    }

    pub async fn generate_blinds(
        &self,
        kid: cdk02::Id,
        amount: Amount,
    ) -> Result<(Uuid, Vec<cdk00::BlindedMessage>)> {
        let request = web_signatures::GenerateBlindedMessagesRequest { kid, total: amount };
        let url = self.base.join("/v1/credit/generate_blinds")?;
        let res = self.cl.post(url).json(&request).send().await?;
        let response: web_signatures::GenerateBlindedMessagesResponse = res.json().await?;
        Ok((response.rid, response.messages))
    }

    pub async fn store_signatures(
        &self,
        rid: uuid::Uuid,
        expiration: chrono::DateTime<chrono::Utc>,
        signatures: Vec<cdk00::BlindSignature>,
    ) -> Result<()> {
        let request = web_signatures::StoreBlindedSignaturesRequest {
            rid,
            expiration,
            signatures,
        };
        let url = self.base.join("/v1/credit/store_signatures")?;
        let res = self.cl.post(url).json(&request).send().await?;
        res.error_for_status()?;
        Ok(())
    }
}
