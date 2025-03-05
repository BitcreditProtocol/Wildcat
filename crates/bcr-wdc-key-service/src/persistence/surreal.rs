// ----- standard library imports
// ----- extra library imports
use anyhow::anyhow;
use async_trait::async_trait;
use bcr_wdc_keys::{persistence::KeysDB, KeysetID};
use cashu::mint::MintKeySetInfo;
use cashu::nuts::nut02 as cdk02;
// ----- local imports
use crate::error::{Error, Result};
use crate::service::KeysRepository;

#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct ConnectionConfig {
    pub connection: String,
    pub namespace: String,
    pub database: String,
    pub table: String,
}

#[derive(Debug, Clone)]
pub struct DB(KeysDB);

impl DB {
    pub async fn new(cfg: ConnectionConfig) -> Result<Self> {
        let db = KeysDB::new(&cfg.connection, &cfg.namespace, &cfg.database, &cfg.table)
            .await
            .map_err(|e| Error::KeysRepository(anyhow!(e)))?;
        Ok(Self(db))
    }
}
#[async_trait]
impl KeysRepository for DB {
    async fn info(&self, id: &cdk02::Id) -> Result<Option<MintKeySetInfo>> {
        let kid = KeysetID::from(*id);
        self.0
            .info(&kid)
            .await
            .map_err(|e| Error::KeysRepository(anyhow!(e)))
    }
    async fn keyset(&self, id: &cdk02::Id) -> Result<Option<cdk02::MintKeySet>> {
        let kid = KeysetID::from(*id);
        self.0
            .keyset(&kid)
            .await
            .map_err(|e| Error::KeysRepository(anyhow!(e)))
    }
}
