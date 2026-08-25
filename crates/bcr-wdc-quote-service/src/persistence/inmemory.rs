// ----- standard library imports
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
// ----- extra library imports
use anyhow::anyhow;
use async_trait::async_trait;
use bcr_common::core::{BillId, NodeId};
use strum::IntoDiscriminant;
use uuid::Uuid;
// ----- local modules
// ----- local imports
use crate::{
    error::{Error, Result},
    persistence::{ExposureReservationInput, Repository},
    quotes,
    service::{ListFilters, SortOrder},
};

#[allow(dead_code)]
#[derive(Default, Clone, Debug)]
pub struct QuotesIDMap {
    quotes: Arc<RwLock<HashMap<Uuid, quotes::Quote>>>,
    reservations: Arc<RwLock<HashMap<Uuid, bcr_common::wire::quotes::CreditExposureReservation>>>,
}
#[async_trait]
impl Repository for QuotesIDMap {
    async fn search_by_bill(&self, bill: &BillId, endorser: &NodeId) -> Result<Vec<quotes::Quote>> {
        Ok(self
            .quotes
            .read()
            .unwrap()
            .iter()
            .filter(|(_, quote)| {
                let holder = &quote
                    .bill
                    .endorsees
                    .last()
                    .unwrap_or(&quote.bill.payee)
                    .node_id();
                quote.bill.id == *bill && holder == endorser
            })
            .map(|x| x.1.clone())
            .collect())
    }

    async fn store(&self, quote: quotes::Quote) -> Result<()> {
        if quote.credit_program().is_none() {
            return Err(Error::CreditProgramNotBound(quote.id));
        }
        self.quotes.write().unwrap().insert(quote.id, quote);
        Ok(())
    }
    async fn load(&self, id: uuid::Uuid) -> Result<Option<quotes::Quote>> {
        Ok(self.quotes.read().unwrap().get(&id).cloned())
    }

    async fn update_status_if_pending(&self, qid: uuid::Uuid, new: quotes::Status) -> Result<()> {
        let mut m = self.quotes.write().unwrap();
        let result = m.get_mut(&qid);
        if let Some(old) = result {
            if matches!(old.status, quotes::Status::Pending { .. }) {
                old.status = new;
                return Ok(());
            }
        }
        Err(Error::QuotesRepository(anyhow!(
            "quote {qid} not found or not pending"
        )))
    }

    async fn execute_authorization(
        &self,
        quote: quotes::Quote,
        exposure: ExposureReservationInput,
    ) -> Result<bcr_common::wire::quotes::CreditAuthorizationReceipt> {
        let receipt = quote
            .authorization_receipt
            .clone()
            .ok_or(Error::CreditAuthorizationInvalid)?;
        let mut quotes = self.quotes.write().unwrap();
        let mut reservations = self.reservations.write().unwrap();
        let stored = quotes
            .get_mut(&quote.id)
            .ok_or_else(|| Error::ResourceNotFound(quote.id.to_string()))?;
        if let Some(existing) = &stored.authorization_receipt {
            return if existing.operation_id == receipt.operation_id
                && existing.authorization_digest == receipt.authorization_digest
            {
                Ok(existing.clone())
            } else {
                Err(Error::CreditAuthorizationConflict)
            };
        }
        if !matches!(stored.status, quotes::Status::Pending { .. }) {
            return Err(Error::CreditAuthorizationConflict);
        }
        let active = reservations
            .values()
            .filter(|reservation| {
                reservation.mint_id == exposure.mint_id
                    && matches!(reservation.state.as_str(), "reserved" | "committed")
            })
            .try_fold(0_u64, |total, reservation| {
                reservation
                    .amount_sat
                    .parse::<u64>()
                    .ok()
                    .and_then(|amount| total.checked_add(amount))
            })
            .ok_or(Error::CreditCapacityUnavailable)?;
        if exposure
            .existing_exposure_sat
            .checked_add(active)
            .and_then(|total| total.checked_add(exposure.amount_sat))
            .is_none_or(|total| total > exposure.exposure_limit_sat)
        {
            return Err(Error::CreditCapacityExceeded);
        }
        stored.status = quote.status;
        stored.authorization_receipt = Some(receipt.clone());
        reservations.insert(
            quote.id,
            bcr_common::wire::quotes::CreditExposureReservation {
                reservation_version: String::from("credit-exposure-reservation-v1"),
                reservation_id: uuid::Uuid::new_v4(),
                mint_id: exposure.mint_id,
                quote_id: quote.id,
                amount_sat: exposure.amount_sat.to_string(),
                capacity_evidence_id: exposure.capacity_evidence_id,
                state: String::from("reserved"),
                created_at: exposure.now,
                updated_at: exposure.now,
            },
        );
        Ok(receipt)
    }

