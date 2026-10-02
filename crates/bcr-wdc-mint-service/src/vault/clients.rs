// ----- standard library imports
use std::sync::Arc;
// ----- extra library imports
use async_trait::async_trait;
use bcr_common::{cashu, client::core::Client as CoreClient};
// ----- local imports
use crate::{core, error::Result, vault};

// ----- end imports

/// Gives the vault the view of core it needs. In the merged binary core is in-process, so this
/// calls the service directly instead of going back out over HTTP to `/v1/checkstate`.
pub struct WildcatCl {
    pub core: Arc<core::service::Service>,
}

#[async_trait]
impl vault::WildcatClient for WildcatCl {
    async fn check_spent(&self, ys: Vec<cashu::PublicKey>) -> Result<Vec<cashu::ProofState>> {
        // same call the /v1/checkstate handler makes, so the expiry cleanups it performs still
        // happen on the same schedule
        let now = time::OffsetDateTime::now_utc();
        let states = self.core.check_state(&ys, now).await?;
        Ok(states)
    }
    fn unit(&self) -> cashu::CurrencyUnit {
        CoreClient::currency_unit()
    }
}
