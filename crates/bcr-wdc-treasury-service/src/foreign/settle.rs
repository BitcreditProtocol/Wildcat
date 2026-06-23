// ----- standard library imports
use std::sync::Arc;
// ----- extra library imports
use anyhow::Result as AnyResult;
use async_trait::async_trait;
use bcr_common::cashu::{self, ProofsMethods};
use bcr_wdc_utils::{routine::Routine, TStamp};
use tracing::{debug, error, info, warn};
// ----- local imports
use crate::foreign::{
    signed_swap_with_foreign, ClowderClient, MintClientFactory, OfflineRepository, OnlineRepository,
};

// ----- end imports

pub struct Handler {
    pub online: Arc<dyn OnlineRepository>,
    pub offline: Arc<dyn OfflineRepository>,
    pub clowder: Arc<dyn ClowderClient>,
    pub mint_factory: Arc<dyn MintClientFactory>,
}

macro_rules! async_try_or_warn {
    ($expr:expr) => {
        match $expr.await {
            Ok(result) => result,
            Err(e) => {
                warn!("{} failed: {}, retry later", stringify!($expr), e);
                continue;
            }
        }
    };
}

#[async_trait]
impl Routine for Handler {
    async fn run_task(&self, now: TStamp) -> AnyResult<Option<std::time::Duration>> {
        let mints = self.offline.list_foreign_pks().await?;
        for mint_id in mints {
            debug!("Checking offline status for mint {mint_id}");
            let mint_url = async_try_or_warn!(self.clowder.get_mint_url_from_pk(&mint_id));
            let is_offline = async_try_or_warn!(self.clowder.is_offline(mint_id));
            if is_offline {
                debug!("{} still offline", mint_url);
                continue;
            }
            // foreign mint is back online, proceed to settle
            // load the proofs from the repository
            let foreign_proofs = async_try_or_warn!(self.offline.load_proofs(mint_id));
            let foreign_ys: Vec<cashu::PublicKey> = foreign_proofs
                .iter()
                .map(|p| p.y().expect("Proof::y(): impossible!!"))
                .collect();
            let foreign_client =
                async_try_or_warn!(self.mint_factory.make_client(mint_url.clone(), mint_id));
            let new_proofs = async_try_or_warn!(signed_swap_with_foreign(
                foreign_proofs,
                self.clowder.as_ref(),
                foreign_client.as_ref(),
                now
            ));
            loop {
                match self.online.store(mint_id, new_proofs.clone()).await {
                    Ok(_) => break,
                    Err(e) => {
                        error!("Failed to store proofs for mint {mint_url}: {e}, retrying...");
                        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                    }
                }
            }
            if self.offline.remove_proofs(&foreign_ys).await.is_err() {
                warn!("remove_proofs failed {mint_url}");
            }
            let total = new_proofs.total_amount().unwrap_or_default();
            info!("Settled {total} from mint {mint_url}");
        }
        Ok(None)
    }
}
