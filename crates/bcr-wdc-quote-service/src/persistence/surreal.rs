// ----- standard library imports
// ----- extra library imports
use anyhow::anyhow;
use async_trait::async_trait;
use bcr_common::core::{BillId, NodeId};
use bcr_wdc_utils::surreal;
use std::sync::Arc;
use surrealdb::Result as SurrealResult;
use surrealdb::{engine::any::Any, Surreal};
use tokio::sync::Mutex;
use uuid::Uuid;
// ----- local imports
use crate::{
    error::{Error, Result},
    persistence::{ExposureReservationInput, Repository},
    quotes,
    service::{ListFilters, SortOrder},
    TStamp,
};

// ----- end imports

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct QuoteDBEntry {
    qid: surrealdb::Uuid, // can't be `id`, reserved word in surreal
    bill: quotes::BillInfo,
    submitted: TStamp,
    status: quotes::Status,
    #[serde(default)]
    credit_program: Option<quotes::CreditProgramBinding>,
    #[serde(default)]
    authorization_receipt: Option<bcr_common::wire::quotes::CreditAuthorizationReceipt>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ExposureReservationRecord {
    reservation_version: String,
    reservation_id: uuid::Uuid,
    mint_id: String,
    quote_id: uuid::Uuid,
    amount_sat: String,
    amount: u64,
    capacity_evidence_id: uuid::Uuid,
    state: String,
    created_at: crate::TStamp,
    updated_at: crate::TStamp,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ExposureEvent {
    event_version: String,
    event_id: uuid::Uuid,
    quote_id: uuid::Uuid,
    reservation_id: uuid::Uuid,
    mint_id: String,
    amount_sat: String,
    from_state: Option<String>,
    to_state: String,
    capacity_evidence_id: uuid::Uuid,
    recorded_at: crate::TStamp,
}

impl From<QuoteDBEntry> for quotes::Quote {
    fn from(dbq: QuoteDBEntry) -> Self {
        Self {
            id: dbq.qid,
            bill: dbq.bill,
            submitted: dbq.submitted,
            status: dbq.status,
            credit_program: dbq.credit_program,
            authorization_receipt: dbq.authorization_receipt,
        }
    }
}

impl From<quotes::Quote> for QuoteDBEntry {
    fn from(quote: quotes::Quote) -> Self {
        Self {
            qid: quote.id,
            bill: quote.bill,
            submitted: quote.submitted,
            status: quote.status,
            credit_program: quote.credit_program,
            authorization_receipt: quote.authorization_receipt,
        }
    }
}

#[derive(Debug, Clone, serde::Deserialize)]
struct LightQuoteDBEntry {
    qid: uuid::Uuid,
    status: quotes::StatusDiscriminants,
    sum: bitcoin::Amount,
    maturity_date: chrono::NaiveDate,
    #[allow(dead_code)]
    submitted: TStamp,
}
impl From<LightQuoteDBEntry> for quotes::LightQuote {
    fn from(dbq: LightQuoteDBEntry) -> Self {
        Self {
            id: dbq.qid,
            status: dbq.status,
            sum: dbq.sum,
            maturity_date: dbq.maturity_date,
        }
    }
}

#[derive(Debug, Clone)]
pub struct DBQuotes {
    db: Surreal<surrealdb::engine::any::Any>,
    // ponytail: one lock matches the single quote-service replica deployed today. Move the
    // compare-and-reserve into a serializable external ledger before horizontally scaling it.
    credit_exposure_lock: Arc<Mutex<()>>,
}

macro_rules! add_filter_statement {
    ($query:ident, $first:ident, $filter:expr, $statement:expr) => {
        if $filter.is_some() {
            if $first {
                $first = false;
                $query += " WHERE ";
            } else {
                $query += " AND ";
            }
            $query += $statement;
        }
    };
}

impl DBQuotes {
    const TABLE: &'static str = "quotes";

    pub async fn new(cfg: surreal::DBConnConfig) -> SurrealResult<Self> {
        let db_connection = Surreal::<Any>::init();
        db_connection.connect(cfg.connection).await?;
        db_connection.use_ns(cfg.namespace).await?;
        db_connection.use_db(cfg.database).await?;
        Ok(Self {
            db: db_connection,
            credit_exposure_lock: Arc::new(Mutex::new(())),
        })
    }

    async fn load(&self, qid: Uuid) -> SurrealResult<Option<QuoteDBEntry>> {
        let rid = surrealdb::RecordId::from_table_key(Self::TABLE, qid);
        self.db.select(rid).await
    }

    async fn store(&self, quote: QuoteDBEntry) -> SurrealResult<Option<QuoteDBEntry>> {
        let rid = surrealdb::RecordId::from_table_key(Self::TABLE, quote.qid);
        self.db.insert(rid).content(quote).await
    }

    async fn light_list(
        &self,
        filters: ListFilters,
        sort: Option<SortOrder>,
    ) -> SurrealResult<Vec<LightQuoteDBEntry>> {
        let mut statement = String::from(
            "SELECT qid, status.status as status, bill.sum AS sum, bill.maturity_date as maturity_date, submitted FROM type::table($table)",
        );

        let mut first = true;

        add_filter_statement!(
            statement,
            first,
            filters.bill_maturity_date_from,
            "bill.maturity_date >= $bill_maturity_date_from"
        );
        add_filter_statement!(
            statement,
            first,
            filters.bill_maturity_date_to,
            "bill.maturity_date <= $bill_maturity_date_to"
        );
        let status = filters.status;
        add_filter_statement!(statement, first, status, "status.status == $status");
        add_filter_statement!(statement, first, filters.bill_id, "bill.id == $bill_id");
        add_filter_statement!(
            statement,
            first,
            filters.bill_drawee_id,
            "bill.drawee.node_id == $bill_drawee_id"
        );
        add_filter_statement!(
            statement,
            first,
            filters.bill_drawer_id,
            "bill.drawer.node_id == $bill_drawer_id"
        );
        add_filter_statement!(
            statement,
            first,
            filters.bill_payer_id,
            "bill.payer.node_id == $bill_payer_id"
        );
        #[allow(unused_assignments)]
        {
            add_filter_statement!(
                statement,
                first,
                filters.bill_holder_id,
                "bill.holder.node_id == $bill_holder_id"
            );
        }
        if let Some(sort) = sort {
            statement += match sort {
                SortOrder::BillMaturityDateAsc => " ORDER BY maturity_date ASC",
                SortOrder::BillMaturityDateDesc => " ORDER BY maturity_date DESC",
                SortOrder::SubmittedAsc => " ORDER BY submitted ASC",
                SortOrder::SubmittedDesc => " ORDER BY submitted DESC",
            };
        }
        let query = self
            .db
            .query(statement)
            .bind(("table", Self::TABLE))
            .bind(filters);

        query.await?.take(0)
    }

    async fn search_by_bill(
        &self,
        bill: &BillId,
        endorser: &NodeId,
    ) -> SurrealResult<Vec<QuoteDBEntry>> {
        let results: Vec<QuoteDBEntry> = self
            .db
            .query("SELECT * FROM type::table($table) WHERE bill.id == $bill AND (bill.current_holder.Anon.node_id == $endorser OR bill.current_holder.Ident.node_id == $endorser) ORDER BY submitted DESC")
            .bind(("table", Self::TABLE))
            .bind(("bill", bill.to_owned()))
            .bind(("endorser", endorser.to_owned()))
            .await?
            .take(0)?;
        Ok(results)
    }
}

#[async_trait]
impl Repository for DBQuotes {
    async fn load(&self, qid: uuid::Uuid) -> Result<Option<quotes::Quote>> {
        let res = self
            .load(qid)
            .await
            .map_err(|e| Error::QuotesRepository(anyhow!(e)))?
            .map(quotes::Quote::from);
        Ok(res)
    }

    async fn update_status_if_pending(&self, qid: uuid::Uuid, new: quotes::Status) -> Result<()> {
        let recordid = surrealdb::RecordId::from_table_key(Self::TABLE, qid);
        let before: Option<QuoteDBEntry> = self
            .db
            .query("UPDATE $rid SET status = $new WHERE status.status == $status RETURN BEFORE ")
            .bind(("rid", recordid))
            .bind(("new", new))
            .bind(("status", quotes::StatusDiscriminants::Pending))
            .await
            .map_err(|e| Error::QuotesRepository(anyhow!(e)))?
            .take(0)
            .map_err(|e| Error::QuotesRepository(anyhow!(e)))?;
        match before {
            Some(QuoteDBEntry {
                status: quotes::Status::Pending { .. },
                ..
            }) => Ok(()),
            Some(_) => Err(Error::QuotesRepository(anyhow!("Quote not pending"))),
            None => Err(Error::QuotesRepository(anyhow!(
                "Quote not found or not pending"
            ))),
        }
    }

    async fn execute_authorization(
        &self,
        quote: quotes::Quote,
        exposure: ExposureReservationInput,
    ) -> Result<bcr_common::wire::quotes::CreditAuthorizationReceipt> {
        let _exposure_guard = self.credit_exposure_lock.lock().await;
        let receipt = quote
            .authorization_receipt
            .clone()
            .ok_or(Error::CreditAuthorizationInvalid)?;
        let recordid = surrealdb::RecordId::from_table_key(Self::TABLE, quote.id);
        let reservation_id = uuid::Uuid::new_v4();
        let reservation_rid = surrealdb::RecordId::from_table_key(
            crate::credit_evidence::Store::RESERVATION_TABLE,
            quote.id,
        );
        let ledger_rid = surrealdb::RecordId::from_table_key(
            "credit_exposure_ledgers",
            exposure.mint_id.clone(),
        );
        let event_rid =
            surrealdb::RecordId::from_table_key("credit_exposure_events", uuid::Uuid::new_v4());
        let reservation = ExposureReservationRecord {
            reservation_version: String::from("credit-exposure-reservation-v1"),
            reservation_id,
            mint_id: exposure.mint_id.clone(),
            quote_id: quote.id,
            amount_sat: exposure.amount_sat.to_string(),
            amount: exposure.amount_sat,
            capacity_evidence_id: exposure.capacity_evidence_id,
            state: String::from("reserved"),
            created_at: exposure.now,
            updated_at: exposure.now,
        };
        let event = ExposureEvent {
            event_version: String::from("credit-exposure-event-v1"),
            event_id: uuid::Uuid::new_v4(),
            quote_id: quote.id,
            reservation_id,
            mint_id: exposure.mint_id.clone(),
            amount_sat: exposure.amount_sat.to_string(),
            from_state: None,
            to_state: String::from("reserved"),
            capacity_evidence_id: exposure.capacity_evidence_id,
            recorded_at: exposure.now,
        };
        // Create the single Mint ledger record before the transactional compare-and-increment.
        // A shared, pre-existing key is what lets SurrealDB detect concurrent write conflicts;
        // creating it inside both snapshots would permit a first-write race on an empty database.
        self.db
            .query(
                r#"UPSERT $ledger_rid SET
                    activeAmount = IF activeAmount == NONE THEN 0 ELSE activeAmount END,
                    revision = IF revision == NONE THEN 0 ELSE revision END,
                    mintId = $mint_id,
                    updatedAt = $now"#,
            )
            .bind(("ledger_rid", ledger_rid.clone()))
            .bind(("mint_id", exposure.mint_id.clone()))
            .bind(("now", exposure.now))
            .await
            .map_err(|error| Error::QuotesRepository(anyhow!(error)))?
            .check()
            .map_err(|error| Error::QuotesRepository(anyhow!(error)))?;
        let mut response = self
            .db
            .query(
                r#"
                BEGIN TRANSACTION;
                LET $updated = UPDATE $rid SET status = $new, authorization_receipt = $receipt
                    WHERE status.status == $status AND authorization_receipt == NONE RETURN AFTER;
                IF !$updated { THROW "CREDIT_AUTHORIZATION_CONFLICT" };
                LET $ledger = UPDATE $ledger_rid
                    SET activeAmount += $amount, revision += 1, updatedAt = $now
                    WHERE $existing + activeAmount + $amount <= $limit RETURN AFTER;
                IF !$ledger { THROW "CREDIT_CAPACITY_EXCEEDED" };
                CREATE $reservation_rid CONTENT $reservation;
                CREATE $event_rid CONTENT $event;
                COMMIT TRANSACTION;
                "#,
            )
            .bind(("rid", recordid))
            .bind(("new", quote.status))
            .bind(("receipt", receipt.clone()))
            .bind(("status", quotes::StatusDiscriminants::Pending))
            .bind(("mint_id", exposure.mint_id))
            .bind(("existing", exposure.existing_exposure_sat))
            .bind(("amount", exposure.amount_sat))
            .bind(("limit", exposure.exposure_limit_sat))
            .bind(("ledger_rid", ledger_rid))
            .bind(("now", exposure.now))
            .bind(("reservation_rid", reservation_rid))
            .bind(("reservation", reservation))
            .bind(("event_rid", event_rid))
            .bind(("event", event))
            .await
            .map_err(|error| Error::QuotesRepository(anyhow!(error)))?;
        let errors = response.take_errors();
        if !errors.is_empty() {
            let message = errors
                .values()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("; ");
            if message.contains("CREDIT_CAPACITY_EXCEEDED") {
                return Err(Error::CreditCapacityExceeded);
            }
            if !message.contains("CREDIT_AUTHORIZATION_CONFLICT") {
                return Err(Error::QuotesRepository(anyhow!(message)));
            }
        } else {
            return Ok(receipt);
        }
        let stored = self
            .load(quote.id)
            .await
            .map_err(|error| Error::QuotesRepository(anyhow!(error)))?
            .ok_or_else(|| Error::ResourceNotFound(quote.id.to_string()))?;
        match stored.authorization_receipt {
            Some(existing)
                if existing.operation_id == receipt.operation_id
                    && existing.authorization_digest == receipt.authorization_digest =>
            {
                Ok(existing)
            }
            _ => Err(Error::CreditAuthorizationConflict),
        }
    }

    async fn update_status_if_offered(
        &self,
        qid: uuid::Uuid,
        new: quotes::Status,
        now: crate::TStamp,
    ) -> Result<()> {
        let _exposure_guard = self.credit_exposure_lock.lock().await;
        let target_state = match &new {
            quotes::Status::Accepted { .. } => "committed",
            quotes::Status::Rejected { .. } | quotes::Status::OfferExpired { .. } => "released",
            _ => {
                return Err(Error::QuotesRepository(anyhow!(
                    "offered quote transition has no exposure lifecycle"
                )))
            }
        };
        let recordid = surrealdb::RecordId::from_table_key(Self::TABLE, qid);
        let reservation_rid = surrealdb::RecordId::from_table_key(
            crate::credit_evidence::Store::RESERVATION_TABLE,
            qid,
        );
        let event_rid =
            surrealdb::RecordId::from_table_key("credit_exposure_events", uuid::Uuid::new_v4());
        let response = self
            .db
            .query(
                r#"
                BEGIN TRANSACTION;
                LET $reservation = SELECT * FROM ONLY $reservation_rid;
                IF !$reservation OR $reservation.state != "reserved" {
                    THROW "CREDIT_EXPOSURE_RESERVATION_CONFLICT"
                };
                LET $updated = UPDATE $rid SET status = $new
                    WHERE status.status == $status RETURN BEFORE;
                IF !$updated { THROW "CREDIT_QUOTE_STATUS_CONFLICT" };
                UPDATE $reservation_rid SET state = $target_state, updatedAt = $now;
                IF $target_state == "released" {
                    LET $ledger = UPDATE type::thing("credit_exposure_ledgers", $reservation.mintId)
                        SET activeAmount -= $reservation.amount, revision += 1, updatedAt = $now
                        WHERE activeAmount >= $reservation.amount RETURN AFTER;
                    IF !$ledger { THROW "CREDIT_EXPOSURE_LEDGER_CONFLICT" };
                } ELSE {
                    UPDATE type::thing("credit_exposure_ledgers", $reservation.mintId)
                        SET revision += 1, updatedAt = $now;
                };
                CREATE $event_rid SET
                    eventVersion = "credit-exposure-event-v1",
                    eventId = $event_id,
                    quoteId = $qid,
                    reservationId = $reservation.reservationId,
                    mintId = $reservation.mintId,
                    amountSat = $reservation.amountSat,
                    fromState = "reserved",
                    toState = $target_state,
                    capacityEvidenceId = $reservation.capacityEvidenceId,
                    recordedAt = $now;
                COMMIT TRANSACTION;
                "#,
            )
            .bind(("rid", recordid))
            .bind(("new", new))
            .bind(("status", quotes::StatusDiscriminants::Offered))
            .bind(("reservation_rid", reservation_rid))
            .bind(("target_state", target_state))
            .bind(("now", now))
            .bind(("event_rid", event_rid))
            .bind(("event_id", uuid::Uuid::new_v4()))
            .bind(("qid", qid))
            .await
            .map_err(|error| Error::QuotesRepository(anyhow!(error)))?;
        response
            .check()
            .map(|_| ())
            .map_err(|error| Error::QuotesRepository(anyhow!(error)))
    }

    async fn update_status_if_accepted(&self, qid: uuid::Uuid, new: quotes::Status) -> Result<()> {
        let recordid = surrealdb::RecordId::from_table_key(Self::TABLE, qid);
        let before: Option<QuoteDBEntry> = self
            .db
            .query("UPDATE $rid SET status = $new WHERE status.status == $status RETURN BEFORE")
            .bind(("rid", recordid))
            .bind(("new", new))
            .bind(("status", quotes::StatusDiscriminants::Accepted))
            .await
            .map_err(|e| Error::QuotesRepository(anyhow!(e)))?
            .take(0)
            .map_err(|e| Error::QuotesRepository(anyhow!(e)))?;
        match before {
            Some(QuoteDBEntry {
                status: quotes::Status::Accepted { .. },
                ..
            }) => Ok(()),
            Some(_) => Err(Error::QuotesRepository(anyhow!("Quote not accepted"))),
            None => Err(Error::QuotesRepository(anyhow!(
                "Quote not found or not accepted"
            ))),
        }
    }
    async fn update_status_if_failedebillvalidation(
        &self,
        qid: uuid::Uuid,
        new: quotes::Status,
    ) -> Result<()> {
        let recordid = surrealdb::RecordId::from_table_key(Self::TABLE, qid);
        let before: Option<QuoteDBEntry> = self
            .db
            .query("UPDATE $rid SET status = $new WHERE status.status == $status RETURN BEFORE")
            .bind(("rid", recordid))
            .bind(("new", new))
            .bind(("status", quotes::StatusDiscriminants::FailedEbillValidation))
            .await
            .map_err(|e| Error::QuotesRepository(anyhow!(e)))?
            .take(0)
            .map_err(|e| Error::QuotesRepository(anyhow!(e)))?;
        match before {
            Some(QuoteDBEntry {
                status: quotes::Status::FailedEbillValidation { .. },
                ..
            }) => Ok(()),
            Some(_) => Err(Error::QuotesRepository(anyhow!(
                "Quote not failedebillvalidation"
            ))),
            None => Err(Error::QuotesRepository(anyhow!(
                "Quote not found or not failedebillvalidation"
            ))),
        }
    }

    async fn list_light(
        &self,
        filters: ListFilters,
        sort: Option<SortOrder>,
    ) -> Result<Vec<quotes::LightQuote>> {
        let db_result = self
            .light_list(filters, sort)
            .await
            .map_err(|e| Error::QuotesRepository(anyhow!(e)))?;
        let response = db_result
            .into_iter()
            .map(std::convert::Into::into)
            .collect();
        Ok(response)
    }

    async fn search_by_bill(&self, bill: &BillId, endorser: &NodeId) -> Result<Vec<quotes::Quote>> {
        let res = self
            .search_by_bill(bill, endorser)
            .await
            .map_err(|e| Error::QuotesRepository(anyhow!(e)))?
            .into_iter()
            .map(quotes::Quote::from)
            .collect();
        Ok(res)
    }

    async fn store(&self, quote: quotes::Quote) -> Result<()> {
        if quote.credit_program().is_none() {
            return Err(Error::CreditProgramNotBound(quote.id));
        }
        self.store(quote.into())
            .await
            .map_err(|e| Error::QuotesRepository(anyhow!(e)))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bcr_wdc_utils::keys::test_utils as keys_test;

    #[test]
    fn legacy_entry_without_credit_program_remains_readable_but_unbound() {
        let quote = quotes::Quote::new(
            quotes::BillInfo::random(),
            keys_test::publics()[0],
            TStamp::default(),
            quotes::test_credit_program_binding(),
        );
        let mut value = serde_json::to_value(QuoteDBEntry::from(quote)).unwrap();
        value
            .as_object_mut()
            .expect("quote entry is an object")
            .remove("credit_program");

        let legacy: QuoteDBEntry = serde_json::from_value(value).unwrap();
        let restored = quotes::Quote::from(legacy);

        assert!(restored.credit_program().is_none());
    }
}
