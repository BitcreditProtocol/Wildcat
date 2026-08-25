// ----- standard library imports
// ----- extra library imports
use anyhow::anyhow;
use async_trait::async_trait;
use bcr_common::core::{BillId, NodeId};
use bcr_common::wire::quotes::SignedCreditQuoteReissuePermit;
use bcr_wdc_utils::surreal;
use std::sync::Arc;
use surrealdb::Result as SurrealResult;
use surrealdb::{engine::any::Any, Surreal};
use tokio::sync::Mutex;
use uuid::Uuid;
// ----- local imports
use crate::{
    authorization::same_quote_reissue_authority,
    error::{Error, Result},
    persistence::{
        same_executed_quote, same_pending_quote_request, ExposureReservationInput, Repository,
    },
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
    /// Commit-time compare-and-set marker shared by normal and reviewed reissue creation.
    #[serde(default)]
    successor_quote_id: Option<uuid::Uuid>,
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

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct QuoteReissueRecord {
    previous_quote_id: uuid::Uuid,
    reissued_quote_id: uuid::Uuid,
    permit_digest: String,
    signed: SignedCreditQuoteReissuePermit,
    minting_pubkey: bcr_common::cashu::PublicKey,
    consumed_at: crate::TStamp,
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
            successor_quote_id: None,
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
    quote_reissue_lock: Arc<Mutex<()>>,
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
    const QUOTE_REISSUE_TABLE: &'static str = "credit_quote_reissue_permits";

    pub async fn new(cfg: surreal::DBConnConfig) -> SurrealResult<Self> {
        let db_connection = Surreal::<Any>::init();
        db_connection.connect(cfg.connection).await?;
        db_connection.use_ns(cfg.namespace).await?;
        db_connection.use_db(cfg.database).await?;
        Ok(Self {
            db: db_connection,
            credit_exposure_lock: Arc::new(Mutex::new(())),
            quote_reissue_lock: Arc::new(Mutex::new(())),
        })
    }

    #[cfg(test)]
    pub(super) fn independent_test_handle(&self) -> Self {
        Self {
            db: self.db.clone(),
            credit_exposure_lock: Arc::new(Mutex::new(())),
            quote_reissue_lock: Arc::new(Mutex::new(())),
        }
    }

    async fn load(&self, qid: Uuid) -> SurrealResult<Option<QuoteDBEntry>> {
        let rid = surrealdb::RecordId::from_table_key(Self::TABLE, qid);
        self.db.select(rid).await
    }

    async fn store(&self, quote: QuoteDBEntry) -> SurrealResult<Option<QuoteDBEntry>> {
        let rid = surrealdb::RecordId::from_table_key(Self::TABLE, quote.qid);
        self.db.insert(rid).content(quote).await
    }

    async fn load_quote_reissue_record(
        &self,
        previous_quote_id: Uuid,
    ) -> SurrealResult<Option<QuoteReissueRecord>> {
        let rid = surrealdb::RecordId::from_table_key(Self::QUOTE_REISSUE_TABLE, previous_quote_id);
        self.db.select(rid).await
    }

    async fn resolve_store_conflict(&self, requested: &quotes::Quote) -> Result<Uuid> {
        self.search_by_bill(&requested.bill.id, &requested.bill.current_holder.node_id())
            .await
            .map_err(|error| Error::QuotesRepository(anyhow!(error)))?
            .into_iter()
            .map(quotes::Quote::from)
            .max_by_key(|candidate| (candidate.submitted, candidate.id))
            .filter(|candidate| same_pending_quote_request(candidate, requested))
            .map(|candidate| candidate.id)
            .ok_or(Error::CreditQuoteReissueConflict)
    }

    async fn resolve_quote_reissue_receipt(
        &self,
        previous_quote_id: Uuid,
        signed: &SignedCreditQuoteReissuePermit,
        requested: &quotes::Quote,
    ) -> Result<Option<Uuid>> {
        let Some(existing) = self
            .load_quote_reissue_record(previous_quote_id)
            .await
            .map_err(|error| Error::QuotesRepository(anyhow!(error)))?
        else {
            return Ok(None);
        };
        let same_permit = existing.permit_digest == signed.permit_digest
            || same_quote_reissue_authority(&existing.signed.permit, &signed.permit);
        let stored = self
            .load(existing.reissued_quote_id)
            .await
            .map_err(|error| Error::QuotesRepository(anyhow!(error)))?
            .map(quotes::Quote::from);
        match stored {
            Some(stored)
                if same_permit
                    && existing.reissued_quote_id == signed.permit.reissued_mint_quote_id
                    && same_executed_quote(&stored, requested, existing.minting_pubkey) =>
            {
                Ok(Some(existing.reissued_quote_id))
            }
            _ => Err(Error::CreditQuoteReissueConflict),
        }
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

    async fn release_committed_exposure(&self, qid: uuid::Uuid, now: crate::TStamp) -> Result<()> {
        let _exposure_guard = self.credit_exposure_lock.lock().await;
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
                IF !$reservation { THROW "CREDIT_EXPOSURE_RESERVATION_MISSING" };
                IF $reservation.state == "committed" {
                    LET $ledger = UPDATE type::thing("credit_exposure_ledgers", $reservation.mintId)
                        SET activeAmount -= $reservation.amount, revision += 1, updatedAt = $now
                        WHERE activeAmount >= $reservation.amount RETURN AFTER;
                    IF !$ledger { THROW "CREDIT_EXPOSURE_LEDGER_CONFLICT" };
                    UPDATE $reservation_rid SET state = "released", updatedAt = $now;
                    CREATE $event_rid SET
                        eventVersion = "credit-exposure-event-v1",
                        eventId = $event_id,
                        quoteId = $qid,
                        reservationId = $reservation.reservationId,
                        mintId = $reservation.mintId,
                        amountSat = $reservation.amountSat,
                        fromState = "committed",
                        toState = "released",
                        capacityEvidenceId = $reservation.capacityEvidenceId,
                        recordedAt = $now;
                } ELSE {
                    IF $reservation.state != "released" {
                        THROW "CREDIT_EXPOSURE_RESERVATION_CONFLICT"
                    };
                };
                COMMIT TRANSACTION;
                "#,
            )
            .bind(("reservation_rid", reservation_rid))
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

    async fn store_if_latest(
        &self,
        expected_latest: Option<Uuid>,
        quote: quotes::Quote,
    ) -> Result<Uuid> {
        if quote.credit_program().is_none() {
            return Err(Error::CreditProgramNotBound(quote.id));
        }
        let quote_id = quote.id;
        let requested_quote = quote.clone();
        let quote_record = QuoteDBEntry::from(quote);
        let quote_rid = surrealdb::RecordId::from_table_key(Self::TABLE, quote_id);
        let previous_rid =
            surrealdb::RecordId::from_table_key(Self::TABLE, expected_latest.unwrap_or_default());
        let response = self
            .db
            .query(
                r#"
                BEGIN TRANSACTION;
                LET $head = SELECT qid, submitted FROM type::table($quotes_table)
                    WHERE bill.id == $bill_id
                        AND (
                            bill.current_holder.Anon.node_id == $holder_id
                            OR bill.current_holder.Ident.node_id == $holder_id
                        )
                    ORDER BY submitted DESC, qid DESC
                    LIMIT 1;
                IF $expect_empty {
                    IF array::len($head) > 0 { THROW "CREDIT_QUOTE_HEAD_CONFLICT" };
                } ELSE {
                    IF array::len($head) != 1 OR $head[0].qid != $expected_latest {
                        THROW "CREDIT_QUOTE_HEAD_CONFLICT"
                    };
                    LET $claimed = UPDATE $previous_rid SET successor_quote_id = $quote_id
                        WHERE successor_quote_id == NONE RETURN BEFORE;
                    IF !$claimed { THROW "CREDIT_QUOTE_HEAD_CONFLICT" };
                };
                LET $target = SELECT * FROM ONLY $quote_rid;
                IF $target { THROW "CREDIT_QUOTE_STORE_CONFLICT" };
                CREATE $quote_rid CONTENT $quote;
                COMMIT TRANSACTION;
                "#,
            )
            .bind(("quotes_table", Self::TABLE))
            .bind(("bill_id", quote_record.bill.id.clone()))
            .bind(("holder_id", quote_record.bill.current_holder.node_id()))
            .bind(("expect_empty", expected_latest.is_none()))
            .bind(("expected_latest", expected_latest.unwrap_or_default()))
            .bind(("previous_rid", previous_rid))
            .bind(("quote_id", quote_id))
            .bind(("quote_rid", quote_rid))
            .bind(("quote", quote_record.clone()))
            .await;
        let mut response = match response {
            Ok(response) => response,
            Err(error) if is_transaction_conflict(&error.to_string()) => {
                return self.resolve_store_conflict(&requested_quote).await;
            }
            Err(error) => return Err(Error::QuotesRepository(anyhow!(error))),
        };
        let errors = response.take_errors();
        if errors.is_empty() {
            return Ok(quote_id);
        }
        let message = errors
            .values()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("; ");
        if message.contains("CREDIT_QUOTE_HEAD_CONFLICT") {
            return self.resolve_store_conflict(&requested_quote).await;
        }
        Err(Error::QuotesRepository(anyhow!(message)))
    }

    async fn execute_quote_reissue(
        &self,
        signed: SignedCreditQuoteReissuePermit,
        quote: quotes::Quote,
        consumed_at: crate::TStamp,
    ) -> Result<Uuid> {
        let _guard = self.quote_reissue_lock.lock().await;
        let previous_quote_id = signed.permit.previous_mint_quote_id;
        let quote_id = signed.permit.reissued_mint_quote_id;
        if quote.id != quote_id {
            return Err(Error::CreditQuoteReissueConflict);
        }
        if let Some(id) = self
            .resolve_quote_reissue_receipt(previous_quote_id, &signed, &quote)
            .await?
        {
            return Ok(id);
        }

        crate::service::validate_basic_ebill_amount(&quote.bill)?;

        let expires_at = chrono::DateTime::parse_from_rfc3339(&signed.permit.expires_at)
            .map_err(|_| Error::CreditQuoteReissueInvalid)?
            .with_timezone(&chrono::Utc);
        let minting_pubkey = match &quote.status {
            quotes::Status::Pending { wallet_pubkey } => *wallet_pubkey,
            _ => return Err(Error::CreditQuoteReissueConflict),
        };
        let requested_quote = quote.clone();
        let quote_record = QuoteDBEntry::from(quote);
        let receipt = QuoteReissueRecord {
            previous_quote_id,
            reissued_quote_id: quote_id,
            permit_digest: signed.permit_digest.clone(),
            signed: signed.clone(),
            minting_pubkey,
            consumed_at,
        };
        let receipt_rid =
            surrealdb::RecordId::from_table_key(Self::QUOTE_REISSUE_TABLE, previous_quote_id);
        let previous_rid = surrealdb::RecordId::from_table_key(Self::TABLE, previous_quote_id);
        let quote_rid = surrealdb::RecordId::from_table_key(Self::TABLE, quote_id);
        let response = self
            .db
            .query(
                r#"
                BEGIN TRANSACTION;
                LET $receipt_before = SELECT * FROM ONLY $receipt_rid;
                IF $receipt_before { THROW "CREDIT_QUOTE_REISSUE_CONFLICT" };
                IF $maturity_date < $today { THROW "CREDIT_QUOTE_REISSUE_MATURED" };
                LET $old = SELECT * FROM ONLY $previous_rid;
                IF !$old OR $old.status.status != $denied {
                    THROW "CREDIT_QUOTE_REISSUE_CONFLICT"
                };
                LET $claimed = UPDATE $previous_rid SET successor_quote_id = $quote_id
                    WHERE successor_quote_id == NONE RETURN BEFORE;
                IF !$claimed { THROW "CREDIT_QUOTE_REISSUE_CONFLICT" };
                LET $newer = SELECT VALUE qid FROM type::table($quotes_table)
                    WHERE bill.id == $bill_id
                        AND (
                            bill.current_holder.Anon.node_id == $holder_id
                            OR bill.current_holder.Ident.node_id == $holder_id
                        )
                        AND qid != $previous_quote_id
                        AND submitted >= $old.submitted
                    LIMIT 1;
                IF array::len($newer) > 0 { THROW "CREDIT_QUOTE_REISSUE_CONFLICT" };
                LET $target = SELECT * FROM ONLY $quote_rid;
                IF $target { THROW "CREDIT_QUOTE_REISSUE_CONFLICT" };
                IF $expires_at <= $now { THROW "CREDIT_QUOTE_REISSUE_EXPIRED" };
                CREATE $receipt_rid CONTENT $receipt;
                CREATE $quote_rid CONTENT $quote;
                COMMIT TRANSACTION;
                "#,
            )
            .bind(("receipt_rid", receipt_rid))
            .bind(("previous_rid", previous_rid))
            .bind(("maturity_date", quote_record.bill.maturity_date))
            .bind(("today", consumed_at.date_naive()))
            .bind(("denied", quotes::StatusDiscriminants::Denied))
            .bind(("quotes_table", Self::TABLE))
            .bind(("bill_id", quote_record.bill.id.clone()))
            .bind(("holder_id", quote_record.bill.current_holder.node_id()))
            .bind(("previous_quote_id", previous_quote_id))
            .bind(("quote_id", quote_id))
            .bind(("quote_rid", quote_rid))
            .bind(("expires_at", expires_at))
            .bind(("now", consumed_at))
            .bind(("receipt", receipt))
            .bind(("quote", quote_record.clone()))
            .await;
        let mut response = match response {
            Ok(response) => response,
            Err(error) if is_transaction_conflict(&error.to_string()) => {
                if let Some(id) = self
                    .resolve_quote_reissue_receipt(previous_quote_id, &signed, &requested_quote)
                    .await?
                {
                    return Ok(id);
                }
                return Err(Error::CreditQuoteReissueConflict);
            }
            Err(error) => return Err(Error::QuotesRepository(anyhow!(error))),
        };
        let errors = response.take_errors();
        if errors.is_empty() {
            return Ok(quote_id);
        }
        let message = errors
            .values()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("; ");

        // A concurrent first execution or a dropped response resolves through the durable receipt.
        if let Some(id) = self
            .resolve_quote_reissue_receipt(previous_quote_id, &signed, &requested_quote)
            .await?
        {
            return Ok(id);
        }
        if message.contains("CREDIT_QUOTE_REISSUE_MATURED") {
            Err(Error::InvalidInput(String::from(
                "maturity date must be >= today",
            )))
        } else if message.contains("CREDIT_QUOTE_REISSUE_EXPIRED") {
            Err(Error::CreditQuoteReissueInvalid)
        } else if message.contains("CREDIT_QUOTE_REISSUE_CONFLICT")
            || is_transaction_conflict(&message)
        {
            Err(Error::CreditQuoteReissueConflict)
        } else {
            Err(Error::QuotesRepository(anyhow!(message)))
        }
    }
}

fn is_transaction_conflict(message: &str) -> bool {
    message.contains("read or write conflict") || message.contains("transaction can be retried")
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
