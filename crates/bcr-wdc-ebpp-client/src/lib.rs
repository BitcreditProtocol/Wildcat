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
            .join("/v1/admin/onchain/balance")
            .expect("balance relative path");
        let res = self.cl.get(url).send().await?;
        let blnc = res.json::<web_wallet::Balance>().await?;
        Ok(bdk_wallet::Balance::from(blnc))
    }
}
