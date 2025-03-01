// ----- standard library imports
// ----- extra library imports
use anyhow::Result as AnyResult;
use async_trait::async_trait;
use bcr_wdc_keys as keys;
use bcr_wdc_keys::persistence::{DBKeys, KeysetEntry};
use cashu::mint as cdk_mint;
use cashu::nuts::nut02 as cdk02;
use surrealdb::{engine::any::Any, Result as SurrealResult, Surreal};
use uuid::Uuid;
// ----- local modules
// ----- local imports
use crate::keys_factory;
use crate::persistence::surreal::ConnectionConfig;

// ----- quote-based keys repository
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct DBQuoteKeys {
    qid: surrealdb::Uuid,
    data: DBKeys,
}

#[derive(Debug, Clone)]
pub struct QuoteKeysDB {
    db: Surreal<surrealdb::engine::any::Any>,
    table: String,
}

impl QuoteKeysDB {
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
}

#[async_trait]
impl keys_factory::QuoteBasedRepository for QuoteKeysDB {
    async fn load(&self, _kid: &keys::KeysetID, qid: Uuid) -> AnyResult<Option<KeysetEntry>> {
        let res: Option<DBQuoteKeys> = self.db.select((self.table.clone(), qid)).await?;
        Ok(res.map(|dbqk| dbqk.data.into()))
    }

    async fn store(
        &self,
        qid: Uuid,
        keyset: cdk02::MintKeySet,
        info: cdk_mint::MintKeySetInfo,
    ) -> AnyResult<()> {
        let dbqk = DBQuoteKeys {
            qid,
            data: DBKeys::from((info, keyset)),
        };
        let _: Option<DBQuoteKeys> = self
            .db
            .insert((self.table.clone(), qid))
            .content(dbqk)
            .await?;
        Ok(())
    }
}
