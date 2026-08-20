// ----- standard library imports
use std::sync::Arc;
// ----- extra library imports
use anyhow::Result as AnyResult;
use async_trait::async_trait;
use bcr_common::cashu::{self, ProofsMethods};
use bcr_wdc_utils::{routine::Routine, TStamp};
use tracing::{info, warn};
// ----- local imports
use crate::foreign::{ClowderClient, KeysClient, OnlineRepository};
// ----- end imports

/// Takes back exchange eCash whose locktime has passed without the recipient ever
/// unlocking it. Only this mint's refund key can spend it now, and the collateral
/// it was issued against was never claimed, so retiring it is what keeps issuance
/// backed.
pub struct Handler {
    pub online: Arc<dyn OnlineRepository>,
    pub keys: Arc<dyn KeysClient>,
    pub clowder: Arc<dyn ClowderClient>,
}

#[async_trait]
impl Routine for Handler {
    async fn run_task(&self, now: TStamp) -> AnyResult<Option<std::time::Duration>> {
        let expired = self.online.list_expired_issued(now).await?;
        if expired.is_empty() {
            return Ok(None);
        }
        let ys: Vec<cashu::PublicKey> = expired
            .iter()
            .map(|proof| proof.y())
            .collect::<Result<_, _>>()?;
        let amount = expired.total_amount().unwrap_or_default();

        // The refund path is a signature over the secret from the key the lock
        // named, which is this mint's own node.
        let witnessed = match self.clowder.sign_p2pk_proofs(&expired).await {
            Ok(witnessed) => witnessed,
            Err(e) => {
                warn!("Cannot witness expired exchange eCash: {e}, retry later");
                return Ok(None);
            }
        };
        match self.keys.burn(witnessed.clone()).await {
            Ok(()) => {
                // The local burn stops it circulating; the chain entry is what makes
                // this mint's supply match what it can honour.
                if let Err(e) = self.clowder.signal_burn_event(witnessed).await {
                    warn!("Burned {amount} locally but could not record it on chain: {e}");
                }
                self.online.remove_issued(&ys).await?;
                info!("Burned {amount} of expired exchange eCash");
            }
            Err(e) => warn!("Burn of {amount} deferred: {e}"),
        }
        Ok(None)
    }
}
