// ----- standard library imports
// ----- extra library imports
use async_trait::async_trait;
use cashu::{
    nut00 as cdk00, nut01 as cdk01, nut02 as cdk02, nut12 as cdk12, secret::Secret, Amount,
};
use surrealdb::RecordId;
use surrealdb::{engine::any::Any, Result as SurrealResult, Surreal};
use uuid::Uuid;
// ----- local modules
// ----- local imports
use crate::credit::{PremintSignatures, Repository};
use crate::error::{Error, Result};

#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct ConnectionConfig {
    pub connection: String,
    pub namespace: String,
    pub database: String,
    pub secrets: String,
    pub counters: String,
    pub signatures: String,
    pub proofs: String,
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
    request_id: Uuid,
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

#[derive(Debug, Default, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
struct DBEntryCounter {
    counter: u32,
}

#[derive(Debug, Default, Clone, serde::Serialize, serde::Deserialize)]
struct DBEntrySignatures {
    request_id: Uuid,
    signatures: Vec<cdk00::BlindSignature>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct DBEntryProof {
    id: RecordId,
    amount: Amount,
    keyset_id: cdk02::Id,
    secret: cashu::secret::Secret,
    c: cdk01::PublicKey,
    witness: Option<cdk00::Witness>,
    dleq: Option<cdk12::ProofDleq>,
}
fn convert_to_db_entry_proof(id: RecordId, entry: cdk00::Proof) -> DBEntryProof {
    DBEntryProof {
        id,
        amount: entry.amount,
        keyset_id: entry.keyset_id,
        secret: entry.secret,
        c: entry.c,
        witness: entry.witness,
        dleq: entry.dleq,
    }
}
impl std::convert::From<DBEntryProof> for cdk00::Proof {
    fn from(entry: DBEntryProof) -> Self {
        Self {
            amount: entry.amount,
            keyset_id: entry.keyset_id,
            secret: entry.secret,
            c: entry.c,
            witness: entry.witness,
            dleq: entry.dleq,
        }
    }
}

#[derive(Debug, Clone, serde::Deserialize)]
struct DBEntryBalance {
    keyset_id: cdk02::Id,
    amount: Amount,
}

#[derive(Debug, Clone)]
pub struct DBRepository {
    db: Surreal<Any>,
    secrets: String,
    counters: String,
    signatures: String,
    proofs: String,
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
            proofs: config.proofs,
        })
    }

    async fn next_counter(&self, kid: cdk02::Id) -> SurrealResult<DBEntryCounter> {
        let rid = RecordId::from_table_key(&self.counters, kid.to_string());
        let val: Option<DBEntryCounter> = self.db.select(rid).await?;
        Ok(val.unwrap_or_default())
    }
    async fn increment_counter(&self, kid: cdk02::Id, inc: u32) -> SurrealResult<()> {
        let rid = RecordId::from_table_key(&self.counters, kid.to_string());
        let mut val: DBEntryCounter = self.db.select(rid.clone()).await?.unwrap_or_default();
        val.counter += inc;
        let _: Option<DBEntryCounter> = self.db.upsert(rid).content(val).await?;
        Ok(())
    }

    async fn store_secrets(&self, entry: DBEntryPremintSecret) -> SurrealResult<()> {
        let rid = RecordId::from_table_key(&self.secrets, entry.request_id);
        let _: Option<DBEntryPremintSecret> = self.db.insert(rid).content(entry).await?;
        Ok(())
    }
    async fn load_secrets(&self, rid: Uuid) -> SurrealResult<Option<DBEntryPremintSecret>> {
        let rid = RecordId::from_table_key(&self.secrets, rid);
        self.db.select(rid).await
    }
    async fn delete_secrets(&self, rid: Uuid) -> SurrealResult<()> {
        let rid = RecordId::from_table_key(&self.secrets, rid);
        let _: Option<DBEntryPremintSecret> = self.db.delete(rid).await?;
        Ok(())
    }

    async fn store_premint_signatures(&self, entry: DBEntrySignatures) -> SurrealResult<()> {
        let rid = RecordId::from_table_key(&self.signatures, entry.request_id);
        let _: Option<DBEntrySignatures> = self.db.insert(rid).content(entry).await?;
        Ok(())
    }
    async fn list_premint_signatures(&self) -> SurrealResult<Vec<DBEntrySignatures>> {
        let statement = String::from("SELECT * FROM type::table($table)");
        self.db
            .query(statement)
            .bind(("table", self.signatures.clone()))
            .await?
            .take(0)
    }
    async fn delete_premint_signatures(&self, request_id: Uuid) -> SurrealResult<()> {
        let rid = RecordId::from_table_key(&self.signatures, request_id);
        let _: Option<DBEntrySignatures> = self.db.delete(rid).await?;
        Ok(())
    }

    async fn store_proofs(&self, proofs: Vec<cdk00::Proof>) -> SurrealResult<()> {
        let mut dbproofs = Vec::with_capacity(proofs.len());
        for proof in proofs.into_iter() {
            let rid = RecordId::from_table_key(&self.proofs, proof.secret.to_string());
            dbproofs.push(convert_to_db_entry_proof(rid, proof));
        }
        let _: Vec<DBEntryProof> = self.db.insert(&self.proofs).content(dbproofs).await?;
        Ok(())
    }

    async fn list_balance_by_keyset_id(&self) -> SurrealResult<Vec<(cdk02::Id, Amount)>> {
        let statement = String::from(
            "SELECT keyset_id, math::sum(amount) AS amount FROM type::table($table) GROUP BY keyset_id",
        );
        let balances: Vec<DBEntryBalance> = self
            .db
            .query(statement)
            .bind(("table", self.proofs.clone()))
            .await?
            .take(0)?;
        dbg!(&balances);
        let mut ret_val = Vec::with_capacity(balances.len());
        for balance in balances {
            let DBEntryBalance { keyset_id, amount } = balance;
            ret_val.push((keyset_id, amount));
        }
        Ok(ret_val)
    }
}

