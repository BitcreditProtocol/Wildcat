// ----- standard library imports
use std::collections::{BTreeMap, HashMap};
// ----- extra library imports
use anyhow::anyhow;
use async_trait::async_trait;
use bcr_wdc_utils::keys::KeysetEntry;
use cashu::nuts::nut00 as cdk00;
use cashu::nuts::nut01 as cdk01;
use cashu::nuts::nut02 as cdk02;
use cashu::Amount;
use cdk_common::mint as cdk_mint;
use cdk_common::mint::MintKeySetInfo;
use surrealdb::{engine::any::Any, RecordId, Result as SurrealResult, Surreal};
// ----- local imports
use crate::error::{Error, Result};
use crate::service::{KeysRepository, QuoteKeysRepository};

// ----- end imports

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct KeysDBEntry {
    info: cdk_mint::MintKeySetInfo,
    // unpacking MintKeySet because surrealdb doesn't support BTreeMap<K,V> where K is not a String
    unit: cdk00::CurrencyUnit,
    // surrealdb supports only strings as key type
    keys: HashMap<String, cdk01::MintKeyPair>,
}

impl From<KeysetEntry> for KeysDBEntry {
    fn from(ke: KeysetEntry) -> Self {
        let (info, keyset) = ke;
        let mut serialized_keys = HashMap::new();
        let cdk02::MintKeySet { unit, mut keys, .. } = keyset;
        while let Some((amount, keypair)) = keys.pop_last() {
            // surrealDB does not accept map with keys of type anything but Strings
            // so we need to serialize the keys to strings...
            serialized_keys.insert(amount.to_string(), keypair);
        }
        Self {
            info,
            unit,
            keys: serialized_keys,
        }
    }
}

