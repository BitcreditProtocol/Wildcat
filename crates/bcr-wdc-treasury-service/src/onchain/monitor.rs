// ----- standard library imports
use std::sync::Arc;
// ----- extra library imports
use anyhow::Result as AnyResult;
use async_trait::async_trait;
use bcr_wdc_utils::routine::{Routine, TStamp};
// ----- local imports
use crate::{error::Result, onchain::MintStatus};

// ----- end imports

pub struct MintOpMonitor {
    pub srvc: Arc<crate::onchain::Service>,
}

impl MintOpMonitor {
    async fn check_pendings(&self, now: TStamp) -> Result<()> {
        let pendings = self.srvc.repo.list_onchain_pending_mintops().await?;
        for pending in pendings {
            let op = self.srvc.repo.load_onchain_mintop(pending).await?;
            let MintStatus::Pending { blinds } = op.status else {
                tracing::warn!("mintop {pending} is not pending, skipping");
                continue;
            };
            let received = self
                .srvc
                .clowder_cl
                .verify_onchain_mint_payment(op.qid, op.kid)
                .await?;
            if received >= op.target {
                tracing::info!("mintop {pending} {received} >= {}", op.target);
                let signatures = self.srvc.wdc.sign(blinds).await?;
                let new = MintStatus::Paid {
                    signatures: signatures.clone(),
                };
                self.srvc
                    .clowder_cl
                    .mint_onchain(op.qid, op.kid, signatures)
                    .await?;
                self.srvc
                    .repo
                    .update_onchain_mintop_status(op.qid, new)
                    .await?;
            } else if op.expiry < now {
                // TODO: we should accept transactions included in the first block after expiry
                tracing::info!("mintop {pending} expired");
                let new = MintStatus::Expired;
                self.srvc
                    .repo
                    .update_onchain_mintop_status(op.qid, new)
                    .await?;
            }
        }
        Ok(())
    }
}

#[async_trait]
impl Routine for MintOpMonitor {
    async fn run_task(&self, now: TStamp) -> AnyResult<Option<std::time::Duration>> {
        self.check_pendings(now).await?;
        Ok(None)
    }
}
