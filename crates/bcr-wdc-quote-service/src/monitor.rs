// ----- standard library imports
use std::sync::Arc;
// ----- extra library imports
use anyhow::Result as AnyResult;
use async_trait::async_trait;
use bcr_wdc_utils::routine::Routine;
// ----- local imports
use crate::{
    error::{Error, Result},
    quotes::{Status, StatusDiscriminants},
    service::{ListFilters, Service},
};

// ----- end imports

#[derive(Clone)]
pub struct EbillMonitor {
    pub srvc: Arc<Service>,
}

impl EbillMonitor {
    async fn check_quotes(&self) {
        tracing::info!("Checking accepted quotes for endorsed ebills");
        let qids = match self.list_accepted_quotes().await {
            Ok(qids) => qids,
            Err(e) => {
                tracing::error!("Failed to list accepted quotes: {e}");
                return;
            }
        };
        for qid in qids {
            match self.check_quote_ebill(qid).await {
                Ok(()) => tracing::info!("Checked quote {qid} successfully"),
                Err(e) => tracing::error!("Failed to check quote {qid}: {e}"),
            }
        }
    }

    async fn list_accepted_quotes(&self) -> Result<Vec<uuid::Uuid>> {
        let filters = ListFilters {
            status: Some(StatusDiscriminants::Accepted),
            ..Default::default()
        };
        let now = chrono::Utc::now();
        let list = self.srvc.list_light(filters, None, now).await?;
        list.into_iter().map(|q| Ok(q.id)).collect()
    }

    async fn check_quote_ebill(&self, qid: uuid::Uuid) -> Result<()> {
        let Some(quote) = self.srvc.quotes.load(qid).await? else {
            return Err(Error::QuoteIDNotFound(qid));
        };
        if !matches!(quote.status, Status::Accepted { .. }) {
            return Err(Error::InvalidQuoteStatus(
                qid,
                StatusDiscriminants::Accepted,
                StatusDiscriminants::from(&quote.status),
            ));
        }
        let bid = quote.bill.id;
        let Ok(_billinfo) = self.srvc.wdc_client.get_ebill(bid.clone()).await else {
            tracing::info!("ebill {bid} from quote {qid} not found yet, skipping");
            return Ok(());
        };
        self.srvc.enable_minting(qid).await
    }
}

#[async_trait]
impl Routine for EbillMonitor {
    async fn run_task(&self) -> AnyResult<()> {
        self.check_quotes().await;
        Ok(())
    }
}
