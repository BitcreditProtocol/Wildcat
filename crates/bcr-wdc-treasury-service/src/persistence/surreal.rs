// ----- standard library imports
// ----- extra library imports
use async_trait::async_trait;
use cashu::nut00 as cdk00;
use cashu::nut01 as cdk01;
use cashu::nut02 as cdk02;
use cashu::secret::Secret;
use cashu::Amount;
use surrealdb::RecordId;
use surrealdb::Result as SurrealResult;
use surrealdb::{engine::any::Any, Surreal};
use uuid::Uuid;
// ----- local modules
// ----- local imports
use crate::credit::Repository;
use crate::error::{Error, Result};

#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct ConnectionConfig {
    pub connection: String,
    pub namespace: String,
    pub database: String,
    pub secrets: String,
    pub counters: String,
    pub signatures: String,
}

// cdk00::PreMint is not Deserialize
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct DBEntryPremint {
    blinded: cdk00::BlindedMessage,
    secret: Secret,
    r: cdk01::SecretKey,
    amount: Amount,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct DBEntryPremintSecret {
    rid: Uuid,
    kid: cdk02::Id,
    secrets: Vec<DBEntryPremint>,
}

impl std::convert::From<DBEntryPremint> for cdk00::PreMint {
    fn from(entry: DBEntryPremint) -> Self {
        Self {
            blinded_message: entry.blinded,
            secret: entry.secret,
            r: entry.r,
            amount: entry.amount,
        }
    }
}

impl std::convert::From<cdk00::PreMint> for DBEntryPremint {
    fn from(entry: cdk00::PreMint) -> Self {
        Self {
            blinded: entry.blinded_message,
            secret: entry.secret,
            r: entry.r,
            amount: entry.amount,
        }
    }
}

impl std::convert::From<DBEntryPremintSecret> for cdk00::PreMintSecrets {
    fn from(entry: DBEntryPremintSecret) -> Self {
        let DBEntryPremintSecret { kid, secrets, .. } = entry;
        let secrets: Vec<cdk00::PreMint> = secrets.into_iter().map(|e| e.into()).collect();
        Self {
            keyset_id: kid,
            secrets,
        }
    }
}

#[derive(Debug, Default, Clone, serde::Serialize, serde::Deserialize)]
struct DBEntryCounter {
    counter: u32,
}

#[derive(Debug, Default, Clone, serde::Serialize, serde::Deserialize)]
struct DBEntrySignatures {
    rid: Uuid,
    signatures: Vec<cdk00::BlindSignature>,
}

#[derive(Debug, Clone)]
pub struct DBRepository {
    db: Surreal<Any>,
    secrets: String,
    counters: String,
    signatures: String,
}

impl DBRepository {
    pub async fn new(config: ConnectionConfig) -> SurrealResult<Self> {
        let db_connection = Surreal::<Any>::init();
        db_connection.connect(config.connection).await?;
        db_connection.use_ns(config.namespace).await?;
        db_connection.use_db(config.database).await?;
        Ok(Self {
            db: db_connection,
            secrets: config.secrets,
            counters: config.counters,
            signatures: config.signatures,
        })
    }
}

#[async_trait]
impl Repository for DBRepository {
    async fn next_counter(&self, kid: cdk02::Id) -> Result<u32> {
        let rid = RecordId::from_table_key(&self.counters, kid.to_string());
        let val: Option<DBEntryCounter> = self.db.select(rid).await.map_err(Error::DB)?;
        Ok(val.unwrap_or_default().counter)
    }

    async fn increment_counter(&self, kid: cdk02::Id, inc: u32) -> Result<()> {
        let rid = RecordId::from_table_key(&self.counters, kid.to_string());
        let mut val: Option<DBEntryCounter> =
            self.db.select(rid.clone()).await.map_err(Error::DB)?;
        val.get_or_insert_default().counter += inc;
        let _: Option<DBEntryCounter> =
            self.db.upsert(rid).content(val).await.map_err(Error::DB)?;
        Ok(())
    }

    async fn store_secrets(&self, rid: Uuid, premint: cdk00::PreMintSecrets) -> Result<()> {
        let cdk00::PreMintSecrets { keyset_id, secrets } = premint;
        let secrets: Vec<DBEntryPremint> =
            secrets.into_iter().map(std::convert::From::from).collect();
        let entry = DBEntryPremintSecret {
            rid,
            kid: keyset_id,
            secrets,
        };

        let rid = RecordId::from_table_key(&self.secrets, rid);
        let _: Option<DBEntryPremintSecret> = self
            .db
            .insert(rid)
            .content(entry)
            .await
            .map_err(Error::DB)?;
        Ok(())
    }

    async fn store_signatures(
        &self,
        rid: Uuid,
        signatures: Vec<cdk00::BlindSignature>,
    ) -> Result<()> {
        let entry = DBEntrySignatures { rid, signatures };

        let rid = RecordId::from_table_key(&self.signatures, rid);
        let _: Option<DBEntrySignatures> = self
            .db
            .insert(rid)
            .content(entry)
            .await
            .map_err(Error::DB)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bcr_wdc_keys::test_utils as keys_utils;

    async fn init_mem_db() -> DBRepository {
        let sdb = Surreal::<Any>::init();
        sdb.connect("mem://").await.unwrap();
        sdb.use_ns("test").await.unwrap();
        sdb.use_db("test").await.unwrap();
        DBRepository {
            db: sdb,
            secrets: "secrets".to_string(),
            counters: "counters".to_string(),
            signatures: "signatures".to_string(),
        }
    }

    #[tokio::test]
    async fn next_counter_newkeyset() {
        let db = init_mem_db().await;
        let kid = keys_utils::generate_random_keysetid();
        let c = db
            .next_counter(kid.into())
            .await
            .expect("next_counter failed");
        assert_eq!(c, 0);
    }

    #[tokio::test]
    async fn next_counter_existingkeyset() {
        let db = init_mem_db().await;
        let kid = keys_utils::generate_random_keysetid();
        let rid = RecordId::from_table_key(&db.counters, kid.to_string());
        let _resp: Option<DBEntryCounter> = db
            .db
            .insert(rid)
            .content(DBEntryCounter { counter: 42 })
            .await
            .expect("insert failed");
        let c = db
            .next_counter(kid.into())
            .await
            .expect("next_counter failed");
        assert_eq!(c, 42);
    }
}