    async fn update_status_if_offered(
        &self,
        qid: uuid::Uuid,
        new: quotes::Status,
        now: chrono::DateTime<chrono::Utc>,
    ) -> Result<()> {
        let mut m = self.quotes.write().unwrap();
        let mut reservations = self.reservations.write().unwrap();
        let result = m.get_mut(&qid);
        if let Some(old) = result {
            if matches!(old.status, quotes::Status::Offered { .. }) {
                let target = match &new {
                    quotes::Status::Accepted { .. } => "committed",
                    quotes::Status::Rejected { .. } | quotes::Status::OfferExpired { .. } => {
                        "released"
                    }
                    _ => {
                        return Err(Error::QuotesRepository(anyhow!(
                            "offered quote transition has no exposure lifecycle"
                        )))
                    }
                };
                let reservation = reservations.get_mut(&qid).ok_or_else(|| {
                    Error::QuotesRepository(anyhow!("quote {qid} has no exposure reservation"))
                })?;
                if reservation.state != "reserved" {
                    return Err(Error::QuotesRepository(anyhow!(
                        "quote {qid} exposure reservation is not reserved"
                    )));
                }
                reservation.state = String::from(target);
                reservation.updated_at = now;
                old.status = new;
                return Ok(());
            }
        }
        Err(Error::QuotesRepository(anyhow!(
            "quote {qid} not found or not offered"
        )))
    }

    async fn release_committed_exposure(
        &self,
        qid: uuid::Uuid,
        now: chrono::DateTime<chrono::Utc>,
    ) -> Result<()> {
        let mut reservations = self.reservations.write().unwrap();
        let reservation = reservations.get_mut(&qid).ok_or_else(|| {
            Error::QuotesRepository(anyhow!("quote {qid} has no exposure reservation"))
        })?;
        match reservation.state.as_str() {
            "released" => Ok(()),
            "committed" => {
                reservation.state = String::from("released");
                reservation.updated_at = now;
                Ok(())
            }
            _ => Err(Error::QuotesRepository(anyhow!(
                "quote {qid} exposure reservation is not committed"
            ))),
        }
    }

    async fn update_status_if_accepted(&self, qid: uuid::Uuid, new: quotes::Status) -> Result<()> {
        let mut m = self.quotes.write().unwrap();
        let result = m.get_mut(&qid);
        if let Some(old) = result {
            if matches!(old.status, quotes::Status::Accepted { .. }) {
                old.status = new;
                return Ok(());
            }
        }
        Err(Error::QuotesRepository(anyhow!(
            "quote {qid} not found or not accepted"
        )))
    }

    async fn update_status_if_failedebillvalidation(
        &self,
        qid: uuid::Uuid,
        new: quotes::Status,
    ) -> Result<()> {
        let mut m = self.quotes.write().unwrap();
        let result = m.get_mut(&qid);
        if let Some(old) = result {
            if matches!(old.status, quotes::Status::FailedEbillValidation { .. }) {
                old.status = new;
                return Ok(());
            }
        }
        Err(Error::QuotesRepository(anyhow!(
            "quote {qid} not found or not failedebillvalidation"
        )))
    }

    async fn list_light(
        &self,
        filters: ListFilters,
        sort: Option<SortOrder>,
    ) -> Result<Vec<quotes::LightQuote>> {
        let mut a: Vec<quotes::Quote> = self
            .quotes
            .read()
            .unwrap()
            .iter()
            .filter(|(_, quote)| {
                let ListFilters {
                    bill_maturity_date_from,
                    bill_maturity_date_to,
                    status,
                    bill_id,
                    bill_drawee_id,
                    bill_drawer_id,
                    bill_payer_id,
                    bill_holder_id,
                } = &filters;
                if let Some(bill_maturity_date_from) = bill_maturity_date_from {
                    if quote.bill.maturity_date < *bill_maturity_date_from {
                        return false;
                    }
                }
                if let Some(bill_maturity_date_to) = bill_maturity_date_to {
                    if quote.bill.maturity_date > *bill_maturity_date_to {
                        return false;
                    }
                }
                if let Some(status) = status {
                    if quote.status.discriminant() != *status {
                        return false;
                    }
                }
                if let Some(bill_id) = bill_id {
                    if quote.bill.id != *bill_id {
                        return false;
                    }
                }
                if let Some(bill_drawee_id) = bill_drawee_id {
                    if quote.bill.drawee.node_id != *bill_drawee_id {
                        return false;
                    }
                }
                if let Some(bill_drawer_id) = bill_drawer_id {
                    if quote.bill.drawer.node_id != *bill_drawer_id {
                        return false;
                    }
                }
                if let Some(bill_payer_id) = bill_payer_id {
                    if quote.bill.payee.node_id() != *bill_payer_id {
                        return false;
                    }
                }
                if let Some(bill_holder_id) = bill_holder_id {
                    let holder_id = &quote
                        .bill
                        .endorsees
                        .last()
                        .unwrap_or(&quote.bill.payee)
                        .node_id();
                    if *holder_id != *bill_holder_id {
                        return false;
                    }
                }
                true
            })
            .map(|(_, quote)| quote.clone())
            .collect();
        if let Some(sort) = sort {
            a.sort_by(|q1, q2| match sort {
                SortOrder::BillMaturityDateAsc => q1.bill.maturity_date.cmp(&q2.bill.maturity_date),
                SortOrder::BillMaturityDateDesc => {
                    q2.bill.maturity_date.cmp(&q1.bill.maturity_date)
                }
                SortOrder::SubmittedAsc => q1.submitted.cmp(&q2.submitted),
                SortOrder::SubmittedDesc => q2.submitted.cmp(&q1.submitted),
            });
        }
        let b = a
            .into_iter()
            .map(|quote| quotes::LightQuote {
                id: quote.id,
                status: quote.status.discriminant(),
                sum: quote.bill.sum,
                maturity_date: quote.bill.maturity_date,
            })
            .collect();
        Ok(b)
    }
}
