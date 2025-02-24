// ----- standard library imports
// ----- extra library imports
use anyhow::Result as AnyResult;
use async_trait::async_trait;
use cdk::nuts::nut00 as cdk00;
use cdk::nuts::nut01 as cdk01;
use cdk::nuts::nut07 as cdk07;
use surrealdb::RecordId;
use surrealdb::Result as SurrealResult;
use surrealdb::{engine::any::Any, Surreal};
// ----- local modules
// ----- local imports
use crate::persistence::surreal::ConnectionConfig;
use crate::swap;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DBProof {
    id: RecordId,
    y: cdk01::PublicKey,
    state: cdk07::State,
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
}

#[async_trait]
impl swap::ProofRepository for DB {
    async fn spend(&self, tokens: &[cdk00::Proof]) -> AnyResult<()> {
        let mut entries: Vec<DBProof> = Vec::with_capacity(tokens.len());
        for tk in tokens {
            let y = cdk::dhke::hash_to_curve(&tk.secret.to_bytes())?;
            let rid = RecordId::from_table_key(&self.table, y.to_string());
            entries.push(DBProof {
                id: rid,
                y,
                state: cdk07::State::Spent,
            });
        }
        let _: Vec<DBProof> = self.db.insert(&self.table).content(entries).await?;
        Ok(())
    }

    async fn get_state(&self, tokens: &[cdk00::Proof]) -> AnyResult<Vec<cdk07::State>> {
        let rids: Vec<_> = tokens
            .iter()
            .map(|tk| {
                let y = cdk::dhke::hash_to_curve(&tk.secret.to_bytes())?;
                Ok(RecordId::from_table_key(&self.table, y.to_string()))
            })
            .collect::<AnyResult<_>>()?;

        let resp: Vec<DBProof> = self
            .db
            .query("SELECT * FROM $rids")
            .bind(("rids", rids.clone()))
            .await?
            .take(0)?;

        let mut states = Vec::with_capacity(rids.len());
        for rid in rids {
            let found = resp.iter().find(|r| r.id == rid);
            if let Some(r) = found {
                states.push(r.state);
            } else {
                states.push(cdk07::State::Unspent);
            }
        }
        Ok(states)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keys::test_utils as keys_test;
    use crate::swap::ProofRepository;
    use crate::utils::tests as utils;

    async fn init_mem_db() -> DB {
        let sdb = Surreal::<Any>::init();
        sdb.connect("mem://").await.unwrap();
        sdb.use_ns("test").await.unwrap();
        sdb.use_db("test").await.unwrap();
        DB {
            db: sdb,
            table: "test".to_string(),
        }
    }

    #[tokio::test]
    async fn test_spend() {
        let db = init_mem_db().await;
        let mintkeys = &keys_test::generate_keyset();
        let proofs = utils::generate_proofs(
            mintkeys,
            &[cdk::Amount::from(16_u64), cdk::Amount::from(8_u64)],
        );
        db.spend(&proofs).await.unwrap();

        let y = cdk::dhke::hash_to_curve(&proofs[0].secret.to_bytes()).unwrap();
        let rid = RecordId::from_table_key(&db.table, y.to_string());
        let res: Option<DBProof> = db.db.select(rid).await.unwrap();
        assert!(res.is_some());
        assert_eq!(res.unwrap().state, cdk07::State::Spent);

        let y = cdk::dhke::hash_to_curve(&proofs[0].secret.to_bytes()).unwrap();
        let rid = RecordId::from_table_key(&db.table, y.to_string());
        let res: Option<DBProof> = db.db.select(rid).await.unwrap();
        assert!(res.is_some());
        assert_eq!(res.unwrap().state, cdk07::State::Spent);
    }

    #[tokio::test]
    async fn test_get_state() {
        let db = init_mem_db().await;

        let mintkeys = &keys_test::generate_keyset();
        let proofs = utils::generate_proofs(
            mintkeys,
            &[
                cdk::Amount::from(16_u64),
                cdk::Amount::from(8_u64),
                cdk::Amount::from(4_u64),
            ],
        );

        let y = cdk::dhke::hash_to_curve(&proofs[0].secret.to_bytes()).unwrap();
        let rid = RecordId::from_table_key(&db.table, y.to_string());
        let _r: Option<DBProof> = db
            .db
            .insert(rid)
            .content(DBProof {
                id: RecordId::from_table_key(&db.table, y.to_string()),
                y,
                state: cdk07::State::Spent,
            })
            .await
            .unwrap();

        let y = cdk::dhke::hash_to_curve(&proofs[1].secret.to_bytes()).unwrap();
        let rid = RecordId::from_table_key(&db.table, y.to_string());
        let _r: Option<DBProof> = db
            .db
            .insert(rid)
            .content(DBProof {
                id: RecordId::from_table_key(&db.table, y.to_string()),
                y,
                state: cdk07::State::Spent,
            })
            .await
            .unwrap();

        let res: Vec<cdk07::State> = db.get_state(&proofs).await.unwrap();
        assert!(res.len() == 3);
        assert_eq!(res[0], cdk07::State::Spent);
        assert_eq!(res[1], cdk07::State::Spent);
        assert_eq!(res[2], cdk07::State::Unspent);
    }
}
