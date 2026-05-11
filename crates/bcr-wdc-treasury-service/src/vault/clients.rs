// ----- standard library imports
use std::sync::Arc;
// ----- extra library imports
use async_trait::async_trait;
use bcr_common::{cashu, client::core::Client as CoreClient};
// ----- local imports
use crate::{error::Result, vault};

// ----- end imports

pub struct WildcatCl {
    pub core: Arc<CoreClient>,
}

#[async_trait]
impl vault::WildcatClient for WildcatCl {
    async fn check_spent(&self, ys: Vec<cashu::PublicKey>) -> Result<Vec<cashu::ProofState>> {
        let states = self.core.check_state(ys).await?;
        Ok(states)
    }
    fn unit(&self) -> cashu::CurrencyUnit {
        CoreClient::currency_unit()
    }
}
