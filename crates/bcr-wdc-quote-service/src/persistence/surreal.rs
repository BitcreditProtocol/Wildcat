// ----- standard library imports
// ----- extra library imports
use anyhow::anyhow;
use async_trait::async_trait;
use bcr_ebill_core::bill::BillId;
use bcr_ebill_core::NodeId;
use surrealdb::Result as SurrealResult;
use surrealdb::{engine::any::Any, Surreal};
use uuid::Uuid;
// ----- local imports
use crate::{
    error::{Error, Result},
    quotes,
    service::{ListFilters, Repository, SortOrder},
    TStamp,
};

// ----- end imports

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct QuoteDBEntry {
    qid: surrealdb::Uuid, // can't be `id`, reserved word in surreal
    bill: quotes::BillInfo,
    submitted: TStamp,
    status: quotes::Status,
}

impl From<QuoteDBEntry> for quotes::Quote {
    fn from(dbq: QuoteDBEntry) -> Self {
        Self {
            id: dbq.qid,
            bill: dbq.bill,
            submitted: dbq.submitted,
            status: dbq.status,
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
        }
    }
}

#[derive(Debug, Clone, serde::Deserialize)]
struct LightQuoteDBEntry {
    qid: uuid::Uuid,
    status: quotes::StatusDiscriminants,
    sum: bitcoin::Amount,
    maturity_date: TStamp,
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

#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct ConnectionConfig {
    pub connection: String,
    pub namespace: String,
    pub database: String,
    pub table: String,
}

#[derive(Debug, Clone)]
pub struct DBQuotes {
    db: Surreal<surrealdb::engine::any::Any>,
    table: String,
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
    pub async fn new(cfg: ConnectionConfig) -> SurrealResult<Self> {
        let db_connection = Surreal::<Any>::init();
        db_connection.connect(cfg.connection).await?;
        db_connection.use_ns(cfg.namespace).await?;
        db_connection.use_db(cfg.database).await?;
        Ok(Self {
            db: db_connection,
            table: cfg.table,
        })
    }

    async fn load(&self, qid: Uuid) -> SurrealResult<Option<QuoteDBEntry>> {
        let rid = surrealdb::RecordId::from_table_key(&self.table, qid);
        self.db.select(rid).await
    }

    async fn store(&self, quote: QuoteDBEntry) -> SurrealResult<Option<QuoteDBEntry>> {
        let rid = surrealdb::RecordId::from_table_key(&self.table, quote.qid);
        self.db.insert(rid).content(quote).await
    }

    async fn light_list(
        &self,
        filters: ListFilters,
        sort: Option<SortOrder>,
    ) -> SurrealResult<Vec<LightQuoteDBEntry>> {
        let mut statement = String::from(
            "SELECT qid, status.status as status, bill.sum AS sum, bill.maturity_date as maturity_date FROM type::table($table)",
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
            };
        }
        let query = self
            .db
            .query(statement)
            .bind(("table", self.table.clone()))
            .bind(filters);

