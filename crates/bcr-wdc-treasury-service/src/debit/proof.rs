// ----- standard library imports
// ----- extra library imports
use async_trait::async_trait;
use bcr_wdc_swap_client::{SwapClient, Url};
use cashu::nuts::nut00 as cdk00;
// ----- local imports
use crate::{
    debit::service::ProofClient,
    error::{Error, Result},
};

// ----- end imports

#[derive(Clone, Debug)]
pub struct ProofCl {
    cl: SwapClient,
}

impl ProofCl {
    pub fn new(base_url: Url) -> Self {
        let cl = SwapClient::new(base_url);
        Self { cl }
    }
}

#[async_trait]
impl ProofClient for ProofCl {
    async fn burn(&self, inputs: &[cdk00::Proof]) -> Result<()> {
        self.cl
            .burn(inputs.to_vec())
            .await
            .map_err(Error::ProofCl)?;
        Ok(())
    }
}
