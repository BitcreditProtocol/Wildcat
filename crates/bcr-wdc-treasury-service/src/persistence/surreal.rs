// ----- standard library imports
// ----- extra library imports
use async_trait::async_trait;
use bcr_common::wire::keys as wire_keys;
use bitcoin::hashes::sha256::Hash as Sha256Hash;
use cashu::{secret::Secret, Amount};
use surrealdb::{engine::any::Any, RecordId, Result as SurrealResult, Surreal};
use uuid::Uuid;
// ----- local imports
use crate::{
    credit::{self, PremintSignatures},
    debit,
    error::{Error, Result},
    foreign,
};

// ----- end imports

#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct CreditConnectionConfig {
    pub connection: String,
    pub namespace: String,
    pub database: String,
    pub secrets: String,
    pub signatures: String,
    pub proofs: String,
}

// cashu::PreMint is not Deserialize
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct DBEntryPremint {
    blinded: cashu::BlindedMessage,
    secret: Secret,
    r: cashu::SecretKey,
    amount: Amount,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct DBEntryPremintSecret {
    request_id: Uuid,
    kid: cashu::Id,
    secrets: Vec<DBEntryPremint>,
}

impl std::convert::From<DBEntryPremint> for cashu::PreMint {
    fn from(entry: DBEntryPremint) -> Self {
        Self {
            blinded_message: entry.blinded,
            secret: entry.secret,
            r: entry.r,
            amount: entry.amount,
        }
    }
}

impl std::convert::From<cashu::PreMint> for DBEntryPremint {
    fn from(entry: cashu::PreMint) -> Self {
        Self {
            blinded: entry.blinded_message,
            secret: entry.secret,
            r: entry.r,
            amount: entry.amount,
        }
    }
}

impl std::convert::From<DBEntryPremintSecret> for cashu::PreMintSecrets {
    fn from(entry: DBEntryPremintSecret) -> Self {
        let DBEntryPremintSecret { kid, secrets, .. } = entry;
        let secrets: Vec<cashu::PreMint> = secrets.into_iter().map(|e| e.into()).collect();
        Self {
            keyset_id: kid,
            secrets,
        }
    }
}

#[derive(Debug, Default, Clone, serde::Serialize, serde::Deserialize)]
struct DBEntrySignatures {
    request_id: Uuid,
    signatures: Vec<cashu::BlindSignature>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct DBEntryProof {
    id: RecordId,
    amount: Amount,
    keyset_id: cashu::Id,
    secret: cashu::secret::Secret,
    c: cashu::PublicKey,
    witness: Option<cashu::Witness>,
    dleq: Option<cashu::ProofDleq>,
}
fn convert_to_db_entry_proof(id: RecordId, entry: cashu::Proof) -> DBEntryProof {
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
impl std::convert::From<DBEntryProof> for cashu::Proof {
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
    keyset_id: cashu::Id,
    amount: Amount,
}

#[derive(Debug, Clone)]
pub struct CreditRepository {
    db: Surreal<Any>,
    secrets: String,
    signatures: String,
    proofs: String,
}

impl CreditRepository {
    pub async fn new(config: CreditConnectionConfig) -> SurrealResult<Self> {
        let db_connection = Surreal::<Any>::init();
        db_connection.connect(config.connection).await?;
        db_connection.use_ns(config.namespace).await?;
        db_connection.use_db(config.database).await?;
        Ok(Self {
            db: db_connection,
            secrets: config.secrets,
            signatures: config.signatures,
            proofs: config.proofs,
        })
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

    async fn store_proofs(&self, proofs: Vec<cashu::Proof>) -> SurrealResult<()> {
        let mut dbproofs = Vec::with_capacity(proofs.len());
        for proof in proofs.into_iter() {
            let rid = RecordId::from_table_key(&self.proofs, proof.secret.to_string());
            dbproofs.push(convert_to_db_entry_proof(rid, proof));
        }
        let _: Vec<DBEntryProof> = self.db.insert(&self.proofs).content(dbproofs).await?;
        Ok(())
    }

    async fn list_balance_by_keyset_id(&self) -> SurrealResult<Vec<(cashu::Id, Amount)>> {
        let statement = String::from(
            "SELECT keyset_id, math::sum(amount) AS amount FROM type::table($table) GROUP BY keyset_id",
        );
        let balances: Vec<DBEntryBalance> = self
            .db
            .query(statement)
            .bind(("table", self.proofs.clone()))
            .await?
            .take(0)?;
        let mut ret_val = Vec::with_capacity(balances.len());
        for balance in balances {
            let DBEntryBalance { keyset_id, amount } = balance;
            ret_val.push((keyset_id, amount));
        }
        Ok(ret_val)
    }
}

#[async_trait]
impl credit::Repository for CreditRepository {
    async fn store_secrets(&self, request_id: Uuid, premint: cashu::PreMintSecrets) -> Result<()> {
        let cashu::PreMintSecrets { keyset_id, secrets } = premint;
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

    async fn load_secrets(&self, rid: Uuid) -> Result<cashu::PreMintSecrets> {
        let entry: Option<DBEntryPremintSecret> =
            self.load_secrets(rid).await.map_err(Error::DB)?;
        let entry = entry.ok_or(Error::RequestIDNotFound(rid))?;
        Ok(cashu::PreMintSecrets::from(entry))
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

    async fn list_premint_signatures(&self) -> Result<Vec<(Uuid, Vec<cashu::BlindSignature>)>> {
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

    async fn store_proofs(&self, proofs: Vec<cashu::Proof>) -> Result<()> {
        self.store_proofs(proofs).await.map_err(Error::DB)?;
        Ok(())
    }
    async fn list_balance_by_keyset_id(&self) -> Result<Vec<(cashu::Id, Amount)>> {
        let balances = self.list_balance_by_keyset_id().await.map_err(Error::DB)?;
        Ok(balances)
    }
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct DebitConnectionConfig {
    pub connection: String,
    pub namespace: String,
    pub database: String,
    pub table: String,
}

#[derive(Debug, Clone)]
pub struct DebitRepository {
    db: Surreal<Any>,
    table: String,
}
impl DebitRepository {
    pub async fn new(config: DebitConnectionConfig) -> SurrealResult<Self> {
        let db_connection = Surreal::<Any>::init();
        db_connection.connect(config.connection).await?;
        db_connection.use_ns(config.namespace).await?;
        db_connection.use_db(config.database).await?;
        Ok(Self {
            db: db_connection,
            table: config.table,
        })
    }
}

#[async_trait]
impl debit::Repository for DebitRepository {
    async fn store_quote(&self, quote: debit::MintQuote) -> Result<()> {
        let rid = RecordId::from_table_key(&self.table, quote.qid.clone());
        let _: Option<debit::MintQuote> = self
            .db
            .insert(rid)
            .content(quote)
            .await
            .map_err(Error::DB)?;
        Ok(())
    }

    async fn delete_quote(&self, quote_id: String) -> Result<()> {
        let rid = RecordId::from_table_key(&self.table, quote_id);
        let _: Option<debit::MintQuote> = self.db.delete(rid).await.map_err(Error::DB)?;
        Ok(())
    }

    async fn list_quotes(&self) -> Result<Vec<debit::MintQuote>> {
        let statement = String::from("SELECT * FROM type::table($table)");
        let entries: Vec<debit::MintQuote> = self
            .db
            .query(statement)
            .bind(("table", self.table.clone()))
            .await
            .map_err(Error::DB)?
            .take(0)
            .map_err(Error::DB)?;
        Ok(entries)
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct ForeignProofEntry {
    id: RecordId,
    proof: cashu::Proof,
    mint_url: cashu::MintUrl,
    mint_pk: secp256k1::PublicKey,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct ForeignOnlineHtlcProofEntry {
    id: RecordId,
    proof: cashu::Proof,
    mint_url: cashu::MintUrl,
    mint_pk: secp256k1::PublicKey,
    hash: Sha256Hash,
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct ForeignOnlineConnectionConfig {
    pub connection: String,
    pub namespace: String,
    pub database: String,
    pub foreigns_table: String,
    pub htlcs_table: String,
}

#[derive(Debug, Clone)]
pub struct ForeignOnlineRepository {
    db: Surreal<Any>,
    foreigns_table: String,
    htlcs_table: String,
}

impl ForeignOnlineRepository {
    pub async fn new(config: ForeignOnlineConnectionConfig) -> SurrealResult<Self> {
        let db_connection = Surreal::<Any>::init();
        db_connection.connect(config.connection).await?;
        db_connection.use_ns(config.namespace).await?;
        db_connection.use_db(config.database).await?;
        Ok(Self {
            db: db_connection,
            foreigns_table: config.foreigns_table,
            htlcs_table: config.htlcs_table,
        })
    }
}

#[async_trait]
impl foreign::OnlineRepository for ForeignOnlineRepository {
    async fn store(
        &self,
        (mint_pk, mint_url): (secp256k1::PublicKey, cashu::MintUrl),
        proofs: Vec<cashu::Proof>,
    ) -> Result<()> {
        let mut entries: Vec<ForeignProofEntry> = Vec::with_capacity(proofs.len());
        for proof in proofs.into_iter() {
            let rid = RecordId::from_table_key(&self.foreigns_table, proof.y()?.to_string());
            entries.push(ForeignProofEntry {
                id: rid,
                proof,
                mint_pk,
                mint_url: mint_url.clone(),
            });
        }
        let _: Vec<ForeignProofEntry> = self
            .db
            .insert(&self.foreigns_table)
            .content(entries)
            .await
            .map_err(Error::DB)?;
        Ok(())
    }

    async fn list(&self) -> Result<Vec<((secp256k1::PublicKey, cashu::MintUrl), cashu::Proof)>> {
        let statement = String::from("SELECT * FROM type::table($table)");
        let entries: Vec<ForeignProofEntry> = self
            .db
            .query(statement)
            .bind(("table", self.foreigns_table.clone()))
            .await
            .map_err(Error::DB)?
            .take(0)
            .map_err(Error::DB)?;
        let mut ret_val = Vec::with_capacity(entries.len());
        for entry in entries {
            let ForeignProofEntry {
                mint_url,
                mint_pk,
                proof,
                ..
            } = entry;
            ret_val.push(((mint_pk, mint_url), proof));
        }
        Ok(ret_val)
    }

    async fn store_htlc(
        &self,
        (mint_pk, mint_url): (secp256k1::PublicKey, cashu::MintUrl),
        hash: Sha256Hash,
        proofs: Vec<cashu::Proof>,
    ) -> Result<()> {
        let mut entries: Vec<ForeignOnlineHtlcProofEntry> = Vec::with_capacity(proofs.len());
        for proof in proofs {
            let id = RecordId::from_table_key(&self.htlcs_table, proof.y()?.to_string());
            let entry = ForeignOnlineHtlcProofEntry {
                hash,
                id,
                proof,
                mint_pk,
                mint_url: mint_url.clone(),
            };
            entries.push(entry);
        }
        let _: Vec<ForeignOnlineHtlcProofEntry> = self
            .db
            .insert(&self.htlcs_table)
            .content(entries)
            .await
            .map_err(Error::DB)?;
        Ok(())
    }

    async fn search_htlc(
        &self,
        hash: &Sha256Hash,
    ) -> Result<Vec<((secp256k1::PublicKey, cashu::MintUrl), cashu::Proof)>> {
        let htlcs: Vec<ForeignOnlineHtlcProofEntry> = self
            .db
            .query("SELECT * FROM type::table($table) WHERE hash = $hash")
            .bind(("table", self.htlcs_table.clone()))
            .bind(("hash", hash.clone()))
            .await
            .map_err(Error::DB)?
            .take(0)
            .map_err(Error::DB)?;
        let ret_val = htlcs
            .into_iter()
            .map(
                |ForeignOnlineHtlcProofEntry {
                     proof,
                     mint_url,
                     mint_pk,
                     ..
                 }| ((mint_pk, mint_url), proof),
            )
            .collect();
        Ok(ret_val)
    }

    async fn remove_htlcs(&self, ys: &[cashu::PublicKey]) -> Result<()> {
        for y in ys {
            let rid = RecordId::from_table_key(&self.htlcs_table, y.to_string());
            let _: Option<ForeignOnlineHtlcProofEntry> =
                self.db.delete(rid).await.map_err(Error::DB)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct ForeignOfflineConnectionConfig {
    pub connection: String,
    pub namespace: String,
    pub database: String,
    pub fps_table: String,
    pub proofs_table: String,
}

#[derive(Debug, Clone)]
pub struct ForeignOfflineRepository {
    db: Surreal<Any>,
    fps_table: String,
    proofs_table: String,
}

impl ForeignOfflineRepository {
    pub async fn new(config: ForeignOfflineConnectionConfig) -> SurrealResult<Self> {
        let db_connection = Surreal::<Any>::init();
        db_connection.connect(config.connection).await?;
        db_connection.use_ns(config.namespace).await?;
        db_connection.use_db(config.database).await?;
        Ok(Self {
            db: db_connection,
            fps_table: config.fps_table,
            proofs_table: config.proofs_table,
        })
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct ForeignFingerprintEntry {
    id: RecordId,
    amount: u64,
    keyset_id: cashu::Id,
    y: cashu::PublicKey,
    c: cashu::PublicKey,
    witness: Option<cashu::Witness>,
    dleq: Option<cashu::ProofDleq>,
    mint_pk: secp256k1::PublicKey,
    mint_url: cashu::MintUrl,
}

#[async_trait]
impl foreign::OfflineRepository for ForeignOfflineRepository {
    async fn store_fps(
        &self,
        (mint_pk, mint_url): (secp256k1::PublicKey, cashu::MintUrl),
        fps: Vec<wire_keys::ProofFingerprint>,
        hash: Vec<Sha256Hash>,
    ) -> Result<()> {
        for (hash, fp) in hash.into_iter().zip(fps.into_iter()) {
            let rid = RecordId::from_table_key(&self.fps_table, hash.to_string());
            let entry = ForeignFingerprintEntry {
                id: rid.clone(),
                amount: fp.amount,
                keyset_id: fp.keyset_id,
                y: fp.y,
                c: fp.c,
                witness: fp.witness,
                dleq: fp.dleq,
                mint_pk,
                mint_url: mint_url.clone(),
            };
            let _: Option<ForeignFingerprintEntry> = self
                .db
                .insert(rid)
                .content(entry)
                .await
                .map_err(Error::DB)?;
        }
        Ok(())
    }

    async fn search_fp(
        &self,
        hash: &Sha256Hash,
    ) -> Result<
        Option<(
            (secp256k1::PublicKey, cashu::MintUrl),
            wire_keys::ProofFingerprint,
        )>,
    > {
        let rid = RecordId::from_table_key(&self.fps_table, hash.to_string());
        let entry: Option<ForeignFingerprintEntry> = self.db.select(rid).await?;
        let Some(entry) = entry else {
            return Ok(None);
        };
        let fp = wire_keys::ProofFingerprint {
            amount: entry.amount,
            keyset_id: entry.keyset_id,
            y: entry.y,
            c: entry.c,
            witness: entry.witness,
            dleq: entry.dleq,
        };
        Ok(Some(((entry.mint_pk, entry.mint_url), fp)))
    }

    async fn remove_fps(&self, ys: &[cashu::PublicKey]) -> Result<()> {
        let _: Vec<ForeignFingerprintEntry> = self
            .db
            .query("DELETE FROM type::table($table) WHERE array::any($ys, y)")
            .bind(("table", self.fps_table.clone()))
            .bind(("ys", ys.to_vec()))
            .await
            .map_err(Error::DB)?
            .take(0)
            .map_err(Error::DB)?;
        Ok(())
    }
    async fn store_proofs(
        &self,
        (mint_pk, mint_url): (secp256k1::PublicKey, cashu::MintUrl),
        proofs: Vec<cashu::Proof>,
    ) -> Result<()> {
        let mut entries: Vec<ForeignProofEntry> = Vec::with_capacity(proofs.len());
        for proof in proofs.into_iter() {
            let rid = RecordId::from_table_key(&self.proofs_table, proof.y()?.to_string());
            let entry = ForeignProofEntry {
                id: rid,
                proof,
                mint_pk,
                mint_url: mint_url.clone(),
            };
            entries.push(entry);
        }
        let _: Vec<ForeignProofEntry> = self.db.insert(&self.proofs_table).content(entries).await?;
        Ok(())
    }

    async fn load_proofs(
        &self,
        (mint_pk, mint_url): &(secp256k1::PublicKey, cashu::MintUrl),
    ) -> Result<Vec<cashu::Proof>> {
        let entries: Vec<ForeignProofEntry> = self
            .db
            .query("SELECT * FROM type::table($table) WHERE mint_url = $mint_url AND mint_pk = $mint_pk")
            .bind(("table", self.proofs_table.clone()))
            .bind(("mint_url", mint_url.clone()))
            .bind(("mint_pk", mint_pk.clone()))
            .await
            .map_err(Error::DB)?
            .take(0)
            .map_err(Error::DB)?;
        let mut ret_val = Vec::with_capacity(entries.len());
        for entry in entries {
            ret_val.push(entry.proof);
        }
        Ok(ret_val)
    }

    async fn remove_proofs(&self, ys: &[cashu::PublicKey]) -> Result<()> {
        let _: Vec<ForeignProofEntry> = self
            .db
            .query("DELETE FROM type::table($table) WHERE array::any($ys, proof.y)")
            .bind(("table", self.proofs_table.clone()))
            .bind(("ys", ys.to_vec()))
            .await
            .map_err(Error::DB)?
            .take(0)
            .map_err(Error::DB)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::debit::Repository;
    use crate::foreign::OfflineRepository;
    use bcr_common::core_tests;
    use bcr_common::core_tests::generate_random_keypair;
    use bcr_wdc_utils::keys::test_utils as keys_utils;
    use bcr_wdc_utils::signatures::test_utils as signatures_test;
    use bitcoin::hashes::Hash;
    use std::str::FromStr;

    async fn init_cred_mem_db() -> CreditRepository {
        let sdb = Surreal::<Any>::init();
        sdb.connect("mem://").await.unwrap();
        sdb.use_ns("test").await.unwrap();
        sdb.use_db("test").await.unwrap();
        CreditRepository {
            db: sdb,
            secrets: "secrets".to_string(),
            signatures: "signatures".to_string(),
            proofs: "proofs".to_string(),
        }
    }

    #[tokio::test]
    async fn list_balance_by_keyset_id_empty() {
        let db = init_cred_mem_db().await;
        let balances = db.list_balance_by_keyset_id().await.unwrap();
        assert_eq!(balances.len(), 0);
    }

    #[tokio::test]
    async fn list_balance_by_keyset_id() {
        let db = init_cred_mem_db().await;

        let (_, keyset) = keys_utils::generate_random_keyset();
        let proofs =
            signatures_test::generate_proofs(&keyset, &[Amount::from(8_u64), Amount::from(4_u64)]);
        db.store_proofs(proofs).await.unwrap();

        let mut expected = vec![(keyset.id, Amount::from(12_u64))];

        let (_, keyset) = keys_utils::generate_random_keyset();
        let proofs =
            signatures_test::generate_proofs(&keyset, &[Amount::from(16_u64), Amount::from(4_u64)]);
        db.store_proofs(proofs).await.unwrap();

        expected.push((keyset.id, Amount::from(20_u64)));

        let mut balances = db.list_balance_by_keyset_id().await.unwrap();

        expected.sort_by(|a, b| a.0.cmp(&b.0));
        balances.sort_by(|a, b| a.0.cmp(&b.0));
        assert_eq!(balances, expected);
    }

    #[tokio::test]
    async fn list_premint_signatures() {
        let db = init_cred_mem_db().await;
        let (_, keyset) = keys_utils::generate_random_keyset();
        let amounts = [Amount::from(8_u64), Amount::from(4_u64)];
        let signatures = signatures_test::generate_signatures(&keyset, &amounts);
        let entry = DBEntrySignatures {
            request_id: Uuid::new_v4(),
            signatures,
        };
        let rid = RecordId::from_table_key(&db.signatures, entry.request_id);
        let _: Option<DBEntrySignatures> = db
            .db
            .insert(rid)
            .content(entry.clone())
            .await
            .expect("insert failed");

        let entries = db.list_premint_signatures().await.unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].request_id, entry.request_id);
        assert_eq!(entries[0].signatures.len(), entry.signatures.len());
    }

    async fn init_deb_mem_db() -> DebitRepository {
        let sdb = Surreal::<Any>::init();
        sdb.connect("mem://").await.unwrap();
        sdb.use_ns("test").await.unwrap();
        sdb.use_db("test").await.unwrap();
        DebitRepository {
            db: sdb,
            table: String::from("test"),
        }
    }

    #[tokio::test]
    async fn test_mint_quote() {
        let db = init_deb_mem_db().await;

        let quote = debit::MintQuote {
            qid: Uuid::new_v4().to_string(),
            ebill_id: core_tests::random_bill_id(),
        };
        db.store_quote(quote.clone()).await.unwrap();

        let list = db.list_quotes().await.unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].qid, quote.qid);

        db.delete_quote(quote.qid.clone()).await.unwrap();
    }

    async fn init_foreignoffline_mem_db() -> ForeignOfflineRepository {
        let sdb = Surreal::<Any>::init();
        sdb.connect("mem://").await.unwrap();
        sdb.use_ns("test").await.unwrap();
        sdb.use_db("test").await.unwrap();
        ForeignOfflineRepository {
            db: sdb,
            fps_table: String::from("fps_table"),
            proofs_table: String::from("proofs_table"),
        }
    }

    #[tokio::test]
    async fn offline_search_fps() {
        let db = init_foreignoffline_mem_db().await;

        let alpha_pk = generate_random_keypair().public_key();
        let alpha = (
            alpha_pk,
            cashu::MintUrl::from_str("http://example.com").unwrap(),
        );
        let y = cashu::PublicKey::from(generate_random_keypair().public_key());
        let c = cashu::PublicKey::from(generate_random_keypair().public_key());
        let fps = vec![
            wire_keys::ProofFingerprint {
                amount: 10,
                keyset_id: cashu::Id::from_bytes(&[1; 33]).unwrap(),
                y,
                c,
                witness: None,
                dleq: None,
            },
            wire_keys::ProofFingerprint {
                amount: 10,
                keyset_id: cashu::Id::from_bytes(&[1; 33]).unwrap(),
                y: cashu::PublicKey::from(generate_random_keypair().public_key()),
                c: cashu::PublicKey::from(generate_random_keypair().public_key()),
                witness: None,
                dleq: None,
            },
        ];
        let hash = vec![
            Sha256Hash::from_slice(&[0u8; 32]).unwrap(),
            Sha256Hash::from_slice(&[1u8; 32]).unwrap(),
        ];
        db.store_fps(alpha.clone(), fps, hash.clone())
            .await
            .unwrap();
        let result = db.search_fp(&hash[0]).await.unwrap();
        assert!(result.is_some());
        let (mint, fp) = result.unwrap();
        assert_eq!(mint.0, alpha.0);
        assert_eq!(fp.y, y);
    }
}
