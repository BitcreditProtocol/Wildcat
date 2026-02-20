// ----- standard library imports
use std::str::FromStr;
// ----- extra library imports
use anyhow::anyhow;
use async_trait::async_trait;
use bcr_common::cashu;
use bcr_wdc_utils::surreal;
use bitcoin::secp256k1::schnorr::Signature;
use surrealdb::{engine::any::Any, RecordId, Result as SurrealResult, Surreal};
// ----- local imports
use crate::{
    commitment,
    error::{Error, Result},
    TStamp,
};

// ----- end imports

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct CommitmentDBEntry {
    id: RecordId,
    inputs: Vec<cashu::PublicKey>,
    outputs: Vec<cashu::PublicKey>,
    expiration: TStamp,
}

#[derive(Debug, Clone)]
pub struct DBCommitments {
    db: Surreal<Any>,
}

impl DBCommitments {
    const TABLE: &'static str = "commitments";

    pub async fn new(cfg: surreal::DBConnConfig) -> SurrealResult<Self> {
        let db_connection = Surreal::<Any>::init();
        db_connection.connect(cfg.connection).await?;
        db_connection.use_ns(cfg.namespace).await?;
        db_connection.use_db(cfg.database).await?;
        Ok(Self { db: db_connection })
    }
}

#[async_trait]
impl commitment::Repository for DBCommitments {
    async fn clean_expired(&self, now: TStamp) -> Result<()> {
        self.db
            .query("DELETE FROM type::table($table) WHERE expiration < $now")
            .bind(("table", Self::TABLE))
            .bind(("now", now))
            .await
            .map_err(|e| Error::DB(anyhow!("SurrealDB error: {}", e)))?;
        Ok(())
    }

    async fn check_committed_inputs(&self, ys: &[cashu::PublicKey]) -> Result<bool> {
        let commitment: Option<CommitmentDBEntry> = self
            .db
            .query("SELECT * FROM type::table($table) WHERE array::is_empty(array::intersect(inputs, $ys)) = false LIMIT 1")
            .bind(("table", Self::TABLE))
            .bind(("ys", ys.to_vec()))
            .await
            .map_err(|e| Error::DB(anyhow!("SurrealDB error: {}", e)))?
            .take(0)
            .map_err(|e| Error::DB(anyhow!("SurrealDB error: {}", e)))?;
        Ok(commitment.is_some())
    }
    async fn check_committed_outputs(&self, secrets: &[cashu::PublicKey]) -> Result<bool> {
        let commitment: Option<CommitmentDBEntry> = self
            .db
            .query("SELECT * FROM type::table($table) WHERE array::is_empty(array::intersect(outputs, $secrets)) = false LIMIT 1")
            .bind(("table", Self::TABLE))
            .bind(("secrets", secrets.to_vec()))
            .await
            .map_err(|e| Error::DB(anyhow!("SurrealDB error: {}", e)))?
            .take(0)
            .map_err(|e| Error::DB(anyhow!("SurrealDB error: {}", e)))?;
        Ok(commitment.is_some())
    }

    async fn store(
        &self,
        inputs: Vec<cashu::PublicKey>,
        outputs: Vec<cashu::PublicKey>,
        expiration: TStamp,
        signature: Signature,
    ) -> Result<()> {
        let rid = RecordId::from_table_key(Self::TABLE, signature.to_string());
        let entry = CommitmentDBEntry {
            id: rid.clone(),
            inputs,
            outputs,
            expiration,
        };
        let _: Option<CommitmentDBEntry> =
            self.db.insert(rid).content(entry).await.map_err(|e| {
                Error::DB(anyhow!("SurrealDB error while storing commitment: {}", e))
            })?;
        Ok(())
    }

    async fn find(
        &self,
        inputs: &[cashu::PublicKey],
        outputs: &[cashu::PublicKey],
    ) -> Result<Option<Signature>> {
        let commitment: Option<CommitmentDBEntry> = self
            .db
            .query(
                "SELECT * FROM type::table($table)
    WHERE 
        array::is_empty(array::difference(inputs, $inputs)) 
    AND 
        array::is_empty(array::difference(outputs, $outputs))
    LIMIT 1",
            )
            .bind(("table", Self::TABLE))
            .bind(("inputs", inputs.to_vec()))
            .bind(("outputs", outputs.to_vec()))
            .await
            .map_err(|e| Error::DB(anyhow!("SurrealDB error: {}", e)))?
            .take(0)
            .map_err(|e| Error::DB(anyhow!("SurrealDB error: {}", e)))?;
        let Some(entry) = commitment else {
            return Ok(None);
        };
        let key = entry.id.key().to_string();
        let commitment = Signature::from_str(&key).expect("signature from recordId");
        Ok(Some(commitment))
    }

