// ----- standard library imports
// ----- extra library imports
use anyhow::{anyhow, Error as AnyError, Result as AnyResult};
use async_trait::async_trait;
use cashu::nuts::nut00 as cdk00;
use surrealdb::Result as SurrealResult;
use surrealdb::{engine::any::Any, Surreal};
use uuid::Uuid;
// ----- local modules
// ----- local imports
use crate::quotes;
use crate::service::{ListFilters, Repository, SortOrder};
use crate::TStamp;

#[derive(Debug, Default, Clone, serde::Serialize, serde::Deserialize, strum::Display)]
enum DBEntryQuoteStatus {
    #[default]
    Pending,
    Denied,
    Offered,
    Rejected,
    Accepted,
}
impl From<&quotes::QuoteStatus> for DBEntryQuoteStatus {
    fn from(value: &quotes::QuoteStatus) -> Self {
        match value {
            quotes::QuoteStatus::Pending { .. } => Self::Pending,
            quotes::QuoteStatus::Denied => Self::Denied,
            quotes::QuoteStatus::Offered { .. } => Self::Offered,
            quotes::QuoteStatus::Rejected { .. } => Self::Rejected,
            quotes::QuoteStatus::Accepted { .. } => Self::Accepted,
        }
    }
}
impl From<DBEntryQuoteStatus> for quotes::QuoteStatusDiscriminants {
    fn from(value: DBEntryQuoteStatus) -> Self {
        match value {
            DBEntryQuoteStatus::Pending => Self::Pending,
            DBEntryQuoteStatus::Denied => Self::Denied,
            DBEntryQuoteStatus::Offered => Self::Offered,
            DBEntryQuoteStatus::Rejected => Self::Rejected,
            DBEntryQuoteStatus::Accepted => Self::Accepted,
        }
    }
}
impl From<quotes::QuoteStatusDiscriminants> for DBEntryQuoteStatus {
    fn from(value: quotes::QuoteStatusDiscriminants) -> Self {
        match value {
            quotes::QuoteStatusDiscriminants::Pending => Self::Pending,
            quotes::QuoteStatusDiscriminants::Denied => Self::Denied,
            quotes::QuoteStatusDiscriminants::Offered => Self::Offered,
            quotes::QuoteStatusDiscriminants::Rejected => Self::Rejected,
            quotes::QuoteStatusDiscriminants::Accepted => Self::Accepted,
        }
    }
}

#[derive(Debug, Default, Clone, serde::Serialize, serde::Deserialize)]
struct DBEntryQuote {
    qid: surrealdb::Uuid, // can't be `id`, reserved world in surreal
    bill: quotes::BillInfo,
    submitted: TStamp,
    status: DBEntryQuoteStatus,
    blinds: Option<Vec<cdk00::BlindedMessage>>,
    signatures: Option<Vec<cdk00::BlindSignature>>,
    ttl: Option<TStamp>,
    rejection: Option<TStamp>,
}

impl From<quotes::Quote> for DBEntryQuote {
    fn from(q: quotes::Quote) -> Self {
        match q.status {
            quotes::QuoteStatus::Pending { blinds } => Self {
                qid: q.id,
                bill: q.bill,
                submitted: q.submitted,
                status: DBEntryQuoteStatus::Pending,
                blinds: Some(blinds),
                signatures: Default::default(),
                ttl: Default::default(),
                rejection: Default::default(),
            },
            quotes::QuoteStatus::Denied => Self {
                qid: q.id,
                bill: q.bill,
                submitted: q.submitted,
                status: DBEntryQuoteStatus::Denied,
                blinds: Default::default(),
                signatures: Default::default(),
                ttl: Default::default(),
                rejection: Default::default(),
            },
            quotes::QuoteStatus::Offered { signatures, ttl } => Self {
                qid: q.id,
                bill: q.bill,
                submitted: q.submitted,
                status: DBEntryQuoteStatus::Accepted,
                signatures: Some(signatures),
                ttl: Some(ttl),
                blinds: Default::default(),
                rejection: Default::default(),
            },
            quotes::QuoteStatus::Rejected { tstamp } => Self {
                qid: q.id,
                bill: q.bill,
                submitted: q.submitted,
                status: DBEntryQuoteStatus::Rejected,
                rejection: Some(tstamp),
                blinds: Default::default(),
                signatures: Default::default(),
                ttl: Default::default(),
            },
            quotes::QuoteStatus::Accepted { signatures } => Self {
                qid: q.id,
                bill: q.bill,
                submitted: q.submitted,
                status: DBEntryQuoteStatus::Accepted,
                signatures: Some(signatures),
                blinds: Default::default(),
                ttl: Default::default(),
                rejection: Default::default(),
            },
        }
    }
}

impl TryFrom<DBEntryQuote> for quotes::Quote {
    type Error = AnyError;
    fn try_from(dbq: DBEntryQuote) -> Result<Self, Self::Error> {
        match dbq.status {
            DBEntryQuoteStatus::Pending => Ok(Self {
                id: dbq.qid,
                bill: dbq.bill,
                submitted: dbq.submitted,
                status: quotes::QuoteStatus::Pending {
                    blinds: dbq.blinds.ok_or_else(|| anyhow!("missing blinds"))?,
                },
            }),
            DBEntryQuoteStatus::Denied => Ok(Self {
                id: dbq.qid,
                bill: dbq.bill,
                submitted: dbq.submitted,
                status: quotes::QuoteStatus::Denied,
            }),
            DBEntryQuoteStatus::Offered => Ok(Self {
                id: dbq.qid,
                bill: dbq.bill,
                submitted: dbq.submitted,
                status: quotes::QuoteStatus::Offered {
                    signatures: dbq
                        .signatures
                        .ok_or_else(|| anyhow!("missing signatures"))?,
                    ttl: dbq.ttl.ok_or_else(|| anyhow!("missing ttl"))?,
                },
            }),
            DBEntryQuoteStatus::Rejected => Ok(Self {
                id: dbq.qid,
                bill: dbq.bill,
                submitted: dbq.submitted,
                status: quotes::QuoteStatus::Rejected {
                    tstamp: dbq.rejection.ok_or_else(|| anyhow!("missing rejection"))?,
                },
            }),
            DBEntryQuoteStatus::Accepted => Ok(Self {
                id: dbq.qid,
                bill: dbq.bill,
                submitted: dbq.submitted,
                status: quotes::QuoteStatus::Accepted {
                    signatures: dbq
                        .signatures
                        .ok_or_else(|| anyhow!("missing signatures"))?,
                },
            }),
        }
    }
}

