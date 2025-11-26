// ----- standard library imports
// ----- extra library imports
use async_trait::async_trait;
use bcr_common::client::{keys::Client as KeysClient, swap::Client as SwapClient};
use cashu::{nut00 as cdk00, nut02 as cdk02};
// ----- local imports
use crate::{debit::service::WildcatService, error::Result};

// ----- end imports

#[derive(Clone, Debug, serde::Deserialize)]
pub struct WildcatClientConfig {
    pub swap_service_url: reqwest::Url,
    pub key_service_url: reqwest::Url,
}

#[derive(Clone, Debug)]
pub struct WildcatCl {
    swap_cl: SwapClient,
    key_cl: KeysClient,
}

impl WildcatCl {
    pub fn new(cfg: WildcatClientConfig) -> Self {
        let swap_cl = SwapClient::new(cfg.swap_service_url);
        let key_cl = KeysClient::new(cfg.key_service_url);
        Self { swap_cl, key_cl }
    }
}

#[async_trait]
impl WildcatService for WildcatCl {
    async fn burn(&self, inputs: &[cdk00::Proof]) -> Result<()> {
        self.swap_cl.burn(inputs.to_vec()).await?;
        Ok(())
    }

    async fn keyset_info(&self, kid: cdk02::Id) -> Result<cdk02::KeySetInfo> {
        let info = self.key_cl.keyset_info(kid).await?;
        Ok(info)
    }
}
