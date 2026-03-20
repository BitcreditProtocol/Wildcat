// ----- standard library imports
use std::sync::Arc;
// ----- extra library imports
use anyhow::Result as AnyResult;
use async_trait::async_trait;
use bcr_wdc_utils::routine::{Routine, TStamp};
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
    async fn check_quotes(&self, now: TStamp) {
        tracing::info!("Checking accepted quotes for endorsed ebills");
        let qids = match self.list_accepted_quotes(now).await {
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

    async fn list_accepted_quotes(&self, now: TStamp) -> Result<Vec<uuid::Uuid>> {
        let filters = ListFilters {
            status: Some(StatusDiscriminants::Accepted),
            ..Default::default()
        };
        let list = self.srvc.list_light(filters, None, now).await?;
        list.into_iter().map(|q| Ok(q.id)).collect()
    }

    async fn check_quote_ebill(&self, qid: uuid::Uuid) -> Result<()> {
        let Some(quote) = self.srvc.quotes.load(qid).await? else {
            return Err(Error::ResourceNotFound(qid.to_string()));
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
    async fn run_task(&self, now: TStamp) -> AnyResult<Option<std::time::Duration>> {
        self.check_quotes(now).await;
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{persistence::MockRepository, quotes, service::MockWdcClient};
    use bcr_common::{cashu, core_tests};
    use mockall::predicate::eq;
    use std::str::FromStr;
    use uuid::Uuid;
    pub const TEST_URL: &str = "http://localhost:8000";

    #[tokio::test]
    async fn ebillmonitor_check_quotes_not_yet_received() {
        let mut repo = MockRepository::new();
        let mut wdc = MockWdcClient::new();
        let qid = Uuid::new_v4();
        let quote = quotes::Quote {
            id: qid,
            status: quotes::Status::Accepted {
                discounted: bitcoin::Amount::from_sat(1000),
                wallet_pubkey: cashu::PublicKey::from(
                    core_tests::generate_random_keypair().public_key(),
                ),
                keyset_id: core_tests::generate_random_ecash_keyset().0.id,
            },
            submitted: chrono::DateTime::default(),
            bill: quotes::BillInfo::random(),
        };
        let bid = quote.bill.id.clone();
        repo.expect_load()
            .times(1)
            .with(eq(qid))
            .returning(move |_| Ok(Some(quote.clone())));
        wdc.expect_get_ebill()
            .times(1)
            .with(eq(bid))
            .returning(|_| Err(Error::ResourceNotFound(String::new())));
        let serv = Service {
            quotes: Box::new(repo),
            wdc_client: Box::new(wdc),
            mint_url: cashu::MintUrl::from_str(TEST_URL).unwrap(),
        };
        let monitor = EbillMonitor {
            srvc: Arc::new(serv),
        };
        monitor.check_quote_ebill(qid).await.unwrap();
    }
}