        query.await?.take(0)
    }

    async fn list_by_status(
        &self,
        status: quotes::StatusDiscriminants,
        since: Option<TStamp>,
    ) -> SurrealResult<Vec<Uuid>> {
        let mut query = String::from(
            "SELECT qid, submitted FROM type::table($table) WHERE status.status == $status",
        );
        if since.is_some() {
            query += " AND submitted >= $since";
        }
        query += " ORDER BY submitted DESC";

        let mut db_query = self
            .db
            .query(query)
            .bind(("table", self.table.clone()))
            .bind(("status", status));

        if let Some(since) = since {
            db_query = db_query.bind(("since", since));
        }

        db_query.await?.take("qid")
    }

    async fn search_by_bill(
        &self,
        bill: &BillId,
        endorser: &NodeId,
    ) -> SurrealResult<Vec<QuoteDBEntry>> {
        let results: Vec<QuoteDBEntry> = self.db
            .query("SELECT * FROM type::table($table) WHERE bill.id == $bill AND (bill.current_holder.Anon.node_id == $endorser OR bill.current_holder.Ident.node_id == $endorser) ORDER BY submitted DESC")
            .bind(("table", self.table.clone()))
            .bind(("bill", bill.to_owned()))
            .bind(("endorser", endorser.to_owned())).await?.take(0)?;
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
        let recordid = surrealdb::RecordId::from_table_key(&self.table, qid);
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
        let before = before.ok_or(Error::QuotesRepository(anyhow!(
            "Quote not found or not pending"
        )))?;
        if !matches!(before.status, quotes::Status::Pending { .. }) {
            return Err(Error::QuotesRepository(anyhow!("Quote not pending")));
        }
        Ok(())
    }

    async fn update_status_if_offered(&self, qid: uuid::Uuid, new: quotes::Status) -> Result<()> {
        let recordid = surrealdb::RecordId::from_table_key(&self.table, qid);
        let before: Option<QuoteDBEntry> = self
            .db
            .query("UPDATE $rid SET status = $new WHERE status.status == $status RETURN BEFORE")
            .bind(("rid", recordid))
            .bind(("new", new))
            .bind(("status", quotes::StatusDiscriminants::Offered))
            .await
            .map_err(|e| Error::QuotesRepository(anyhow!(e)))?
            .take(0)
            .map_err(|e| Error::QuotesRepository(anyhow!(e)))?;
        if before.is_none() {
            return Err(Error::QuotesRepository(anyhow!(
                "Quote not found or not offered"
            )));
        }
        let before = before.unwrap();
        if !matches!(before.status, quotes::Status::Offered { .. }) {
            return Err(Error::QuotesRepository(anyhow!("Quote not offered")));
        }
        Ok(())
    }

    async fn list_pendings(&self, since: Option<TStamp>) -> Result<Vec<Uuid>> {
        self.list_by_status(quotes::StatusDiscriminants::Pending, since)
            .await
            .map_err(|e| Error::QuotesRepository(anyhow!(e)))
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
        self.store(quote.into())
            .await
            .map_err(|e| Error::QuotesRepository(anyhow!(e)))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{quotes::BillInfo, service};
    use bcr_ebill_core::contact::BillParticipant;
    use bcr_wdc_utils::keys::test_utils as keys_test;
    use bcr_wdc_webapi::test_utils::{random_bill_id, random_identity_public_data};
    use std::str::FromStr;
    use surrealdb::RecordId;

    async fn init_mem_db() -> DBQuotes {
        let sdb = Surreal::<Any>::init();
        sdb.connect("mem://").await.unwrap();
        sdb.use_ns("test").await.unwrap();
        sdb.use_db("test").await.unwrap();
        DBQuotes {
            db: sdb,
            table: String::from("test"),
        }
    }

    impl Default for BillInfo {
        fn default() -> Self {
            Self {
                id: random_bill_id(),
                drawee: random_identity_public_data().1.into(),
                drawer: random_identity_public_data().1.into(),
                payee: BillParticipant::Ident(random_identity_public_data().1.into()),
                endorsees: Vec::default(),
                current_holder: BillParticipant::Ident(random_identity_public_data().1.into()),
                sum: bitcoin::Amount::default(),
                maturity_date: TStamp::default(),
                file_urls: Vec::default(),
            }
        }
    }

    #[tokio::test]
    async fn update_status_if_pending_ok() {
        let db = init_mem_db().await;

        let mut quote = quotes::Quote {
            bill: quotes::BillInfo::default(),
            id: Uuid::new_v4(),
            submitted: TStamp::default(),
            status: quotes::Status::Pending {
                public_key: keys_test::publics()[0],
            },
        };
        let dbquote = QuoteDBEntry::from(quote.clone());
        let rid = RecordId::from_table_key(&db.table, quote.id);
        let _inserted: QuoteDBEntry = db.db.insert(rid).content(dbquote).await.unwrap().unwrap();

        quote.status = quotes::Status::Offered {
            keyset_id: keys_test::generate_random_keysetid(),
            ttl: TStamp::default(),
            discounted: quote.bill.sum,
        };
        let res = db.update_status_if_pending(quote.id, quote.status).await;
        assert!(res.is_ok());
    }

    #[tokio::test]
    async fn update_status_if_pending_ko() {
        let db = init_mem_db().await;

        let mut quote = quotes::Quote {
            bill: quotes::BillInfo::default(),
            id: Uuid::new_v4(),
            submitted: TStamp::default(),
            status: quotes::Status::Rejected {
                tstamp: TStamp::default(),
                discounted: bitcoin::Amount::default(),
            },
        };
        let dbquote = QuoteDBEntry::from(quote.clone());
        let rid = RecordId::from_table_key(&db.table, quote.id);
        let _inserted: QuoteDBEntry = db
            .db
            .insert(rid.clone())
            .content(dbquote)
            .await
            .unwrap()
            .unwrap();

        quote.status = quotes::Status::Offered {
            keyset_id: keys_test::generate_random_keysetid(),
            ttl: TStamp::default(),
            discounted: quote.bill.sum,
        };
        let res = db.update_status_if_pending(quote.id, quote.status).await;
        assert!(res.is_err());

        let content: Option<QuoteDBEntry> = db.db.select(rid).await.unwrap();
        assert!(content.is_some());
        let content = content.unwrap();
        assert!(matches!(content.status, quotes::Status::Rejected { .. }));
    }

    #[tokio::test]
    async fn update_status_if_offered_ok() {
        let db = init_mem_db().await;

        let mut quote = quotes::Quote {
            bill: quotes::BillInfo::default(),
            id: Uuid::new_v4(),
            submitted: TStamp::default(),
            status: quotes::Status::Offered {
                keyset_id: keys_test::generate_random_keysetid(),
                ttl: TStamp::default(),
                discounted: bitcoin::Amount::default(),
            },
        };
        let dbquote = QuoteDBEntry::from(quote.clone());
        let rid = RecordId::from_table_key(&db.table, quote.id);
        let _inserted: QuoteDBEntry = db.db.insert(rid).content(dbquote).await.unwrap().unwrap();

        quote.status = quotes::Status::Accepted {
            keyset_id: keys_test::generate_random_keysetid(),
            discounted: bitcoin::Amount::default(),
        };
        let res = db.update_status_if_offered(quote.id, quote.status).await;
        assert!(res.is_ok());
    }

    #[tokio::test]
    async fn update_status_if_offered_ko() {
        let db = init_mem_db().await;
        let now = TStamp::from_timestamp(10000, 0).unwrap();

        let mut quote = quotes::Quote {
            bill: quotes::BillInfo::default(),
            id: Uuid::new_v4(),
            submitted: TStamp::default(),
            status: quotes::Status::Denied { tstamp: now },
        };
        let dbquote = QuoteDBEntry::from(quote.clone());
        let rid = RecordId::from_table_key(&db.table, quote.id);
        let _inserted: QuoteDBEntry = db
            .db
            .insert(rid.clone())
            .content(dbquote)
            .await
            .unwrap()
            .unwrap();

        quote.status = quotes::Status::Offered {
            keyset_id: keys_test::generate_random_keysetid(),
            ttl: TStamp::default(),
            discounted: quote.bill.sum,
        };
        let res = db.update_status_if_offered(quote.id, quote.status).await;
        assert!(res.is_err());

        let content: Option<QuoteDBEntry> = db.db.select(rid).await.unwrap();
        assert!(content.is_some());
        let content = content.unwrap();
        assert!(matches!(content.status, quotes::Status::Denied { .. }));
    }

    #[tokio::test]
    async fn list_light_filter() {
        let db = init_mem_db().await;

        let qid = Uuid::new_v4();
        let rid = RecordId::from_table_key(&db.table, qid);
        let entry = QuoteDBEntry {
            qid,
            status: quotes::Status::Pending {
                public_key: keys_test::publics()[0],
            },
            bill: quotes::BillInfo {
                drawee: random_identity_public_data().1.into(),
                drawer: random_identity_public_data().1.into(),
                payee: BillParticipant::Ident(random_identity_public_data().1.into()),
                endorsees: vec![],
                maturity_date: TStamp::from_str("2021-01-01T00:00:00Z").unwrap(),
                ..Default::default()
            },
            submitted: TStamp::default(),
        };
        let _: QuoteDBEntry = db
            .db
            .insert(rid)
            .content(entry.clone())
            .await
            .unwrap()
            .unwrap();

        let filters = service::ListFilters::default();
        let res = db.list_light(filters, None).await.unwrap();
        assert_eq!(res.len(), 1);

        let date = chrono::NaiveDate::from_ymd_opt(2021, 1, 1);
        let filters = service::ListFilters {
            bill_maturity_date_from: date,
            ..Default::default()
        };
        let res = db.list_light(filters, None).await.unwrap();
        assert_eq!(res.len(), 1);

        let date = chrono::NaiveDate::from_ymd_opt(2022, 1, 1);
        let filters = service::ListFilters {
            bill_maturity_date_from: date,
            ..Default::default()
        };
        let res = db.list_light(filters, None).await.unwrap();
        assert_eq!(res.len(), 0);

        let filters = service::ListFilters {
            status: Some(quotes::StatusDiscriminants::Pending),
            bill_drawee_id: Some(random_identity_public_data().1.node_id),
            ..Default::default()
        };
        let res = db.list_light(filters, None).await.unwrap();
        assert_eq!(res.len(), 0);

        let filters = service::ListFilters {
            status: Some(quotes::StatusDiscriminants::Pending),
            bill_drawee_id: Some(entry.bill.drawee.node_id),
            ..Default::default()
        };
        let res = db.list_light(filters, None).await.unwrap();
        assert_eq!(res.len(), 1);
    }

    #[tokio::test]
    async fn list_light_sort() {
        let db = init_mem_db().await;

        let qid1 = Uuid::new_v4();
        let rid = RecordId::from_table_key(&db.table, qid1);
        let entry = QuoteDBEntry {
            qid: qid1,
            status: quotes::Status::Pending {
                public_key: keys_test::publics()[0],
            },
            bill: quotes::BillInfo {
                maturity_date: TStamp::from_str("2021-01-01T00:00:00Z").unwrap(),
                ..Default::default()
            },
            submitted: TStamp::default(),
        };
        let _: QuoteDBEntry = db.db.insert(rid).content(entry).await.unwrap().unwrap();

        let qid2 = Uuid::new_v4();
        let rid = RecordId::from_table_key(&db.table, qid2);
        let entry = QuoteDBEntry {
            qid: qid2,
            status: quotes::Status::Pending {
                public_key: keys_test::publics()[0],
            },
            bill: quotes::BillInfo {
                maturity_date: TStamp::from_str("2020-01-01T00:00:00Z").unwrap(),
                ..Default::default()
            },
            submitted: TStamp::default(),
        };
        let _: QuoteDBEntry = db.db.insert(rid).content(entry).await.unwrap().unwrap();

        let qid3 = Uuid::new_v4();
        let rid = RecordId::from_table_key(&db.table, qid3);
        let entry = QuoteDBEntry {
            qid: qid3,
            status: quotes::Status::Pending {
                public_key: keys_test::publics()[0],
            },
            bill: quotes::BillInfo {
                maturity_date: TStamp::from_str("2022-01-01T00:00:00Z").unwrap(),
                ..Default::default()
            },
            submitted: TStamp::default(),
        };
        let _: QuoteDBEntry = db.db.insert(rid).content(entry).await.unwrap().unwrap();

        let filters = service::ListFilters::default();
        let res = db
            .list_light(filters, Some(SortOrder::BillMaturityDateAsc))
            .await
            .unwrap();
        assert_eq!(res.len(), 3);
        assert_eq!(res[0].id, qid2);
        assert_eq!(res[1].id, qid1);
        assert_eq!(res[2].id, qid3);

        let filters = service::ListFilters::default();
        let res = db
            .list_light(filters, Some(SortOrder::BillMaturityDateDesc))
            .await
            .unwrap();
        assert_eq!(res.len(), 3);
        assert_eq!(res[0].id, qid3);
        assert_eq!(res[1].id, qid1);
        assert_eq!(res[2].id, qid2);
    }

    #[tokio::test]
    async fn search_by_bill() {
        let db = init_mem_db().await;

        let qid1 = Uuid::new_v4();
        let rid = RecordId::from_table_key(&db.table, qid1);
        let entry = QuoteDBEntry {
            qid: qid1,
            status: quotes::Status::Pending {
                public_key: keys_test::publics()[0],
            },
            bill: quotes::BillInfo {
                maturity_date: TStamp::from_str("2021-01-01T00:00:00Z").unwrap(),
                current_holder: BillParticipant::Ident(random_identity_public_data().1.into()),
                ..Default::default()
            },
            submitted: TStamp::default(),
        };
        let _: QuoteDBEntry = db
            .db
            .insert(rid)
            .content(entry.clone())
            .await
            .unwrap()
            .unwrap();

        let result = db
            .search_by_bill(&entry.bill.id, &entry.bill.current_holder.node_id())
            .await
            .unwrap();
        assert_eq!(result.len(), 1);
    }
}
