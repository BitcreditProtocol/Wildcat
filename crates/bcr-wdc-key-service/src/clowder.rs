// ----- standard library imports
// ----- extra library imports
use async_trait::async_trait;
use bcr_common::wire::clowder::events;
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
    },
}

pub async fn build_clowder_client(cfg: ClowderClientConfig) -> Result<Box<dyn ClowderClient>> {
    match cfg {
        ClowderClientConfig::Dummy => Ok(Box::new(DummyClowderClient)),
        ClowderClientConfig::ClowderNats { url } => {
            let client = ClowderNatsClient::new(url)
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
        let req = events::KeysetCreationRequest {
            id: keyset.id,
            expiry: keyset.final_expiry.unwrap_or(0_u64),
            unit: keyset.unit.clone(),
        };
        let resp = events::KeysetCreationResponse {
            public_keys: keyset.keys.keys().clone(),
            id: keyset.id,
            expiry: keyset.final_expiry.unwrap_or(0_u64),
            unit: keyset.unit,
        };
        self.0
            .post_keyset(req, resp)
            .await
            .map_err(|e| Error::ClowderClient(anyhow::anyhow!(e.to_string())))?;
        Ok(())
    }

    async fn keyset_deactivated(&self, kid: cashu::Id) -> Result<()> {
        self.0
            .deactivate_keyset(events::KeysetDeactivationRequest { id: kid })
            .await
            .map_err(|e| Error::ClowderClient(anyhow::anyhow!(e.to_string())))?;
        Ok(())
    }
}
