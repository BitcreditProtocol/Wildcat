#![allow(dead_code)]
// ----- standard library imports
use std::str::FromStr;
// ----- extra library imports
use anyhow::anyhow;
use async_trait::async_trait;
use bcr_common::core::{BillId, NodeId};
use bcr_wdc_utils::postgres;
use sqlx::{types::Json, PgPool, QueryBuilder};
use strum::IntoDiscriminant;
use uuid::Uuid;
// ----- local imports
use crate::{
    error::{Error, Result},
    persistence, quotes, service, TStamp,
};

// ----- end imports

// ///////////////////////////////////////////////////////////////////////// Versioned blob for quotes

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "version", content = "data")]
enum QuoteBlob {
    V1(QuoteBlobV1),
    V2(QuoteBlobV2),
    V3(Box<QuoteBlobV3>),
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct QuoteBlobV1 {
    bill: quotes::BillInfo,
    status: quotes::Status,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct QuoteBlobV2 {
    bill: quotes::BillInfo,
    status: quotes::Status,
    credit_program: quotes::CreditProgramBinding,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct QuoteBlobV3 {
    bill: quotes::BillInfo,
    status: quotes::Status,
    credit_program: quotes::CreditProgramBinding,
    authorization_receipt: Option<bcr_common::wire::quotes::CreditAuthorizationReceipt>,
}

// ///////////////////////////////////////////////////////////////////////// QuoteRow

#[derive(sqlx::FromRow)]
struct QuoteRow {
    pub qid: Uuid,
    pub status: String,
    pub submitted: TStamp,
    pub maturity_date: chrono::NaiveDate,
    pub bill_id: String,
    pub bill_sum: i64,
    pub bill_drawee_id: String,
    pub bill_drawer_id: String,
    pub bill_payer_id: String,
    pub bill_holder_id: String,
    pub blob: Json<QuoteBlob>,
}

fn quote_to_row(quote: quotes::Quote) -> Result<QuoteRow> {
    let quotes::Quote {
        status,
        id,
        bill,
        submitted,
        credit_program,
        authorization_receipt,
    } = quote;
    let maturity_date = bill.maturity_date;
    let bill_id = bill.id.to_string();
    let bill_sum = i64::try_from(bill.sum.to_sat()).expect("21x10^14 satoshis fits in i64");
    let bill_drawee_id = bill.drawee.node_id.to_string();
    let bill_drawer_id = bill.drawer.node_id.to_string();
    let bill_payer_id = bill.payee.node_id().to_string();
    let bill_holder_id = bill.current_holder.node_id().to_string();
    let status_d = status.discriminant().to_string();
    let credit_program = credit_program.ok_or(Error::CreditProgramNotBound(id))?;
    let blob_v3 = QuoteBlobV3 {
        bill,
        status,
        credit_program,
        authorization_receipt,
    };
    let blob = Json(QuoteBlob::V3(Box::new(blob_v3)));
    Ok(QuoteRow {
        qid: id,
        status: status_d,
        submitted,
        maturity_date,
        bill_id,
        bill_sum,
        bill_drawee_id,
        bill_drawer_id,
        bill_payer_id,
        bill_holder_id,
        blob,
    })
}

fn quote_from_row(row: QuoteRow) -> Result<quotes::Quote> {
    let (bill, status, credit_program, authorization_receipt) = match row.blob.0 {
        QuoteBlob::V1(blob_v1) => {
            let QuoteBlobV1 { bill, status } = blob_v1;
            (bill, status, None, None)
        }
        QuoteBlob::V2(blob_v2) => {
            let QuoteBlobV2 {
                bill,
                status,
                credit_program,
            } = blob_v2;
            (bill, status, Some(credit_program), None)
        }
        QuoteBlob::V3(blob_v3) => {
            let QuoteBlobV3 {
                bill,
                status,
                credit_program,
                authorization_receipt,
            } = *blob_v3;
            (bill, status, Some(credit_program), authorization_receipt)
        }
    };
    let expected_d = status.discriminant().to_string();
    if expected_d != row.status {
        tracing::error!(
            "status mismatch for {}: expected {expected_d}, found {}",
            row.qid,
            row.status,
        );
    }
    Ok(quotes::Quote {
        id: row.qid,
        bill,
        status,
        submitted: row.submitted,
        credit_program,
        authorization_receipt,
    })
}

// ///////////////////////////////////////////////////////////////////////// DBQuotes

fn add_filter_statement<'q, DB, F>(
    query: &mut QueryBuilder<'q, DB>,
    first: bool,
    filter: Option<F>,
    statement: &str,
) -> bool
where
    DB: sqlx::Database,
    F: sqlx::Encode<'q, DB> + sqlx::Type<DB> + 'q,
{
    if let Some(filter) = filter {
        if first {
            query.push(" WHERE ");
        } else {
            query.push(" AND ");
        }
        query.push(statement).push_bind(filter);
        false
    } else {
        first
    }
}

#[derive(Debug, Clone)]
pub struct DBQuotes {
    pool: PgPool,
}

impl DBQuotes {
    pub async fn new(cfg: postgres::DBConnConfig) -> Result<Self> {
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(cfg.max_connections)
            .connect(&cfg.connection)
            .await
            .map_err(|e| Error::QuotesRepository(anyhow!(e)))?;
        Ok(Self { pool })
    }

    pub fn from_pool(pool: PgPool) -> Self {
        Self { pool }
    }

    async fn update_status_if(
        &self,
        id: Uuid,
        expected: quotes::StatusDiscriminants,
        new_status: quotes::Status,
    ) -> Result<()> {
        let new_disc = new_status.discriminant().to_string();
        let new_status_val =
            serde_json::to_value(&new_status).map_err(|e| Error::QuotesRepository(anyhow!(e)))?;
        let rows = sqlx::query!(
            r#"
            UPDATE quote_quotes
            SET status = $2,
                blob = jsonb_set(blob, '{data,status}', $3, true)
            WHERE qid = $1 AND status = $4
            "#,
            id,
            new_disc,
            new_status_val,
            expected.to_string(),
        )
        .execute(&self.pool)
        .await
        .map_err(|e| Error::QuotesRepository(anyhow!(e)))?
        .rows_affected();
        if rows == 0 {
            return Err(Error::QuotesRepository(anyhow!(
                "quote {id} not found or not {expected}"
            )));
        }
        Ok(())
    }
}

#[async_trait]
impl persistence::Repository for DBQuotes {
    async fn load(&self, id: uuid::Uuid) -> Result<Option<quotes::Quote>> {
        let row = sqlx::query_as!(
            QuoteRow,
            r#"
            SELECT qid, status, submitted, maturity_date, bill_id, bill_sum,
                   bill_drawee_id, bill_drawer_id, bill_payer_id, bill_holder_id, blob as "blob: Json<QuoteBlob>"
            FROM quote_quotes
            WHERE qid = $1
            "#,
            id,
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| Error::QuotesRepository(anyhow!(e)))?;
        let Some(row) = row else {
            return Ok(None);
        };
        let quote = quote_from_row(row)?;
        Ok(Some(quote))
    }

    async fn update_status_if_pending(
        &self,
        id: uuid::Uuid,
        new_status: quotes::Status,
    ) -> Result<()> {
        self.update_status_if(id, quotes::StatusDiscriminants::Pending, new_status)
            .await
    }

    async fn execute_authorization(
        &self,
        quote: quotes::Quote,
    ) -> Result<bcr_common::wire::quotes::CreditAuthorizationReceipt> {
        let receipt = quote
            .authorization_receipt
            .clone()
            .ok_or(Error::CreditAuthorizationInvalid)?;
        let row = quote_to_row(quote)?;
        let blob = serde_json::to_value(row.blob.0)
            .map_err(|error| Error::QuotesRepository(anyhow!(error)))?;
        let rows = sqlx::query(
            r#"
            UPDATE quote_quotes
            SET status = $2, blob = $3
            WHERE qid = $1 AND status = $4
              AND (blob->'data'->'authorization_receipt') IS NULL
            "#,
        )
        .bind(row.qid)
        .bind(row.status)
        .bind(blob)
        .bind(quotes::StatusDiscriminants::Pending.to_string())
        .execute(&self.pool)
        .await
        .map_err(|error| Error::QuotesRepository(anyhow!(error)))?
        .rows_affected();
        if rows == 1 {
            return Ok(receipt);
        }
        let stored = <Self as persistence::Repository>::load(self, row.qid)
            .await?
            .ok_or_else(|| Error::ResourceNotFound(row.qid.to_string()))?;
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
        id: uuid::Uuid,
        new_status: quotes::Status,
    ) -> Result<()> {
        self.update_status_if(id, quotes::StatusDiscriminants::Offered, new_status)
            .await
    }

    async fn update_status_if_accepted(
        &self,
        id: uuid::Uuid,
        new_status: quotes::Status,
    ) -> Result<()> {
        self.update_status_if(id, quotes::StatusDiscriminants::Accepted, new_status)
            .await
    }

    async fn update_status_if_failedebillvalidation(
        &self,
        id: uuid::Uuid,
        new_status: quotes::Status,
    ) -> Result<()> {
        self.update_status_if(
            id,
            quotes::StatusDiscriminants::FailedEbillValidation,
            new_status,
        )
        .await
    }

    async fn list_light(
        &self,
        filters: service::ListFilters,
        sort: Option<service::SortOrder>,
    ) -> Result<Vec<quotes::LightQuote>> {
        let mut qb: QueryBuilder<'_, sqlx::Postgres> =
            QueryBuilder::new("SELECT qid, status, bill_sum, maturity_date FROM quote_quotes");

        let first = add_filter_statement(
            &mut qb,
            true,
            filters.bill_maturity_date_from,
            "maturity_date >= ",
        );
        let first = add_filter_statement(
            &mut qb,
            first,
            filters.bill_maturity_date_to,
            "maturity_date <= ",
        );
        let status = filters.status.map(|s| s.to_string());
        let first = add_filter_statement(&mut qb, first, status, "status = ");
        let bid = filters.bill_id.as_ref().map(|id| id.to_string());
        let first = add_filter_statement(&mut qb, first, bid, "bill_id = ");
        let drawer_id = filters.bill_drawer_id.as_ref().map(|id| id.to_string());
        let first = add_filter_statement(&mut qb, first, drawer_id, "bill_drawer_id = ");
        let drawee_id = filters.bill_drawee_id.as_ref().map(|id| id.to_string());
        let first = add_filter_statement(&mut qb, first, drawee_id, "bill_drawee_id = ");
        let payer_id = filters.bill_payer_id.as_ref().map(|id| id.to_string());
        let first = add_filter_statement(&mut qb, first, payer_id, "bill_payer_id = ");
        let holder_id = filters.bill_holder_id.as_ref().map(|id| id.to_string());
        add_filter_statement(&mut qb, first, holder_id, "bill_holder_id = ");
        if let Some(sort) = sort {
            match sort {
                service::SortOrder::BillMaturityDateAsc => {
                    qb.push(" ORDER BY maturity_date ASC");
                }
                service::SortOrder::BillMaturityDateDesc => {
                    qb.push(" ORDER BY maturity_date DESC");
                }
                service::SortOrder::SubmittedAsc => {
                    qb.push(" ORDER BY submitted ASC");
                }
                service::SortOrder::SubmittedDesc => {
                    qb.push(" ORDER BY submitted DESC");
                }
            }
        }
        let rows = qb
            .build_query_as::<LightQuoteRow>()
            .fetch_all(&self.pool)
            .await
            .map_err(|e| Error::QuotesRepository(anyhow!(e)))?;
        rows.into_iter()
            .map(|r| {
                Ok(quotes::LightQuote {
                    id: r.qid,
                    status: quotes::StatusDiscriminants::from_str(&r.status)
                        .map_err(|e| Error::QuotesRepository(anyhow!(e)))?,
                    sum: bitcoin::Amount::from_sat(
                        u64::try_from(r.bill_sum).expect("bill_sum always positive"),
                    ),
                    maturity_date: r.maturity_date,
                })
            })
            .collect()
    }

    async fn search_by_bill(&self, bill: &BillId, endorser: &NodeId) -> Result<Vec<quotes::Quote>> {
        let bill_id = bill.to_string();
        let endorser_id = endorser.to_string();
        let results = sqlx::query_as!(
            QuoteRow,
            r#"
            SELECT qid, status, submitted, maturity_date, bill_id, bill_sum,
                   bill_drawee_id, bill_drawer_id, bill_payer_id, bill_holder_id, blob as "blob: Json<QuoteBlob>"
            FROM quote_quotes
            WHERE bill_id = $1 AND bill_holder_id = $2
            ORDER BY submitted DESC
            "#,
            bill_id,
            endorser_id
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| Error::QuotesRepository(anyhow!(e)))?;
        results.into_iter().map(quote_from_row).collect()
    }

    async fn store(&self, quote: quotes::Quote) -> Result<()> {
        let row = quote_to_row(quote)?;
        let blob = row.blob.0;
        let json_blob =
            serde_json::to_value(&blob).map_err(|e| Error::QuotesRepository(anyhow!(e)))?;
        let result = sqlx::query!(
            r#"
            INSERT INTO quote_quotes (qid, status, submitted, maturity_date, bill_id, bill_sum,
                                bill_drawee_id, bill_drawer_id, bill_payer_id, bill_holder_id, blob)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
            ON CONFLICT (qid) DO NOTHING
            "#,
            row.qid,
            row.status,
            row.submitted,
            row.maturity_date,
            row.bill_id,
            row.bill_sum,
            row.bill_drawee_id,
            row.bill_drawer_id,
            row.bill_payer_id,
            row.bill_holder_id,
            json_blob,
        )
        .execute(&self.pool)
        .await
        .map_err(|e| Error::QuotesRepository(anyhow!(e)))?;
        if result.rows_affected() == 0 {
            return Err(Error::QuotesRepository(anyhow!(
                "quote already exists: {}",
                row.qid
            )));
        }
        Ok(())
    }
}

#[derive(sqlx::FromRow)]
struct LightQuoteRow {
    qid: Uuid,
    status: String,
    bill_sum: i64,
    maturity_date: chrono::NaiveDate,
}

#[cfg(test)]
mod tests {
    use super::*;
    use bcr_wdc_utils::keys::test_utils as keys_test;

    fn pending_quote() -> quotes::Quote {
        quotes::Quote::new(
            quotes::BillInfo::random(),
            keys_test::publics()[0],
            TStamp::default(),
            quotes::test_credit_program_binding(),
        )
    }

    #[test]
    fn v2_blob_roundtrip_preserves_credit_program() {
        let quote = pending_quote();
        let expected = quote.credit_program().cloned();

        let row = quote_to_row(quote).unwrap();
        let restored = quote_from_row(row).unwrap();

        assert_eq!(restored.credit_program(), expected.as_ref());
    }

    #[test]
    fn v1_blob_restores_as_unbound_and_cannot_be_offered() {
        let quote = pending_quote();
        let mut row = quote_to_row(quote.clone()).unwrap();
        row.blob = Json(QuoteBlob::V1(QuoteBlobV1 {
            bill: quote.bill,
            status: quote.status,
        }));

        let mut restored = quote_from_row(row).unwrap();
        let keyset_id = bcr_common::core_tests::generate_random_ecash_keyset().0.id;
        let result = restored.offer(keyset_id, TStamp::default(), bitcoin::Amount::from_sat(1));

        assert!(restored.credit_program().is_none());
        assert!(matches!(result, Err(Error::CreditProgramNotBound(_))));
    }
}
