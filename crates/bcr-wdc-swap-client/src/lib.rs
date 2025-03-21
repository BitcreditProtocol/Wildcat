// ----- standard library imports
// ----- extra library imports
use bcr_wdc_webapi::swap as web_swap;
use cashu::nut00 as cdk00;
use cashu::nut03 as cdk03;
use thiserror::Error;
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
pub struct SwapClient {
    cl: reqwest::Client,
    base: reqwest::Url,
}

impl SwapClient {
    pub fn new(base: reqwest::Url) -> Result<Self> {
        Ok(Self {
            cl: reqwest::Client::new(),
            base,
        })
    }

    pub async fn swap(
        &self,
        inputs: Vec<cdk00::Proof>,
        outputs: Vec<cdk00::BlindedMessage>,
    ) -> Result<Vec<cdk00::BlindSignature>> {
        let url = self.base.join("/v1/swap").expect("swap relative path");
        let request = cdk03::SwapRequest { inputs, outputs };
        let res = self.cl.post(url).json(&request).send().await?;
        let signatures: cdk03::SwapResponse = res.json().await?;
        Ok(signatures.signatures)
    }

    pub async fn burn(&self, proofs: Vec<cdk00::Proof>) -> Result<web_swap::BurnResponse> {
        let url = self.base.join("/v1/burn").expect("burn relative path");
        let request = web_swap::BurnRequest { proofs };
        let res = self.cl.post(url).json(&request).send().await?;
        let burn_resp: web_swap::BurnResponse = res.json().await?;
        Ok(burn_resp)
    }

    pub async fn recover(&self, proofs: Vec<cdk00::Proof>) -> Result<web_swap::RecoverResponse> {
        let url = self.base.join("/v1/recover").expect("recover relative path");
        let request = web_swap::RecoverRequest { proofs };
        let res = self.cl.post(url).json(&request).send().await?;
        let recover_resp: web_swap::RecoverResponse = res.json().await?;
        Ok(recover_resp)
    }
}
