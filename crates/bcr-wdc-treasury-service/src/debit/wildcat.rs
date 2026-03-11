// ----- standard library imports
// ----- extra library imports
use async_trait::async_trait;
use bcr_common::{cashu, client::core::Client as CoreClient, wire::keys as wire_keys};
use futures::future::JoinAll;
// ----- local imports
use crate::{
    debit::WildcatClient,
    error::{Error, Result},
};

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
impl WildcatClient for WildcatCl {
    async fn sign(&self, blinds: Vec<cashu::BlindedMessage>) -> Result<Vec<cashu::BlindSignature>> {
        let joined: JoinAll<_> = blinds
            .iter()
            .map(|blind| self.core_cl.sign(blind))
            .collect();
        let signatures: Vec<cashu::BlindSignature> = joined
            .await
            .into_iter()
            .collect::<std::result::Result<_, _>>()?;
        Ok(signatures)
    }

    async fn burn(&self, inputs: &[cashu::Proof]) -> Result<()> {
        self.core_cl.burn(inputs.to_vec()).await?;
        Ok(())
    }

    async fn keyset_info(&self, kid: cashu::Id) -> Result<cashu::KeySetInfo> {
        let info = self.core_cl.keyset_info(kid).await?;
        Ok(info)
    }

    async fn get_active_keyset(&self) -> Result<cashu::Id> {
        let filter = wire_keys::KeysetInfoFilters {
            unit: Some(cashu::CurrencyUnit::Sat),
            ..Default::default()
        };
        let mut infos = self.core_cl.list_keyset_info(filter).await?;
        infos.retain(|info| info.active);
        if infos.is_empty() {
            return Err(Error::Internal(String::from("no active keyset found")));
        }
        infos.sort_by_key(|info| info.final_expiry);
        let last_kid = infos.last().unwrap().id;
        let kid = infos
            .into_iter()
            .find(|info| info.final_expiry.is_none())
            .map(|info| info.id)
            .unwrap_or_else(|| last_kid);
        Ok(kid)
    }
}
