// ----- standard library imports
// ----- extra library imports
use anyhow::anyhow;
use async_trait::async_trait;
use cashu::{nut00 as cdk00, nut02 as cdk02, nut07 as cdk07};
use surrealdb::RecordId;
use surrealdb::Result as SurrealResult;
use surrealdb::{engine::any::Any, Surreal};
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
    kid: cdk02::Id,
    secret: cashu::secret::Secret,
    c: cashu::PublicKey,
    witness: Option<cdk00::Witness>,
}
fn convert_to_db(proof: &cdk00::Proof, table: &str) -> Result<DBProof> {
    let rid = proof_to_record_id(table, proof)?;
    let dbentry = DBProof {
        id: rid,
        kid: proof.keyset_id,
        secret: proof.secret.clone(),
        c: proof.c,
        witness: proof.witness.clone(),
    };
    Ok(dbentry)
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
            let db_entry = convert_to_db(tk, &self.table)?;
            entries.push(db_entry);
        }
        let _: Vec<DBProof> = self
            .db
            .insert(())
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

    async fn remove(&self, tokens: &[cdk00::Proof]) -> Result<()> {
        for tk in tokens {
            let rid = proof_to_record_id(&self.table, tk)?;
            let _p: Option<cdk00::Proof> = self
                .db
                .delete(rid)
                .await
                .map_err(|e| Error::ProofRepository(anyhow!(e)))?;
        }
        Ok(())
    }

    async fn contains(&self, y: cashu::PublicKey) -> Result<Option<cdk07::ProofState>> {
        let rid = y_to_record_id(&self.table, y);
        let res: Option<DBProof> = self
            .db
            .select(rid)
            .await
            .map_err(|e| Error::ProofRepository(anyhow!(e)))?;
        if res.is_some() {
            let ret_v = cdk07::ProofState {
                y,
                state: cdk07::State::Spent,
                witness: None,
            };
            return Ok(Some(ret_v));
        }
        Ok(None)
    }
}

fn proof_to_record_id(main_table: &str, proof: &cdk00::Proof) -> Result<RecordId> {
    let y = cashu::dhke::hash_to_curve(proof.secret.as_bytes()).map_err(Error::CdkDhke)?;
    Ok(y_to_record_id(main_table, y))
}
fn y_to_record_id(main_table: &str, y: cashu::PublicKey) -> RecordId {
    RecordId::from_table_key(main_table, y.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use bcr_wdc_utils::keys::test_utils as keys_test;
    use bcr_wdc_utils::signatures::test_utils as signatures_test;
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
        let proofs = signatures_test::generate_proofs(
            &keyset,
            &[cdk_Amount::from(16_u64), cdk_Amount::from(8_u64)],
        );
        db.insert(&proofs).await.unwrap();

        let rid = proof_to_record_id(&db.table, &proofs[0]).expect("Failed to get record id");
        let res: Option<DBProof> = db.db.select(rid).await.unwrap();
        assert!(res.is_some());
        assert_eq!(res.unwrap().secret, proofs[0].secret);

        let rid = proof_to_record_id(&db.table, &proofs[1]).expect("Failed to get record id");
        let res: Option<DBProof> = db.db.select(rid).await.unwrap();
        assert!(res.is_some());
        assert_eq!(res.unwrap().secret, proofs[1].secret);
    }

    #[tokio::test]
    async fn test_insert_double_spent_all() {
        let db = init_mem_db().await;
        let (_, keyset) = keys_test::generate_keyset();
        let proofs = signatures_test::generate_proofs(
            &keyset,
            &[cdk_Amount::from(16_u64), cdk_Amount::from(8_u64)],
        );
        db.insert(&proofs).await.unwrap();

        let res = db.insert(&proofs).await;
        assert!(res.is_err());
        assert!(matches!(res.unwrap_err(), Error::ProofsAlreadySpent));
    }

    #[tokio::test]
    async fn test_insert_double_spent_partial() {
        let db = init_mem_db().await;
        let (_, keyset) = keys_test::generate_keyset();
        let proofs = signatures_test::generate_proofs(
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
        let proofs = signatures_test::generate_proofs(
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
