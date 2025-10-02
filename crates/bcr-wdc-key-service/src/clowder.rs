// ----- standard library imports
// ----- extra library imports
use async_trait::async_trait;
use clwdr_client::ClowderNatsClient;
// ----- local imports
use crate::{
    error::{Error, Result},
    service::ClowderClient,
};

// ----- end imports

#[derive(Debug, Clone, Default, serde::Deserialize)]
pub enum ClowderClientConfig {
    #[default]
    Dummy,
    ClowderNats {
        url: reqwest::Url,
        wait_ack: bool,
    },
}

pub async fn build_clowder_client(cfg: ClowderClientConfig) -> Result<Box<dyn ClowderClient>> {
    match cfg {
        ClowderClientConfig::Dummy => Ok(Box::new(DummyClowderClient)),
        ClowderClientConfig::ClowderNats { url, wait_ack } => {
            let client = ClowderNatsClient::new(url, wait_ack)
                .await
                .map_err(|e| Error::ClowderClient(anyhow::anyhow!(e.to_string())))?;
            Ok(Box::new(ClowderCl(client)))
        }
    }
}

pub struct DummyClowderClient;

#[async_trait]
impl ClowderClient for DummyClowderClient {
    async fn new_keyset(&self, keyset: cashu::KeySet) -> Result<()> {
        tracing::debug!("DummyClowderClient::new_keyset for kid {}", keyset.id);

        Ok(())
    }
    async fn keyset_deactivated(&self, kid: cashu::Id) -> Result<()> {
        tracing::debug!("DummyClowderClient::keyset_deactivated for kid {}", kid);

        Ok(())
    }
}

pub struct ClowderCl(ClowderNatsClient);

#[async_trait]
impl ClowderClient for ClowderCl {
    async fn new_keyset(&self, keyset: cashu::KeySet) -> Result<()> {
        self.0
            .post_keyset(keyset)
            .await
            .map_err(|e| Error::ClowderClient(anyhow::anyhow!(e.to_string())))?;
        Ok(())
    }
    async fn keyset_deactivated(&self, kid: cashu::Id) -> Result<()> {
        self.0
            .deactivate_keyset(kid)
            .await
            .map_err(|e| Error::ClowderClient(anyhow::anyhow!(e.to_string())))?;
        Ok(())
    }
}
