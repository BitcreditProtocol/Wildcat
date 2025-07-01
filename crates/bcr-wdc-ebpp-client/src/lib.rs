// ----- standard library imports
// ----- extra library imports
use bcr_wdc_webapi::wallet as web_wallet;
use thiserror::Error;
// ----- local imports
pub use reqwest::Url;

// ----- end imports

pub type Result<T> = std::result::Result<T, Error>;
#[derive(Debug, Error)]
pub enum Error {
    #[error("internal error {0}")]
    Reqwest(#[from] reqwest::Error),
}

#[derive(Debug, Clone)]
pub struct EBPPClient {
    cl: reqwest::Client,
    base: reqwest::Url,
    #[cfg(feature = "authorized")]
    auth: bcr_wdc_utils::client::AuthorizationPlugin,
}

impl EBPPClient {
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
    pub async fn balance(&self) -> Result<bdk_wallet::Balance> {
        let url = self
            .base
            .join("/v1/admin/ebpp/onchain/balance")
            .expect("balance relative path");
        let request = self.cl.get(url);
        let response: web_wallet::Balance =
            self.auth.authorize(request).send().await?.json().await?;
        Ok(bdk_wallet::Balance::from(response))
    }
}
