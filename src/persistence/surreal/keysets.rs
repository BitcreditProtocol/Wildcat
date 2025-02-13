use std::collections::BTreeMap;
// ----- standard library imports
use std::collections::HashMap;
// ----- extra library imports
use anyhow::Result as AnyResult;
use async_trait::async_trait;
use cdk::nuts::nut00 as cdk00;
use cdk::nuts::nut01 as cdk01;
use cdk::nuts::nut02 as cdk02;
use surrealdb::RecordId;
use surrealdb::Result as SurrealResult;
use surrealdb::{engine::any::Any, Surreal};
use uuid::Uuid;
// ----- local modules
// ----- local imports
use crate::credit::keys as creditkeys;
use crate::keys;
use crate::persistence::surreal::ConnectionConfig;

// ----- keys repository
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct DBKeys {
    info: cdk::mint::MintKeySetInfo,
    // unpacking MintKeySet because surrealdb doesn't support BTreeMap<K,V> where K is not a String
    unit: cdk00::CurrencyUnit,
    keys: HashMap<String, cdk01::MintKeyPair>,
}

impl From<keys::KeysetEntry> for DBKeys {
    fn from(ke: keys::KeysetEntry) -> Self {
        let (info, keyset) = ke;
        let mut serialized_keys = HashMap::new();
        let cdk02::MintKeySet { unit, mut keys, .. } = keyset;
        while let Some((amount, keypair)) = keys.pop_last() {
            // surrealDB does not accept map with keys of type anything but Strings
            // so we need to serialize the keys to strings...
            serialized_keys.insert(amount.to_string(), keypair);
        }
        DBKeys {
            info,
            unit,
            keys: serialized_keys,
        }
    }
}

impl From<DBKeys> for keys::KeysetEntry {
    fn from(dbk: DBKeys) -> Self {
        let DBKeys { info, unit, keys } = dbk;
        let mut keysmap: BTreeMap<cdk::Amount, cdk01::MintKeyPair> = BTreeMap::default();
        for (val, keypair) in keys {
            // ... and parse them back to the original type
            let uval = val.parse::<u64>().expect("Failed to parse amount");
            keysmap.insert(cdk::Amount::from(uval), keypair);
        }
        let keyset = cdk02::MintKeySet {
            id: info.id,
            unit,
            keys: cdk01::MintKeys::new(keysmap),
        };
        (info, keyset)
    }
}

#[derive(Debug, Clone)]
pub struct KeysDB {
    pub db: Surreal<surrealdb::engine::any::Any>,
}

impl KeysDB {
    const DB_TABLE: &'static str = "creditkeys";

    pub async fn new(cfg: ConnectionConfig) -> SurrealResult<Self> {
        let db_connection = Surreal::<Any>::init();
        db_connection.connect(cfg.connection).await?;
        db_connection.use_ns(cfg.namespace).await?;
        db_connection.use_db(cfg.database).await?;
        Ok(Self {
            db: db_connection,
        })
    }

    async fn store(&self, keys: keys::KeysetEntry) -> AnyResult<()> {
        let dbkeys = DBKeys::from(keys);
        let rid = RecordId::from_table_key(Self::DB_TABLE, dbkeys.info.id.to_string());
        let _resp: Option<DBKeys> = self.db.insert(rid).content(dbkeys).await?;
        Ok(())
    }

    async fn load(&self, kid: &keys::KeysetID) -> AnyResult<Option<keys::KeysetEntry>> {
        let rid = RecordId::from_table_key(Self::DB_TABLE, kid.to_string());
        let response: Option<DBKeys> = self.db.select(rid).await?;
        Ok(response.map(|dbk| dbk.into()))
    }
}

#[async_trait]
impl keys::Repository for KeysDB {
    async fn info(&self, kid: &keys::KeysetID) -> AnyResult<Option<cdk::mint::MintKeySetInfo>> {
        let rid = RecordId::from_table_key(Self::DB_TABLE, kid.to_string());
        let result: Option<cdk::mint::MintKeySetInfo> = self
            .db
            .query("SELECT info FROM $rid")
            .bind(("rid", rid))
            .await?
            .take(0)?;
        Ok(result)
    }

    async fn keyset(&self, kid: &keys::KeysetID) -> AnyResult<Option<cdk02::MintKeySet>> {
        self.load(kid)
            .await
            .map(|res| res.map(|(_, keyset)| keyset))
    }

    async fn load(&self, kid: &keys::KeysetID) -> AnyResult<Option<keys::KeysetEntry>> {
        self.load(kid).await
    }

    async fn store(
        &self,
        keyset: cdk02::MintKeySet,
        info: cdk::mint::MintKeySetInfo,
    ) -> AnyResult<()> {
        self.store((info, keyset)).await
    }
}

// ----- quote-based keys repository
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct DBQuoteKeys {
    qid: surrealdb::Uuid,
    keys: DBKeys,
}

#[derive(Debug, Clone)]
pub struct QuoteKeysDB {
    pub db: Surreal<surrealdb::engine::any::Any>,
}

impl QuoteKeysDB {
    const DB_TABLE: &'static str = "quotekeys";

    pub async fn new(cfg: ConnectionConfig) -> SurrealResult<Self> {
        let db_connection = Surreal::<Any>::init();
        db_connection.connect(cfg.connection).await?;
        db_connection.use_ns(cfg.namespace).await?;
        db_connection.use_db(cfg.database).await?;
        Ok(Self { db: db_connection })
    }
}

#[async_trait]
impl creditkeys::QuoteBasedRepository for QuoteKeysDB {
    async fn load(&self, _kid: &keys::KeysetID, qid: Uuid) -> AnyResult<Option<keys::KeysetEntry>> {
        let res: Option<DBQuoteKeys> = self.db.select((Self::DB_TABLE, qid)).await?;
        Ok(res.map(|dbqk| dbqk.keys.into()))
    }

    async fn store(
        &self,
        qid: Uuid,
        keyset: cdk02::MintKeySet,
        info: cdk::mint::MintKeySetInfo,
    ) -> AnyResult<()> {
        let dbqk = DBQuoteKeys{qid, keys: DBKeys::from((info, keyset))};
        let _: Option<DBQuoteKeys> = self.db.insert((Self::DB_TABLE, qid)).content(dbqk).await?;
        Ok(())
    }
}
