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
    use bcr_common::{cashu, core_tests, wire::bill as wire_bill, wire_tests};
    use mockall::predicate::{always, eq};
    use std::str::FromStr;
    use uuid::Uuid;
    pub const TEST_URL: &str = "http://localhost:8000";

    #[tokio::test]
    async fn ebillmonitor_check_quote_ebill_not_yet_received() {
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

    #[tokio::test]
    async fn ebillmonitor_check_quote_enableminting() {
        let mut repo = MockRepository::new();
        let mut wdc = MockWdcClient::new();
        let qid = Uuid::new_v4();
        let keyset = core_tests::generate_random_ecash_keyset().1;
        let pk = cashu::PublicKey::from(core_tests::generate_random_keypair().public_key());
        let quote = quotes::Quote {
            id: qid,
            status: quotes::Status::Accepted {
                discounted: bitcoin::Amount::from_sat(1000),
                wallet_pubkey: pk.clone(),
                keyset_id: keyset.id,
            },
            submitted: chrono::DateTime::default(),
            bill: quotes::BillInfo {
                sum: bitcoin::Amount::from_sat(2000),
                ..quotes::BillInfo::random()
            },
        };
        let bid = quote.bill.id.clone();
        repo.expect_load()
            .times(2)
            .with(eq(qid))
            .returning(move |_| Ok(Some(quote.clone())));
        let bill = wire_bill::BitcreditBill {
            id: bid.clone(),
            participants: wire_bill::BillParticipants {
                drawee: wire_tests::random_identity_public_data().1,
                drawer: wire_tests::random_identity_public_data().1,
                payee: wire_bill::BillParticipant::Ident(
                    wire_tests::random_identity_public_data().1,
                ),
                endorsee: None,
                endorsements: vec![],
                endorsements_count: 0,
                all_participant_node_ids: vec![],
            },
            data: wire_bill::BillData::default(),

            status: wire_bill::BillStatus::default(),
            current_waiting_state: None,
        };
        wdc.expect_get_ebill()
            .times(1)
            .with(eq(bid.clone()))
            .returning(move |_| Ok(bill.clone()));
        let cloned = cashu::KeySet::from(keyset.clone());
        wdc.expect_get_keys()
            .times(1)
            .with(eq(keyset.id))
            .returning(move |_| Ok(cloned.clone()));
        let cloned = keyset.clone();
        wdc.expect_sign().times(1).returning(move |blnds| {
            let amounts = blnds.iter().map(|b| b.amount).collect::<Vec<_>>();
            let signatures = core_tests::generate_ecash_signatures(&cloned, &amounts);
            Ok(signatures)
        });
        repo.expect_update_status_if_accepted()
            .times(1)
            .with(eq(qid), always())
            .returning(|_, _| Ok(()));
        wdc.expect_add_new_mint_operation()
            .times(1)
            .with(
                eq(qid),
                eq(keyset.id),
                eq(pk),
                eq(cashu::Amount::from(1000)),
                eq(bid),
            )
            .returning(|_, _, _, _, _| Ok(()));
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