#[derive(Debug, Clone, serde::Deserialize)]
struct DBEntryLightQuote {
    qid: uuid::Uuid,
    status: DBEntryQuoteStatus,
}
impl From<DBEntryLightQuote> for quotes::LightQuote {
    fn from(dbq: DBEntryLightQuote) -> Self {
        Self {
            id: dbq.qid,
            status: dbq.status.into(),
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
        self.db.select((&self.table, qid)).await
    }

    async fn store(&self, quote: DBEntryQuote) -> SurrealResult<Option<DBEntryQuote>> {
        self.db
            .insert((&self.table, quote.qid))
            .content(quote)
            .await
    }

    async fn light_list(
        &self,
        filters: ListFilters,
        sort: Option<SortOrder>,
    ) -> SurrealResult<Vec<DBEntryLightQuote>> {
        let mut statement =
            String::from("SELECT qid, status, bill.maturity_date FROM type::table($table)");

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
        let status = filters.status.map(DBEntryQuoteStatus::from);
        add_filter_statement!(statement, first, status, "status == $status");
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
        status: DBEntryQuoteStatus,
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
            .query("SELECT * FROM type::table($table) WHERE bill == $bill AND endorser == $endorser ORDER BY submitted DESC")
            .bind(("table", self.table.clone()))
            .bind(("bill", bill.to_owned()))
            .bind(("endorser", endorser.to_owned())).await?.take(0)?;
        Ok(results)
    }
}

#[async_trait]
impl Repository for DBQuotes {
    async fn load(&self, qid: uuid::Uuid) -> AnyResult<Option<quotes::Quote>> {
        self.load(qid)
            .await?
            .map(std::convert::TryInto::try_into)
            .transpose()
    }

    async fn update_if_pending(&self, new: quotes::Quote) -> AnyResult<()> {
        if matches!(new.status, quotes::QuoteStatus::Pending { .. }) {
            return Err(anyhow!("cannot update to pending"));
        }
        let recordid = surrealdb::RecordId::from_table_key(&self.table, new.id);
        self.db
            .query("UPDATE $rid CONTENT $new WHERE status == $status")
            .bind(("rid", recordid))
            .bind(("new", DBEntryQuote::from(new)))
            .bind(("status", DBEntryQuoteStatus::Pending))
            .await?;
        Ok(())
    }

    async fn update_if_offered(&self, new: quotes::Quote) -> AnyResult<()> {
        if matches!(new.status, quotes::QuoteStatus::Pending { .. }) {
            return Err(anyhow!("cannot update to pending"));
        }
        let recordid = surrealdb::RecordId::from_table_key(&self.table, new.id);
        self.db
            .query("UPDATE $rid CONTENT $new WHERE status == $status")
            .bind(("rid", recordid))
            .bind(("new", DBEntryQuote::from(new)))
            .bind(("status", DBEntryQuoteStatus::Offered))
            .await?;
        Ok(())
    }

    async fn list_pendings(&self, since: Option<TStamp>) -> AnyResult<Vec<Uuid>> {
        self.list_by_status(DBEntryQuoteStatus::Pending, since)
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
        self.search_by_bill(bill, endorser)
            .await?
            .into_iter()
            .map(std::convert::TryInto::try_into)
            .collect()
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
    async fn list_light_filter() {
        let db = init_mem_db().await;

        let qid = Uuid::new_v4();
        let rid = RecordId::from_table_key(&db.table, qid);
        let entry = DBEntryQuote {
            qid,
            status: DBEntryQuoteStatus::Pending,
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
            ..Default::default()
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
            status: DBEntryQuoteStatus::Pending,
            bill: quotes::BillInfo {
                maturity_date: TStamp::from_str("2021-01-01T00:00:00Z").unwrap(),
                ..Default::default()
            },
            ..Default::default()
        };
        let _: DBEntryQuote = db.db.insert(rid).content(entry).await.unwrap().unwrap();

        let qid2 = Uuid::new_v4();
        let rid = RecordId::from_table_key(&db.table, qid2);
        let entry = DBEntryQuote {
            qid: qid2,
            status: DBEntryQuoteStatus::Pending,
            bill: quotes::BillInfo {
                maturity_date: TStamp::from_str("2020-01-01T00:00:00Z").unwrap(),
                ..Default::default()
            },
            ..Default::default()
        };
        let _: DBEntryQuote = db.db.insert(rid).content(entry).await.unwrap().unwrap();

        let qid3 = Uuid::new_v4();
        let rid = RecordId::from_table_key(&db.table, qid3);
        let entry = DBEntryQuote {
            qid: qid3,
            status: DBEntryQuoteStatus::Pending,
            bill: quotes::BillInfo {
                maturity_date: TStamp::from_str("2022-01-01T00:00:00Z").unwrap(),
                ..Default::default()
            },
            ..Default::default()
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
}