    async fn delete(&self, commitment: Signature) -> Result<()> {
        let rid = RecordId::from_table_key(Self::TABLE, commitment.to_string());
        let _: Option<CommitmentDBEntry> =
            self.db.delete(rid).await.map_err(|e| {
                Error::DB(anyhow!("SurrealDB error while deleting commitment: {}", e))
            })?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bitcoin::{key::rand, secp256k1 as secp};
    use commitment::Repository;
    use rand::Rng;

    async fn init_mem_db() -> DBCommitments {
        let sdb = Surreal::<Any>::init();
        sdb.connect("mem://").await.unwrap();
        sdb.use_ns("test").await.unwrap();
        sdb.use_db("test").await.unwrap();
        DBCommitments { db: sdb }
    }

    fn random_cdk_pks(sz: usize) -> Vec<cashu::PublicKey> {
        std::iter::repeat_with(|| {
            let pk = secp::generate_keypair(&mut rand::thread_rng()).1;
            cashu::PublicKey::from(pk)
        })
        .take(sz)
        .collect()
    }

    fn random_signature() -> Signature {
        let mut sl = [0; secp::constants::SCHNORR_SIGNATURE_SIZE];
        rand::thread_rng().fill(&mut sl[..]);
        Signature::from_slice(&sl).unwrap()
    }

    #[tokio::test]
    async fn store() {
        let db = init_mem_db().await;
        let inputs = random_cdk_pks(5);
        let outputs = random_cdk_pks(3);
        let tstamp = TStamp::from_timestamp(100000, 0).unwrap();
        let signature = random_signature();
        db.store(inputs, outputs, tstamp, signature).await.unwrap();
    }

    #[tokio::test]
    async fn check_committed_inputs() {
        let db = init_mem_db().await;
        let inputs = random_cdk_pks(5);
        let outputs = random_cdk_pks(3);
        let tstamp = TStamp::from_timestamp(100000, 0).unwrap();
        let signature = random_signature();
        db.store(inputs.clone(), outputs.clone(), tstamp, signature)
            .await
            .unwrap();

        let mut tester = random_cdk_pks(2);
        let result = db.check_committed_inputs(&tester).await;
        assert!(!result.unwrap());
        tester.push(inputs[0]);
        let result = db.check_committed_inputs(&tester).await;
        assert!(result.unwrap());
        let result = db.check_committed_inputs(&inputs).await;
        assert!(result.unwrap());
        let result = db.check_committed_inputs(&outputs).await;
        assert!(!result.unwrap());
    }

    #[tokio::test]
    async fn check_committed_outputs() {
        let db = init_mem_db().await;
        let inputs = random_cdk_pks(5);
        let outputs = random_cdk_pks(3);
        let tstamp = TStamp::from_timestamp(100000, 0).unwrap();
        let signature = random_signature();
        db.store(inputs.clone(), outputs.clone(), tstamp, signature)
            .await
            .unwrap();

        let mut tester = random_cdk_pks(2);
        let result = db.check_committed_outputs(&tester).await;
        assert!(!result.unwrap());
        tester.push(outputs[0]);
        let result = db.check_committed_outputs(&tester).await;
        assert!(result.unwrap());
        let result = db.check_committed_outputs(&outputs).await;
        assert!(result.unwrap());
        let result = db.check_committed_outputs(&inputs).await;
        assert!(!result.unwrap());
    }

    #[tokio::test]
    async fn search() {
        let db = init_mem_db().await;
        let inputs = random_cdk_pks(5);
        let outputs = random_cdk_pks(3);
        let tstamp = TStamp::from_timestamp(100000, 0).unwrap();
        let signature = random_signature();
        db.store(inputs.clone(), outputs.clone(), tstamp, signature)
            .await
            .unwrap();

        let tester = random_cdk_pks(2);
        let result = db.find(&tester, &outputs).await.unwrap();
        assert!(result.is_none());
        let result = db.find(&inputs, &tester).await.unwrap();
        assert!(result.is_none());
        let mut tester = random_cdk_pks(4);
        tester.push(inputs[0]);
        let result = db.find(&tester, &outputs).await.unwrap();
        assert!(result.is_none());
        let mut tester = random_cdk_pks(2);
        tester.push(outputs[0]);
        let result = db.find(&inputs, &tester).await.unwrap();
        assert!(result.is_none());
        let result = db.find(&inputs, &outputs).await.unwrap();
        assert!(result.is_some());
        assert_eq!(result.unwrap(), signature);
    }
}
