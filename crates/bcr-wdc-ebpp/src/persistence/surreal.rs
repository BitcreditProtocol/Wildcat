// ----- standard library imports
use std::collections::HashMap;
use std::str::FromStr;
// ----- extra library imports
use anyhow::anyhow;
use async_trait::async_trait;
use bdk_wallet::miniscript::{
    bitcoin::hashes::Hash,
    descriptor::{DescriptorSecretKey, KeyMap},
    Descriptor, DescriptorPublicKey,
};
use surrealdb::{engine::any::Any, Result as SurrealResult, Surreal};
// ----- local imports
use crate::error::{Error, Result};
use crate::onchain::{PrivateKeysRepository, SingleKeyWallet};

// ----- end imports

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct KeysDBEntry {
    desc: Descriptor<DescriptorPublicKey>,
    kmap: HashMap<DescriptorPublicKey, String>, // DescriptorSecretKey
}

impl From<(Descriptor<DescriptorPublicKey>, KeyMap)> for KeysDBEntry {
    fn from(ke: SingleKeyWallet) -> Self {
        let (desc, kmap) = ke;
        let mut serialized_keys: HashMap<_, _> = Default::default();
        for (k, v) in kmap {
            serialized_keys.insert(k, v.to_string());
        }
        Self {
            desc,
            kmap: serialized_keys,
        }
    }
}

impl From<KeysDBEntry> for SingleKeyWallet {
    fn from(dbk: KeysDBEntry) -> Self {
        let KeysDBEntry { desc, kmap } = dbk;
        let mut keysmap: KeyMap = Default::default();
        for (k, v) in kmap {
            keysmap.insert(k, DescriptorSecretKey::from_str(&v).unwrap());
        }
        (desc, keysmap)
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
pub struct DBPrivateKeys {
    db: Surreal<surrealdb::engine::any::Any>,
    table: String,
}

impl DBPrivateKeys {
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

    async fn list_keys(&self) -> SurrealResult<Vec<KeysDBEntry>> {
        self.db
            .query("SELECT * FROM type::table($table)")
            .bind(("table", self.table.clone()))
            .await?
            .take(0)
    }
}

#[async_trait]
impl PrivateKeysRepository for DBPrivateKeys {
    async fn get_private_keys(&self) -> Result<Vec<SingleKeyWallet>> {
        let dbkeys = self.list_keys().await.map_err(|e| Error::DB(anyhow!(e)))?;
        let keys = dbkeys.into_iter().map(From::from).collect();
        Ok(keys)
    }

    async fn add_key(&self, key: SingleKeyWallet) -> Result<()> {
        let rkey = bitcoin::hashes::sha256::Hash::hash(key.0.to_string().as_bytes()).to_string();
        let rid = surrealdb::RecordId::from_table_key(&self.table, rkey);
        let dbkey = KeysDBEntry::from(key);
        let _resp: Option<KeysDBEntry> = self
            .db
            .insert(rid)
            .content(dbkey)
            .await
            .map_err(|e| Error::DB(anyhow!(e)))?;
        Ok(())
    }
}
