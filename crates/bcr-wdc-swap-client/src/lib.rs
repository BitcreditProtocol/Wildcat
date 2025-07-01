// ----- standard library imports
// ----- extra library imports
use bcr_wdc_webapi::swap as web_swap;
use cashu::{nut00 as cdk00, nut03 as cdk03, nut07 as cdk07};
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
    #[cfg(feature = "authorized")]
    auth: bcr_wdc_utils::client::AuthorizationPlugin,
}

impl SwapClient {
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

    pub async fn swap(
        &self,
        inputs: Vec<cdk00::Proof>,
        outputs: Vec<cdk00::BlindedMessage>,
    ) -> Result<Vec<cdk00::BlindSignature>> {
        let url = self.base.join("/v1/swap").expect("swap relative path");
        let request = cdk03::SwapRequest::new(inputs, outputs);
        let res = self.cl.post(url).json(&request).send().await?;
        let signatures: cdk03::SwapResponse = res.json().await?;
        Ok(signatures.signatures)
    }

    pub async fn burn(&self, proofs: Vec<cdk00::Proof>) -> Result<Vec<cashu::PublicKey>> {
        let url = self.base.join("/v1/burn").expect("burn relative path");
        let request = web_swap::BurnRequest { proofs };
        let res = self.cl.post(url).json(&request).send().await?;
        let burn_resp: web_swap::BurnResponse = res.json().await?;
        Ok(burn_resp.ys)
    }

    #[cfg(feature = "authorized")]
    pub async fn recover(&self, proofs: Vec<cdk00::Proof>) -> Result<web_swap::RecoverResponse> {
        let url = self
            .base
            .join("/v1/admin/swap/recover")
            .expect("recover relative path");
        let msg = web_swap::RecoverRequest { proofs };
        let request = self.cl.post(url).json(&msg);
        let response = self.auth.authorize(request).send().await?.json().await?;
        Ok(response)
    }

    pub async fn check_state(&self, ys: Vec<cashu::PublicKey>) -> Result<Vec<cdk07::ProofState>> {
        let url = self
            .base
            .join("/v1/checkstate")
            .expect("checkstate relative path");
        let request = cdk07::CheckStateRequest { ys };
        let res = self.cl.post(url).json(&request).send().await?;
        let state_resp: cdk07::CheckStateResponse = res.json().await?;
        Ok(state_resp.states)
    }
}
