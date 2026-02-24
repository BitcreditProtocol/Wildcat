// ----- standard library imports
// ----- extra library imports
use async_trait::async_trait;
use bcr_common::{cashu, client::core::Client as CoreClient};
// ----- local imports
use crate::{debit::service::WildcatService, error::Result};

// ----- end imports

#[derive(Clone, Debug, serde::Deserialize)]
pub struct WildcatClientConfig {
    pub core_service_url: reqwest::Url,
}

#[derive(Clone, Debug)]
pub struct WildcatCl {
    core_cl: CoreClient,
}

impl WildcatCl {
    pub fn new(cfg: WildcatClientConfig) -> Self {
        let core_cl = CoreClient::new(cfg.core_service_url);
        Self { core_cl }
    }
}

#[async_trait]
impl WildcatService for WildcatCl {
    async fn burn(&self, inputs: &[cashu::Proof]) -> Result<()> {
        self.core_cl.burn(inputs.to_vec()).await?;
        Ok(())
    }

    async fn keyset_info(&self, kid: cashu::Id) -> Result<cashu::KeySetInfo> {
        let info = self.core_cl.keyset_info(kid).await?;
        Ok(info)
    }
}
