// ----- standard library imports
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, RwLock};
// ----- extra library imports
use anyhow::Result as AnyResult;
use cashu::nuts::nut00 as cdk00;
use cashu::nuts::nut01 as cdk01;
use cashu::nuts::nut02 as cdk02;
use cashu::Amount as cdk_Amount;
use cdk_common::mint as cdk_mint;
use surrealdb::{engine::any::Any, RecordId, Result as SurrealResult, Surreal};
// ----- local imports
use crate::id::KeysetID;
use crate::KeysetEntry;

#[derive(Default, Clone)]
pub struct InMemoryMap {
    keys: Arc<RwLock<HashMap<KeysetID, KeysetEntry>>>,
}

impl InMemoryMap {
    pub async fn info(&self, kid: &KeysetID) -> AnyResult<Option<cdk_mint::MintKeySetInfo>> {
        let a = self
            .keys
            .read()
            .unwrap()
            .get(kid)
            .map(|(info, _)| info.clone());
        Ok(a)
    }
    pub async fn keyset(&self, kid: &KeysetID) -> AnyResult<Option<cdk02::MintKeySet>> {
        let a = self
            .keys
            .read()
            .unwrap()
            .get(kid)
            .map(|(_, keyset)| keyset.clone());
        Ok(a)
    }
    pub async fn load(&self, kid: &KeysetID) -> AnyResult<Option<KeysetEntry>> {
        let a = self.keys.read().unwrap().get(kid).cloned();
        Ok(a)
    }
    pub async fn store(
        &self,
        keyset: cdk02::MintKeySet,
        info: cdk_mint::MintKeySetInfo,
    ) -> AnyResult<()> {
        self.keys
            .write()
            .unwrap()
            .insert(KeysetID::from(keyset.id), (info, keyset));
        Ok(())
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DBKeys {
    info: cdk_mint::MintKeySetInfo,
    // unpacking MintKeySet because surrealdb doesn't support BTreeMap<K,V> where K is not a String
    unit: cdk00::CurrencyUnit,
    keys: HashMap<String, cdk01::MintKeyPair>,
}

impl From<KeysetEntry> for DBKeys {
    fn from(ke: KeysetEntry) -> Self {
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

impl From<DBKeys> for KeysetEntry {
    fn from(dbk: DBKeys) -> Self {
        let DBKeys { info, unit, keys } = dbk;
        let mut keysmap: BTreeMap<cdk_Amount, cdk01::MintKeyPair> = BTreeMap::default();
        for (val, keypair) in keys {
            // ... and parse them back to the original type
            let uval = val.parse::<u64>().expect("Failed to parse amount");
            keysmap.insert(cdk_Amount::from(uval), keypair);
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
    db: Surreal<surrealdb::engine::any::Any>,
    table: String,
}

impl KeysDB {
    pub async fn new(conn: &str, ns: &str, db: &str, table: &str) -> SurrealResult<Self> {
        let db_connection = Surreal::<Any>::init();
        db_connection.connect(conn).await?;
        db_connection.use_ns(ns).await?;
        db_connection.use_db(db).await?;
        Ok(Self {
            db: db_connection,
            table: table.to_string(),
        })
    }

    pub async fn store(&self, keys: KeysetEntry) -> SurrealResult<()> {
        let dbkeys = DBKeys::from(keys);
        let rid = RecordId::from_table_key(self.table.clone(), dbkeys.info.id.to_string());
        let _resp: Option<DBKeys> = self.db.insert(rid).content(dbkeys).await?;
        Ok(())
    }

    pub async fn load(&self, kid: &KeysetID) -> SurrealResult<Option<KeysetEntry>> {
        let rid = RecordId::from_table_key(self.table.clone(), kid.to_string());
        let response: Option<DBKeys> = self.db.select(rid).await?;
        Ok(response.map(|dbk| dbk.into()))
    }

    pub async fn info(&self, kid: &KeysetID) -> SurrealResult<Option<cdk_mint::MintKeySetInfo>> {
        let rid = RecordId::from_table_key(self.table.clone(), kid.to_string());
        let result: Option<cdk_mint::MintKeySetInfo> = self
            .db
            .query("SELECT info FROM $rid")
            .bind(("rid", rid))
            .await?
            .take(0)?;
        Ok(result)
    }

    pub async fn keyset(&self, kid: &KeysetID) -> SurrealResult<Option<cdk02::MintKeySet>> {
        self.load(kid)
            .await
            .map(|res| res.map(|(_, keyset)| keyset))
    }
}