#[async_trait]
impl Repository for DBRepository {
    async fn next_counter(&self, kid: cdk02::Id) -> Result<u32> {
        let entry = self.next_counter(kid).await.map_err(Error::DB)?;
        Ok(entry.counter)
    }
    async fn increment_counter(&self, kid: cdk02::Id, inc: u32) -> Result<()> {
        self.increment_counter(kid, inc).await.map_err(Error::DB)?;
        Ok(())
    }

    async fn store_secrets(&self, request_id: Uuid, premint: cdk00::PreMintSecrets) -> Result<()> {
        let cdk00::PreMintSecrets { keyset_id, secrets } = premint;
        let secrets: Vec<DBEntryPremint> =
            secrets.into_iter().map(std::convert::From::from).collect();
        let entry = DBEntryPremintSecret {
            request_id,
            kid: keyset_id,
            secrets,
        };
        self.store_secrets(entry).await.map_err(Error::DB)?;
        Ok(())
    }

    async fn load_secrets(&self, rid: Uuid) -> Result<cdk00::PreMintSecrets> {
        let entry: Option<DBEntryPremintSecret> =
            self.load_secrets(rid).await.map_err(Error::DB)?;
        let entry = entry.ok_or(Error::RequestIDNotFound(rid))?;
        Ok(cdk00::PreMintSecrets::from(entry))
    }

    async fn delete_secrets(&self, rid: Uuid) -> Result<()> {
        self.delete_secrets(rid).await.map_err(Error::DB)?;
        Ok(())
    }

    async fn store_premint_signatures(
        &self,
        (request_id, signatures): PremintSignatures,
    ) -> Result<()> {
        let entry = DBEntrySignatures {
            request_id,
            signatures,
        };
        self.store_premint_signatures(entry)
            .await
            .map_err(Error::DB)?;
        Ok(())
    }

    async fn list_premint_signatures(&self) -> Result<Vec<(Uuid, Vec<cdk00::BlindSignature>)>> {
        let entries = self.list_premint_signatures().await.map_err(Error::DB)?;
        let ret_val = entries
            .into_iter()
            .map(|entry| {
                let DBEntrySignatures {
                    request_id,
                    signatures,
                } = entry;
                (request_id, signatures)
            })
            .collect();
        Ok(ret_val)
    }
    async fn delete_premint_signatures(&self, rid: Uuid) -> Result<()> {
        self.delete_premint_signatures(rid)
            .await
            .map_err(Error::DB)?;
        Ok(())
    }

    async fn store_proofs(&self, proofs: Vec<cdk00::Proof>) -> Result<()> {
        self.store_proofs(proofs).await.map_err(Error::DB)?;
        Ok(())
    }
    async fn list_balance_by_keyset_id(&self) -> Result<Vec<(cdk02::Id, Amount)>> {
        let balances = self.list_balance_by_keyset_id().await.map_err(Error::DB)?;
        Ok(balances)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bcr_wdc_keys::test_utils as keys_utils;
    use bcr_wdc_swap_service::utils as swap_utils;

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
            proofs: "proofs".to_string(),
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
        assert_eq!(c.counter, 0);
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
        assert_eq!(c.counter, 42);
    }

    #[tokio::test]
    async fn list_balance_by_keyset_id_empty() {
        let db = init_mem_db().await;
        let balances = db.list_balance_by_keyset_id().await.unwrap();
        assert_eq!(balances.len(), 0);
    }

    #[tokio::test]
    async fn list_balance_by_keyset_id() {
        let db = init_mem_db().await;

        let (_, keyset) = keys_utils::generate_random_keyset();
        let proofs =
            swap_utils::generate_proofs(&keyset, &[Amount::from(8_u64), Amount::from(4_u64)]);
        db.store_proofs(proofs).await.unwrap();

        let mut expected = vec![(keyset.id, Amount::from(12_u64))];

        let (_, keyset) = keys_utils::generate_random_keyset();
        let proofs =
            swap_utils::generate_proofs(&keyset, &[Amount::from(16_u64), Amount::from(4_u64)]);
        db.store_proofs(proofs).await.unwrap();

        expected.push((keyset.id, Amount::from(20_u64)));

        let mut balances = db.list_balance_by_keyset_id().await.unwrap();

        expected.sort_by(|a, b| a.0.cmp(&b.0));
        balances.sort_by(|a, b| a.0.cmp(&b.0));
        assert_eq!(balances, expected);
    }
}
