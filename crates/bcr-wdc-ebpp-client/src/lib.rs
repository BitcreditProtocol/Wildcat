// ----- standard library imports
// ----- extra library imports
use bcr_common::wire::wallet as wire_wallet;
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
}

impl EBPPClient {
    pub fn new(base: reqwest::Url) -> Self {
        Self {
            cl: reqwest::Client::new(),
            base,
        }
    }

    pub async fn balance(&self) -> Result<bdk_wallet::Balance> {
        let url = self
            .base
            .join("/v1/admin/ebpp/onchain/balance")
            .expect("balance relative path");
        let request = self.cl.get(url);
        let response: wire_wallet::Balance = request.send().await?.json().await?;
        Ok(bdk_wallet::Balance::from(response))
    }

    pub async fn network(&self) -> Result<bdk_wallet::bitcoin::Network> {
        let url = self
            .base
            .join("/v1/ebpp/onchain/network")
            .expect("network relative path");
        let request = self.cl.get(url);
        let response: wire_wallet::Network = request.send().await?.json().await?;
        Ok(response.network)
    }
}
