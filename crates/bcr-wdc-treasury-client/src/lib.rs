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
    #[cfg(feature = "authorized")]
    auth: bcr_wdc_utils::client::AuthorizationPlugin,
}

impl TreasuryClient {
    pub fn new(base: reqwest::Url) -> Self {
        Self {
            cl: reqwest::Client::new(),
            base,
            #[cfg(feature = "authorized")]
            auth: Default::default(),
        }
    }

    #[cfg(feature = "authorized")]
    pub async fn authenticate(
        &mut self,
        token_url: Url,
        client_id: &str,
        client_secret: &str,
        username: &str,
        password: &str,
    ) -> Result<()> {
        self.auth
            .authenticate(
                self.cl.clone(),
                token_url,
                client_id,
                client_secret,
                username,
                password,
            )
            .await?;
        Ok(())
    }

    #[cfg(feature = "authorized")]
    pub async fn generate_blinds(
        &self,
        kid: cdk02::Id,
        amount: Amount,
    ) -> Result<(Uuid, Vec<cdk00::BlindedMessage>)> {
        let msg = web_signatures::GenerateBlindedMessagesRequest { kid, total: amount };
        let url = self
            .base
            .join("/v1/admin/treasury/credit/generate_blinds")
            .expect("generate_blinds relative path");
        let request = self.cl.post(url).json(&msg);
        let response: web_signatures::GenerateBlindedMessagesResponse =
            self.auth.authorize(request).send().await?.json().await?;
        Ok((response.request_id, response.messages))
    }

    #[cfg(feature = "authorized")]
    pub async fn store_signatures(
        &self,
        rid: uuid::Uuid,
        expiration: chrono::DateTime<chrono::Utc>,
        signatures: Vec<cdk00::BlindSignature>,
    ) -> Result<()> {
        let msg = web_signatures::StoreBlindSignaturesRequest {
            rid,
            expiration,
            signatures,
        };
        let url = self
            .base
            .join("/v1/admin/treasury/credit/store_signatures")
            .expect("store_signatures relative path");
        let request = self.cl.post(url).json(&msg);
        let response = self.auth.authorize(request).send().await?;
        response.error_for_status()?;
        Ok(())
    }

    pub async fn redeem(
        &self,
        inputs: Vec<cdk00::Proof>,
        outputs: Vec<cdk00::BlindedMessage>,
    ) -> Result<Vec<cdk00::BlindSignature>> {
        let msg = cdk03::SwapRequest::new(inputs, outputs);
        let url = self
            .base
            .join("/v1/treasury/redeem")
            .expect("redeem relative path");
        let request = self.cl.post(url).json(&msg);
        let response: cdk03::SwapResponse = request.send().await?.json().await?;
        Ok(response.signatures)
    }

    #[cfg(feature = "authorized")]
    pub async fn crsat_balance(&self) -> Result<web_wallet::ECashBalance> {
        let url = self
            .base
            .join("/v1/admin/treasury/credit/balance")
            .expect("crsat balance relative path");
        let request = self.cl.get(url);
        let response: web_wallet::ECashBalance =
            self.auth.authorize(request).send().await?.json().await?;
        Ok(response)
    }

    #[cfg(feature = "authorized")]
    pub async fn sat_balance(&self) -> Result<web_wallet::ECashBalance> {
        let url = self
            .base
            .join("/v1/admin/treasury/debit/balance")
            .expect("sat balance relative path");
        let request = self.cl.get(url);
        let response: web_wallet::ECashBalance =
            self.auth.authorize(request).send().await?.json().await?;
        Ok(response)
    }
}
