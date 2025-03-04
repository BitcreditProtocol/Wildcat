// ----- standard library imports
// ----- extra library imports
use anyhow::anyhow;
use async_trait::async_trait;
use cashu::dhke as cdk_dhke;
use cashu::nuts::nut00 as cdk00;
use surrealdb::RecordId;
use surrealdb::Result as SurrealResult;
use surrealdb::{Surreal, engine::any::Any};
// ----- local modules
// ----- local imports
use crate::error::{Error, Result};
use crate::service::ProofRepository;

#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct ConnectionConfig {
    pub connection: String,
    pub namespace: String,
    pub database: String,
    pub table: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DBProof {
    id: RecordId,
    proof: cdk00::Proof,
}

#[derive(Debug, Clone)]
pub struct ProofDB {
    db: Surreal<surrealdb::engine::any::Any>,
    table: String,
}

impl ProofDB {
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
impl ProofRepository for ProofDB {
    async fn insert(&self, tokens: &[cdk00::Proof]) -> Result<()> {
        let mut entries: Vec<DBProof> = Vec::with_capacity(tokens.len());
        for tk in tokens {
            let y = cdk_dhke::hash_to_curve(&tk.secret.to_bytes()).map_err(Error::CdkDhke)?;
            let rid = RecordId::from_table_key(&self.table, y.to_string());
            entries.push(DBProof {
                id: rid,
                proof: tk.clone(),
            });
        }
        let _: Vec<DBProof> = self
            .db
            .insert(&self.table)
            .content(entries)
            .await
            .map_err(|e| match e {
                surrealdb::Error::Db(surrealdb::error::Db::RecordExists { .. }) => {
                    Error::ProofsAlreadySpent
                }
                _ => Error::ProofRepository(anyhow!(e)),
            })?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils;
    use bcr_wdc_keys::test_utils as keys_test;
    use cashu::Amount as cdk_Amount;

    async fn init_mem_db() -> ProofDB {
        let sdb = Surreal::<Any>::init();
        sdb.connect("mem://").await.unwrap();
        sdb.use_ns("test").await.unwrap();
        sdb.use_db("test").await.unwrap();
        ProofDB {
            db: sdb,
            table: "test".to_string(),
        }
    }

    #[tokio::test]
    async fn test_insert() {
        let db = init_mem_db().await;
        let (_, keyset) = keys_test::generate_keyset();
        let proofs = utils::generate_proofs(
            &keyset,
            &[cdk_Amount::from(16_u64), cdk_Amount::from(8_u64)],
        );
        db.insert(&proofs).await.unwrap();

        let y = cdk_dhke::hash_to_curve(&proofs[0].secret.to_bytes()).unwrap();
        let rid = RecordId::from_table_key(&db.table, y.to_string());
        let res: Option<DBProof> = db.db.select(rid).await.unwrap();
        assert!(res.is_some());
        assert_eq!(res.unwrap().proof.secret, proofs[0].secret);

        let y = cdk_dhke::hash_to_curve(&proofs[1].secret.to_bytes()).unwrap();
        let rid = RecordId::from_table_key(&db.table, y.to_string());
        let res: Option<DBProof> = db.db.select(rid).await.unwrap();
        assert!(res.is_some());
        assert_eq!(res.unwrap().proof.secret, proofs[1].secret);
    }

    #[tokio::test]
    async fn test_insert_double_spent_all() {
        let db = init_mem_db().await;
        let (_, keyset) = keys_test::generate_keyset();
        let proofs = utils::generate_proofs(
            &keyset,
            &[cdk_Amount::from(16_u64), cdk_Amount::from(8_u64)],
        );
        db.insert(&proofs).await.unwrap();

        let res = db.insert(&proofs).await;
        assert!(res.is_err());
        dbg!(&res);
        assert!(matches!(res.unwrap_err(), Error::ProofsAlreadySpent));
    }

    #[tokio::test]
    async fn test_insert_double_spent_partial() {
        let db = init_mem_db().await;
        let (_, keyset) = keys_test::generate_keyset();
        let proofs = utils::generate_proofs(
            &keyset,
            &[
                cdk_Amount::from(16_u64),
                cdk_Amount::from(8_u64),
                cdk_Amount::from(4_u64),
            ],
        );
        db.insert(&proofs[0..2]).await.unwrap();

        let res = db.insert(&proofs[1..]).await;
        assert!(res.is_err());
        assert!(matches!(res.unwrap_err(), Error::ProofsAlreadySpent));
    }

    #[tokio::test]
    async fn test_insert_double_spent_partial_still_valid() {
        let db = init_mem_db().await;
        let (_, keyset) = keys_test::generate_keyset();
        let proofs = utils::generate_proofs(
            &keyset,
            &[
                cdk_Amount::from(16_u64),
                cdk_Amount::from(8_u64),
                cdk_Amount::from(4_u64),
            ],
        );
        db.insert(&proofs[0..2]).await.unwrap();

        let res = db.insert(&proofs[1..]).await;
        assert!(res.is_err());
        db.insert(&proofs[2..]).await.unwrap();
    }
}
