// ----- standard library imports
// ----- extra library imports
use anyhow::Result as AnyResult;
use async_trait::async_trait;
use surrealdb::Result as SurrealResult;
use surrealdb::{engine::any::Any, Surreal};
use uuid::Uuid;
// ----- local modules
// ----- local imports
use crate::quotes;
use crate::service::{ListFilters, Repository, SortOrder};
use crate::TStamp;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct DBEntryQuote {
    qid: surrealdb::Uuid, // can't be `id`, reserved word in surreal
    bill: quotes::BillInfo,
    submitted: TStamp,
    status: quotes::QuoteStatus,
}

impl From<DBEntryQuote> for quotes::Quote {
    fn from(dbq: DBEntryQuote) -> Self {
        Self {
            id: dbq.qid,
            bill: dbq.bill,
            submitted: dbq.submitted,
            status: dbq.status,
        }
    }
}

impl From<quotes::Quote> for DBEntryQuote {
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
struct DBEntryLightQuote {
    qid: uuid::Uuid,
    status: quotes::QuoteStatus,
    sum: bitcoin::Amount,
}
impl From<DBEntryLightQuote> for quotes::LightQuote {
    fn from(dbq: DBEntryLightQuote) -> Self {
        Self {
            id: dbq.qid,
            status: dbq.status.into(),
            sum: dbq.sum,
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

    async fn load(&self, qid: Uuid) -> SurrealResult<Option<DBEntryQuote>> {
        let rid = surrealdb::RecordId::from_table_key(&self.table, qid);
        self.db.select(rid).await
    }

    async fn store(&self, quote: DBEntryQuote) -> SurrealResult<Option<DBEntryQuote>> {
        let rid = surrealdb::RecordId::from_table_key(&self.table, quote.qid);
        self.db.insert(rid).content(quote).await
    }

    async fn light_list(
        &self,
        filters: ListFilters,
        sort: Option<SortOrder>,
    ) -> SurrealResult<Vec<DBEntryLightQuote>> {
        let mut statement = String::from(
            "SELECT qid, status, bill.sum AS sum, bill.maturity_date FROM type::table($table)",
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
                SortOrder::BillMaturityDateAsc => " ORDER BY bill.maturity_date ASC",
                SortOrder::BillMaturityDateDesc => " ORDER BY bill.maturity_date DESC",
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
        status: quotes::QuoteStatusDiscriminants,
        since: Option<TStamp>,
    ) -> SurrealResult<Vec<Uuid>> {
        let mut query = self
            .db
            .query(
                "SELECT * FROM type::table($table) WHERE status == $status ORDER BY submitted DESC",
            )
            .bind(("table", self.table.clone()))
            .bind(("status", status));
        if let Some(since) = since {
            query = query
                .query(" AND submitted >= $since")
                .bind(("since", since));
        }
        query.await?.take("quote_id")
    }

    async fn search_by_bill(&self, bill: &str, endorser: &str) -> SurrealResult<Vec<DBEntryQuote>> {
        let results: Vec<DBEntryQuote> = self.db
            .query("SELECT * FROM type::table($table) WHERE bill.id == $bill AND bill.current_holder.node_id == $endorser ORDER BY submitted DESC")
            .bind(("table", self.table.clone()))
            .bind(("bill", bill.to_owned()))
            .bind(("endorser", endorser.to_owned())).await?.take(0)?;
        Ok(results)
    }
}

#[async_trait]
impl Repository for DBQuotes {
    async fn load(&self, qid: uuid::Uuid) -> AnyResult<Option<quotes::Quote>> {
        let res = self.load(qid).await?.map(quotes::Quote::from);
        Ok(res)
    }

    async fn update_status_if_pending(
        &self,
        qid: uuid::Uuid,
        new: quotes::QuoteStatus,
    ) -> AnyResult<()> {
        let recordid = surrealdb::RecordId::from_table_key(&self.table, qid);
        let before: Option<DBEntryQuote> = self
            .db
            .query("UPDATE $rid SET status = $new WHERE status.status == $status RETURN BEFORE ")
            .bind(("rid", recordid))
            .bind(("new", new))
            .bind(("status", quotes::QuoteStatusDiscriminants::Pending))
            .await?
            .take(0)?;
        let before = before.ok_or(anyhow::anyhow!("Quote not found or not pending"))?;
        if !matches!(before.status, quotes::QuoteStatus::Pending { .. }) {
            return Err(anyhow::anyhow!("Quote not pending"));
        }
        Ok(())
    }

    async fn update_status_if_offered(
        &self,
        qid: uuid::Uuid,
        new: quotes::QuoteStatus,
    ) -> AnyResult<()> {
        let recordid = surrealdb::RecordId::from_table_key(&self.table, qid);
        let before: Option<DBEntryQuote> = self
            .db
            .query("UPDATE $rid SET status = $new WHERE status.status == $status RETURN BEFORE")
            .bind(("rid", recordid))
            .bind(("new", new))
            .bind(("status", quotes::QuoteStatusDiscriminants::Offered))
            .await?
            .take(0)?;
        if before.is_none() {
            return Err(anyhow::anyhow!("Quote not found or not pending"));
        }
        let before = before.unwrap();
        if !matches!(before.status, quotes::QuoteStatus::Offered { .. }) {
            return Err(anyhow::anyhow!("Quote not offered"));
        }
        Ok(())
    }

    async fn list_pendings(&self, since: Option<TStamp>) -> AnyResult<Vec<Uuid>> {
        self.list_by_status(quotes::QuoteStatusDiscriminants::Pending, since)
            .await
            .map_err(Into::into)
    }

    async fn list_light(
        &self,
        filters: ListFilters,
        sort: Option<SortOrder>,
    ) -> AnyResult<Vec<quotes::LightQuote>> {
        let db_result = self.light_list(filters, sort).await?;
        let response = db_result
            .into_iter()
            .map(std::convert::Into::into)
            .collect();
        Ok(response)
    }

    async fn search_by_bill(&self, bill: &str, endorser: &str) -> AnyResult<Vec<quotes::Quote>> {
        let res = self
            .search_by_bill(bill, endorser)
            .await?
            .into_iter()
            .map(quotes::Quote::from)
            .collect();
        Ok(res)
    }

    async fn store(&self, quote: quotes::Quote) -> AnyResult<()> {
        self.store(quote.into()).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service;
    use bcr_ebill_core::contact::IdentityPublicData;
    use bcr_wdc_utils::keys::test_utils as keys_test;
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

    #[tokio::test]
    async fn update_status_if_pending_ok() {
        let db = init_mem_db().await;

        let mut quote = quotes::Quote {
            bill: quotes::BillInfo::default(),
            id: Uuid::new_v4(),
            submitted: TStamp::default(),
            status: quotes::QuoteStatus::Pending {
                public_key: keys_test::publics()[0],
            },
        };
        let dbquote = DBEntryQuote::from(quote.clone());
        let rid = RecordId::from_table_key(&db.table, quote.id);
        let _inserted: DBEntryQuote = db.db.insert(rid).content(dbquote).await.unwrap().unwrap();

        quote.status = quotes::QuoteStatus::Offered {
            keyset_id: keys_test::generate_random_keysetid(),
            ttl: TStamp::default(),
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
            status: quotes::QuoteStatus::Rejected {
                tstamp: TStamp::default(),
            },
        };
        let dbquote = DBEntryQuote::from(quote.clone());
        let rid = RecordId::from_table_key(&db.table, quote.id);
        let _inserted: DBEntryQuote = db
            .db
            .insert(rid.clone())
            .content(dbquote)
            .await
            .unwrap()
            .unwrap();

        quote.status = quotes::QuoteStatus::Offered {
            keyset_id: keys_test::generate_random_keysetid(),
            ttl: TStamp::default(),
        };
        let res = db.update_status_if_pending(quote.id, quote.status).await;
        assert!(res.is_err());

        let content: Option<DBEntryQuote> = db.db.select(rid).await.unwrap();
        assert!(content.is_some());
        let content = content.unwrap();
        assert!(matches!(
            content.status,
            quotes::QuoteStatus::Rejected { .. }
        ));
    }

    #[tokio::test]
    async fn update_status_if_offered_ok() {
        let db = init_mem_db().await;

        let mut quote = quotes::Quote {
            bill: quotes::BillInfo::default(),
            id: Uuid::new_v4(),
            submitted: TStamp::default(),
            status: quotes::QuoteStatus::Offered {
                keyset_id: keys_test::generate_random_keysetid(),
                ttl: TStamp::default(),
            },
        };
        let dbquote = DBEntryQuote::from(quote.clone());
        let rid = RecordId::from_table_key(&db.table, quote.id);
        let _inserted: DBEntryQuote = db.db.insert(rid).content(dbquote).await.unwrap().unwrap();

        quote.status = quotes::QuoteStatus::Accepted {
            keyset_id: keys_test::generate_random_keysetid(),
        };
        let res = db.update_status_if_offered(quote.id, quote.status).await;
        assert!(res.is_ok());
    }

    #[tokio::test]
    async fn update_status_if_offered_ko() {
        let db = init_mem_db().await;

        let mut quote = quotes::Quote {
            bill: quotes::BillInfo::default(),
            id: Uuid::new_v4(),
            submitted: TStamp::default(),
            status: quotes::QuoteStatus::Denied,
        };
        let dbquote = DBEntryQuote::from(quote.clone());
        let rid = RecordId::from_table_key(&db.table, quote.id);
        let _inserted: DBEntryQuote = db
            .db
            .insert(rid.clone())
            .content(dbquote)
            .await
            .unwrap()
            .unwrap();

        quote.status = quotes::QuoteStatus::Offered {
            keyset_id: keys_test::generate_random_keysetid(),
            ttl: TStamp::default(),
        };
        let res = db.update_status_if_offered(quote.id, quote.status).await;
        assert!(res.is_err());

        let content: Option<DBEntryQuote> = db.db.select(rid).await.unwrap();
        assert!(content.is_some());
        let content = content.unwrap();
        assert!(matches!(content.status, quotes::QuoteStatus::Denied));
    }

    #[tokio::test]
    async fn list_light_filter() {
        let db = init_mem_db().await;

        let qid = Uuid::new_v4();
        let rid = RecordId::from_table_key(&db.table, qid);
        let entry = DBEntryQuote {
            qid,
            status: quotes::QuoteStatus::Pending {
                public_key: keys_test::publics()[0],
            },
            bill: quotes::BillInfo {
                drawee: IdentityPublicData {
                    node_id: String::from("drawee"),
                    ..Default::default()
                },
                drawer: IdentityPublicData {
                    node_id: String::from("drawer"),
                    ..Default::default()
                },
                payee: IdentityPublicData {
                    node_id: String::from("payer"),
                    ..Default::default()
                },
                endorsees: Default::default(),
                maturity_date: TStamp::from_str("2021-01-01T00:00:00Z").unwrap(),
                ..Default::default()
            },
            submitted: TStamp::default(),
        };
        let _: DBEntryQuote = db.db.insert(rid).content(entry).await.unwrap().unwrap();

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
            status: Some(quotes::QuoteStatusDiscriminants::Pending),
            bill_drawee_id: Some(String::from("none")),
            ..Default::default()
        };
        let res = db.list_light(filters, None).await.unwrap();
        assert_eq!(res.len(), 0);

        let filters = service::ListFilters {
            status: Some(quotes::QuoteStatusDiscriminants::Pending),
            bill_drawee_id: Some(String::from("drawee")),
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
        let entry = DBEntryQuote {
            qid: qid1,
            status: quotes::QuoteStatus::Pending {
                public_key: keys_test::publics()[0],
            },
            bill: quotes::BillInfo {
                maturity_date: TStamp::from_str("2021-01-01T00:00:00Z").unwrap(),
                ..Default::default()
            },
            submitted: TStamp::default(),
        };
        let _: DBEntryQuote = db.db.insert(rid).content(entry).await.unwrap().unwrap();

        let qid2 = Uuid::new_v4();
        let rid = RecordId::from_table_key(&db.table, qid2);
        let entry = DBEntryQuote {
            qid: qid2,
            status: quotes::QuoteStatus::Pending {
                public_key: keys_test::publics()[0],
            },
            bill: quotes::BillInfo {
                maturity_date: TStamp::from_str("2020-01-01T00:00:00Z").unwrap(),
                ..Default::default()
            },
            submitted: TStamp::default(),
        };
        let _: DBEntryQuote = db.db.insert(rid).content(entry).await.unwrap().unwrap();

        let qid3 = Uuid::new_v4();
        let rid = RecordId::from_table_key(&db.table, qid3);
        let entry = DBEntryQuote {
            qid: qid3,
            status: quotes::QuoteStatus::Pending {
                public_key: keys_test::publics()[0],
            },
            bill: quotes::BillInfo {
                maturity_date: TStamp::from_str("2022-01-01T00:00:00Z").unwrap(),
                ..Default::default()
            },
            submitted: TStamp::default(),
        };
        let _: DBEntryQuote = db.db.insert(rid).content(entry).await.unwrap().unwrap();

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
        let entry = DBEntryQuote {
            qid: qid1,
            status: quotes::QuoteStatus::Pending {
                public_key: keys_test::publics()[0],
            },
            bill: quotes::BillInfo {
                maturity_date: TStamp::from_str("2021-01-01T00:00:00Z").unwrap(),
                current_holder: IdentityPublicData {
                    node_id: String::from("endorser"),
                    ..Default::default()
                },
                ..Default::default()
            },
            submitted: TStamp::default(),
        };
        let _: DBEntryQuote = db
            .db
            .insert(rid)
            .content(entry.clone())
            .await
            .unwrap()
            .unwrap();

        let result = db
            .search_by_bill(&entry.bill.id, &entry.bill.current_holder.node_id)
            .await
            .unwrap();
        assert_eq!(result.len(), 1);
    }
}