impl From<KeysDBEntry> for KeysetEntry {
    fn from(dbk: KeysDBEntry) -> Self {
        let KeysDBEntry {
            info, unit, keys, ..
        } = dbk;
        let mut keysmap: BTreeMap<Amount, cdk01::MintKeyPair> = BTreeMap::default();
        for (val, keypair) in keys.into_iter() {
            // ... and parse them back to the original type
            let uval = val.parse::<u64>().expect("Failed to parse amount");
            keysmap.insert(Amount::from(uval), keypair);
        }
        let keyset = cdk02::MintKeySet {
            id: info.id,
            unit,
            keys: cdk01::MintKeys::new(keysmap),
        };
        (info, keyset)
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
struct DB {
    db: Surreal<surrealdb::engine::any::Any>,
    table: String,
}

impl DB {
    async fn new(cfg: ConnectionConfig) -> SurrealResult<Self> {
        let db_connection = Surreal::<Any>::init();
        db_connection.connect(cfg.connection).await?;
        db_connection.use_ns(cfg.namespace).await?;
        db_connection.use_db(cfg.database).await?;
        Ok(Self {
            db: db_connection,
            table: cfg.table,
        })
    }

    async fn store(&self, rid: RecordId, keys: KeysetEntry) -> SurrealResult<()> {
        let dbkeys = KeysDBEntry::from(keys);
        let _resp: Option<KeysDBEntry> = self.db.insert(rid).content(dbkeys).await?;
        Ok(())
    }

    async fn info(&self, rid: RecordId) -> SurrealResult<Option<cdk_mint::MintKeySetInfo>> {
        // more efficient than load and then extract info
        let result: Option<cdk_mint::MintKeySetInfo> = self
            .db
            .query("SELECT info FROM $rid")
            .bind(("rid", rid))
            .await?
            .take(0)?;
        Ok(result)
    }

    async fn list_info(&self) -> SurrealResult<Vec<cdk_mint::MintKeySetInfo>> {
        let response: Vec<KeysDBEntry> = self
            .db
            .query("SELECT * FROM type::table($table)")
            .bind(("table", self.table.clone()))
            .await?
            .take(0)?;
        let sets = response
            .into_iter()
            .map(KeysetEntry::from)
            .map(|(info, _)| info)
            .collect();
        Ok(sets)
    }

    async fn keyset(&self, rid: RecordId) -> SurrealResult<Option<cdk02::MintKeySet>> {
        let response: Option<KeysDBEntry> = self.db.select(rid).await?;
        let Some(keysdbentry) = response else {
            return Ok(None);
        };
        let (_, keyset) = KeysetEntry::from(keysdbentry);
        Ok(Some(keyset))
    }

    async fn list_keyset(&self) -> SurrealResult<Vec<cdk02::MintKeySet>> {
        let response: Vec<KeysDBEntry> = self
            .db
            .query("SELECT * FROM type::table($table)")
            .await?
            .take(0)?;
        let sets = response
            .into_iter()
            .map(KeysetEntry::from)
            .map(|(_, keyset)| keyset)
            .collect();
        Ok(sets)
    }
}

#[derive(Debug, Clone)]
pub struct DBKeys(DB);

impl DBKeys {
    pub async fn new(cfg: ConnectionConfig) -> SurrealResult<Self> {
        Ok(Self(DB::new(cfg).await?))
    }
}

#[async_trait]
impl KeysRepository for DBKeys {
    async fn info(&self, kid: &cdk02::Id) -> Result<Option<MintKeySetInfo>> {
        let rid = RecordId::from_table_key(self.0.table.clone(), kid.to_string());
        self.0
            .info(rid)
            .await
            .map_err(|e| Error::KeysRepository(anyhow!(e)))
    }
    async fn list_info(&self) -> Result<Vec<cdk_mint::MintKeySetInfo>> {
        self.0
            .list_info()
            .await
            .map_err(|e| Error::KeysRepository(anyhow!(e)))
    }
    async fn keyset(&self, kid: &cdk02::Id) -> Result<Option<cdk02::MintKeySet>> {
        let rid = RecordId::from_table_key(self.0.table.clone(), kid.to_string());
        self.0
            .keyset(rid)
            .await
            .map_err(|e| Error::KeysRepository(anyhow!(e)))
    }

    async fn list_keyset(&self) -> Result<Vec<cdk02::MintKeySet>> {
        self.0
            .list_keyset()
            .await
            .map_err(|e| Error::KeysRepository(anyhow!(e)))
    }

    async fn store(&self, entry: KeysetEntry) -> Result<()> {
        let rid = RecordId::from_table_key(self.0.table.clone(), entry.0.id.to_string());
        self.0
            .store(rid, entry)
            .await
            .map_err(|e| Error::KeysRepository(anyhow!(e)))
    }
}

#[derive(Debug, Clone)]
pub struct DBQuoteKeys(DB);

impl DBQuoteKeys {
    pub async fn new(cfg: ConnectionConfig) -> SurrealResult<Self> {
        Ok(Self(DB::new(cfg).await?))
    }
}

#[async_trait]
impl QuoteKeysRepository for DBQuoteKeys {
    async fn info(&self, kid: &cdk02::Id, qid: &uuid::Uuid) -> Result<Option<MintKeySetInfo>> {
        let record_key = format!("{}-{}", kid, qid);
        let rid = RecordId::from_table_key(self.0.table.clone(), record_key);
        self.0
            .info(rid)
            .await
            .map_err(|e| Error::KeysRepository(anyhow!(e)))
    }

    async fn keyset(&self, kid: &cdk02::Id, qid: &uuid::Uuid) -> Result<Option<cdk02::MintKeySet>> {
        let record_key = format!("{}-{}", kid, qid);
        let rid = RecordId::from_table_key(self.0.table.clone(), record_key);
        self.0
            .keyset(rid)
            .await
            .map_err(|e| Error::KeysRepository(anyhow!(e)))
    }

    async fn store(&self, qid: &uuid::Uuid, entry: KeysetEntry) -> Result<()> {
        let record_key = format!("{}-{}", entry.0.id, qid);
        let rid = RecordId::from_table_key(self.0.table.clone(), record_key);
        self.0
            .store(rid, entry)
            .await
            .map_err(|e| Error::KeysRepository(anyhow!(e)))
    }
}

#[cfg(test)]
mod tests {

    use super::*;
    use bcr_wdc_utils::keys::test_utils as keys_test;

    async fn init_mem_db() -> DB {
        let sdb = Surreal::<Any>::init();
        sdb.connect("mem://").await.unwrap();
        sdb.use_ns("test").await.unwrap();
        sdb.use_db("test").await.unwrap();
        DB {
            db: sdb,
            table: String::from("test"),
        }
    }

    #[tokio::test]
    async fn list_info() {
        let db = init_mem_db().await;
        let (info, keyset) = keys_test::generate_keyset();
        let rid = RecordId::from_table_key(db.table.clone(), info.id.to_string());
        let entry = KeysDBEntry::from((info.clone(), keyset));
        let _: KeysDBEntry = db.db.insert(rid).content(entry).await.unwrap().unwrap();
        let infos = db.list_info().await.unwrap();
        assert_eq!(infos.len(), 1);
    }
}
