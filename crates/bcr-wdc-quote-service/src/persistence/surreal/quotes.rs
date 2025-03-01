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
use crate::persistence::surreal::ConnectionConfig;
use crate::quotes;
use crate::TStamp;

#[derive(Debug, Default, Clone, serde::Serialize, serde::Deserialize, strum::Display)]
enum DBQuoteStatus {
    #[default]
    Pending,
    Denied,
    Offered,
    Rejected,
    Accepted,
}
impl From<&quotes::QuoteStatus> for DBQuoteStatus {
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

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct DBQuote {
    quote_id: surrealdb::Uuid, // can't be `id`, reserved world in surreal
    bill: quotes::BillInfo,
    submitted: TStamp,
    status: DBQuoteStatus,
    blinds: Option<Vec<cdk00::BlindedMessage>>,
    signatures: Option<Vec<cdk00::BlindSignature>>,
    ttl: Option<TStamp>,
    rejection: Option<TStamp>,
}

impl From<quotes::Quote> for DBQuote {
    fn from(q: quotes::Quote) -> Self {
        match q.status {
            quotes::QuoteStatus::Pending { blinds } => Self {
                quote_id: q.id,
                bill: q.bill,
                submitted: q.submitted,
                status: DBQuoteStatus::Pending,
                blinds: Some(blinds),
                signatures: Default::default(),
                ttl: Default::default(),
                rejection: Default::default(),
            },
            quotes::QuoteStatus::Denied => Self {
                quote_id: q.id,
                bill: q.bill,
                submitted: q.submitted,
                status: DBQuoteStatus::Denied,
                blinds: Default::default(),
                signatures: Default::default(),
                ttl: Default::default(),
                rejection: Default::default(),
            },
            quotes::QuoteStatus::Offered { signatures, ttl } => Self {
                quote_id: q.id,
                bill: q.bill,
                submitted: q.submitted,
                status: DBQuoteStatus::Accepted,
                signatures: Some(signatures),
                ttl: Some(ttl),
                blinds: Default::default(),
                rejection: Default::default(),
            },
            quotes::QuoteStatus::Rejected { tstamp } => Self {
                quote_id: q.id,
                bill: q.bill,
                submitted: q.submitted,
                status: DBQuoteStatus::Rejected,
                rejection: Some(tstamp),
                blinds: Default::default(),
                signatures: Default::default(),
                ttl: Default::default(),
            },
            quotes::QuoteStatus::Accepted { signatures } => Self {
                quote_id: q.id,
                bill: q.bill,
                submitted: q.submitted,
                status: DBQuoteStatus::Accepted,
                signatures: Some(signatures),
                blinds: Default::default(),
                ttl: Default::default(),
                rejection: Default::default(),
            },
        }
    }
}

impl TryFrom<DBQuote> for quotes::Quote {
    type Error = AnyError;
    fn try_from(dbq: DBQuote) -> Result<Self, Self::Error> {
        match dbq.status {
            DBQuoteStatus::Pending => Ok(Self {
                id: dbq.quote_id,
                bill: dbq.bill,
                submitted: dbq.submitted,
                status: quotes::QuoteStatus::Pending {
                    blinds: dbq.blinds.ok_or_else(|| anyhow!("missing blinds"))?,
                },
            }),
            DBQuoteStatus::Denied => Ok(Self {
                id: dbq.quote_id,
                bill: dbq.bill,
                submitted: dbq.submitted,
                status: quotes::QuoteStatus::Denied,
            }),
            DBQuoteStatus::Offered => Ok(Self {
                id: dbq.quote_id,
                bill: dbq.bill,
                submitted: dbq.submitted,
                status: quotes::QuoteStatus::Offered {
                    signatures: dbq
                        .signatures
                        .ok_or_else(|| anyhow!("missing signatures"))?,
                    ttl: dbq.ttl.ok_or_else(|| anyhow!("missing ttl"))?,
                },
            }),
            DBQuoteStatus::Rejected => Ok(Self {
                id: dbq.quote_id,
                bill: dbq.bill,
                submitted: dbq.submitted,
                status: quotes::QuoteStatus::Rejected {
                    tstamp: dbq.rejection.ok_or_else(|| anyhow!("missing rejection"))?,
                },
            }),
            DBQuoteStatus::Accepted => Ok(Self {
                id: dbq.quote_id,
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

#[derive(Debug, Clone)]
pub struct DB {
    db: Surreal<surrealdb::engine::any::Any>,
    table: String,
}

impl DB {
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

    async fn load(&self, qid: Uuid) -> SurrealResult<Option<DBQuote>> {
        self.db.select((&self.table, qid)).await
    }

    async fn store(&self, quote: DBQuote) -> SurrealResult<Option<DBQuote>> {
        self.db
            .insert((&self.table, quote.quote_id))
            .content(quote)
            .await
    }

    async fn list_by_status(
        &self,
        status: DBQuoteStatus,
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

    async fn search_by_bill(&self, bill: &str, endorser: &str) -> SurrealResult<Vec<DBQuote>> {
        let results: Vec<DBQuote> = self.db
            .query("SELECT * FROM type::table($table) WHERE bill == $bill AND endorser == $endorser ORDER BY submitted DESC")
            .bind(("table", self.table.clone()))
            .bind(("bill", bill.to_owned()))
            .bind(("endorser", endorser.to_owned())).await?.take(0)?;
        Ok(results)
    }
}

#[async_trait]
impl quotes::Repository for DB {
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
            .bind(("new", DBQuote::from(new)))
            .bind(("status", DBQuoteStatus::Pending))
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
            .bind(("new", DBQuote::from(new)))
            .bind(("status", DBQuoteStatus::Offered))
            .await?;
        Ok(())
    }

    async fn list_pendings(&self, since: Option<TStamp>) -> AnyResult<Vec<Uuid>> {
        self.list_by_status(DBQuoteStatus::Pending, since)
            .await
            .map_err(Into::into)
    }

    async fn list_offers(&self, since: Option<TStamp>) -> AnyResult<Vec<Uuid>> {
        self.list_by_status(DBQuoteStatus::Accepted, since)
            .await
            .map_err(Into::into)
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
