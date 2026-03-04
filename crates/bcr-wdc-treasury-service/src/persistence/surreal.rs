// ----- standard library imports
// ----- extra library imports
use anyhow::anyhow;
use async_trait::async_trait;
use bcr_common::{cashu, core, wire::keys as wire_keys};
use bcr_wdc_utils::surreal;
use bitcoin::hashes::sha256::Hash as Sha256Hash;
use surrealdb::{
    engine::any::Any, error::Db as SurrealDBError, Error as SurrealError, RecordId,
    Result as SurrealResult, Surreal,
};
use uuid::Uuid;
// ----- local imports
use crate::{
    credit, debit,
    error::{Error, Result},
    foreign, persistence,
};

// ----- end imports

#[derive(Debug, Clone)]
pub struct DebitRepository {
    db: Surreal<Any>,
}

impl DebitRepository {
    const QUOTES_TABLE: &'static str = "mint_quotes";
    const MELTS_TABLE: &'static str = "onchain_melts";
    const MINTS_TABLE: &'static str = "onchain_mints";

    pub async fn new(config: surreal::DBConnConfig) -> SurrealResult<Self> {
        let db_connection = Surreal::<Any>::init();
        db_connection.connect(config.connection).await?;
        db_connection.use_ns(config.namespace).await?;
        db_connection.use_db(config.database).await?;
        Ok(Self { db: db_connection })
    }
}

#[async_trait]
impl persistence::Repository for DebitRepository {
    async fn store_quote(&self, quote: debit::MintQuote) -> Result<()> {
        let rid = RecordId::from_table_key(Self::QUOTES_TABLE, quote.qid.clone());
        let _: Option<debit::MintQuote> = self
            .db
            .insert(rid)
            .content(quote)
            .await
            .map_err(|e| Error::DB(anyhow!(e)))?;
        Ok(())
    }

    async fn update_quote(&self, quote: debit::MintQuote) -> Result<()> {
        let rid = RecordId::from_table_key(Self::QUOTES_TABLE, quote.qid.clone());
        let _: Option<debit::MintQuote> = self
            .db
            .update(rid)
            .content(quote)
            .await
            .map_err(|e| Error::DB(anyhow!(e)))?;
        Ok(())
    }

    async fn list_quotes(&self) -> Result<Vec<debit::MintQuote>> {
        let statement = String::from("SELECT * FROM type::table($table)");
        let entries: Vec<debit::MintQuote> = self
            .db
            .query(statement)
            .bind(("table", Self::QUOTES_TABLE))
            .await
            .map_err(|e| Error::DB(anyhow!(e)))?
            .take(0)
            .map_err(|e| Error::DB(anyhow!(e)))?;
        Ok(entries)
    }

    async fn store_onchain_melt(
        &self,
        quote_id: uuid::Uuid,
        data: debit::OnchainMeltQuote,
    ) -> Result<()> {
        let rid = RecordId::from_table_key(Self::MELTS_TABLE, quote_id);
        let _: Option<debit::OnchainMeltQuote> = self
            .db
            .insert(rid)
            .content(data)
            .await
            .map_err(|e| Error::DB(anyhow!(e)))?;
        Ok(())
    }

    async fn load_onchain_melt(&self, quote_id: uuid::Uuid) -> Result<debit::OnchainMeltQuote> {
        let rid = RecordId::from_table_key(Self::MELTS_TABLE, quote_id);
        let result: Option<debit::OnchainMeltQuote> = self
            .db
            .select(rid)
            .await
            .map_err(|e| Error::DB(anyhow!(e)))?;
        result.ok_or_else(|| Error::RequestIDNotFound(quote_id))
    }

    async fn store_onchain_mint(
        &self,
        quote_id: uuid::Uuid,
        data: debit::ClowderMintQuoteOnchain,
    ) -> Result<()> {
        let rid = RecordId::from_table_key(Self::MINTS_TABLE, quote_id);
        let _: Option<debit::ClowderMintQuoteOnchain> = self
            .db
            .insert(rid)
            .content(data)
            .await
            .map_err(|e| Error::DB(anyhow!(e)))?;
        Ok(())
    }

    async fn load_onchain_mint(
        &self,
        quote_id: uuid::Uuid,
    ) -> Result<debit::ClowderMintQuoteOnchain> {
        let rid = RecordId::from_table_key(Self::MINTS_TABLE, quote_id);
        let result: Option<debit::ClowderMintQuoteOnchain> = self
            .db
            .select(rid)
            .await
            .map_err(|e| Error::DB(anyhow!(e)))?;
        result.ok_or_else(|| Error::RequestIDNotFound(quote_id))
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

#[derive(Debug, Clone)]
pub struct ForeignOnlineRepository {
    db: Surreal<Any>,
}

impl ForeignOnlineRepository {
    const FOREIGNS_TABLE: &'static str = "online-foreigns";
    const HTLCS_TABLE: &'static str = "online-htlcs";

    pub async fn new(config: surreal::DBConnConfig) -> SurrealResult<Self> {
        let db_connection = Surreal::<Any>::init();
        db_connection.connect(config.connection).await?;
        db_connection.use_ns(config.namespace).await?;
        db_connection.use_db(config.database).await?;
        Ok(Self { db: db_connection })
    }
}

////////////////////////////////////////////////////////////////////// MintOp DB
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MintOpDBEntry {
    id: RecordId,
    kid: cashu::Id,
    pub_key: cashu::PublicKey,
    target: cashu::Amount,
    minted: cashu::Amount,
    bill_id: core::BillId,
}

fn convert_to_mintopdbentry(entry: credit::MintOperation, table: &str) -> MintOpDBEntry {
    let credit::MintOperation {
        uid,
        kid,
        pub_key,
        target,
        minted,
        bill_id,
    } = entry;
    let id = RecordId::from_table_key(table, uid);
    MintOpDBEntry {
        id,
        kid,
        pub_key,
        target,
        minted,
        bill_id,
    }
}
impl std::convert::From<MintOpDBEntry> for credit::MintOperation {
    fn from(entry: MintOpDBEntry) -> Self {
        let key = entry.id.key();
        let uid = Uuid::try_from(key.clone()).expect("key is a uuid");
        Self {
            uid,
            kid: entry.kid,
            pub_key: entry.pub_key,
            target: entry.target,
            minted: entry.minted,
            bill_id: entry.bill_id,
        }
    }
}

#[derive(Debug, Clone)]
pub struct DBMintOps {
    db: Surreal<surrealdb::engine::any::Any>,
}

impl DBMintOps {
    const TABLE: &'static str = "mint_ops";

    pub async fn new(cfg: surreal::DBConnConfig) -> SurrealResult<Self> {
        let db_connection = Surreal::<Any>::init();
        db_connection.connect(cfg.connection).await?;
        db_connection.use_ns(cfg.namespace).await?;
        db_connection.use_db(cfg.database).await?;
        Ok(Self { db: db_connection })
    }
}

#[async_trait]
impl credit::MintOpRepository for DBMintOps {
    async fn store(&self, mint_op: credit::MintOperation) -> Result<()> {
        let uid = mint_op.uid;
        let entry = convert_to_mintopdbentry(mint_op, Self::TABLE);
        let res: SurrealResult<Option<MintOpDBEntry>> =
            self.db.insert(&entry.id).content(entry).await;
        match res {
            Ok(..) => Ok(()),
            Err(SurrealError::Db(SurrealDBError::RecordExists { .. })) => {
                Err(Error::InvalidInput(format!("mintop already exist {uid}")))
            }
            Err(e) => Err(Error::DB(anyhow!(e))),
        }
    }

    async fn load(&self, uid: Uuid) -> Result<credit::MintOperation> {
        let rid = RecordId::from_table_key(Self::TABLE, uid);
        let res: SurrealResult<Option<MintOpDBEntry>> = self.db.select(rid.clone()).await;
        match res {
            Ok(Some(entry)) => Ok(credit::MintOperation::from(entry)),
            Ok(None) => Err(Error::RequestIDNotFound(uid)),
            Err(e) => Err(Error::DB(anyhow!(e))),
        }
    }

    async fn list(&self, kid: cashu::Id) -> Result<Vec<credit::MintOperation>> {
        let ops: Vec<MintOpDBEntry> = self
            .db
            .query("SELECT * FROM type::table($table) WHERE kid == $kid")
            .bind(("table", Self::TABLE))
            .bind(("kid", kid))
            .await
            .map_err(|e| Error::DB(anyhow!(e)))?
            .take(0)
            .map_err(|e| Error::DB(anyhow!(e)))?;

        let ops = ops.into_iter().map(credit::MintOperation::from).collect();
        Ok(ops)
    }
    async fn update(&self, uid: Uuid, old: cashu::Amount, new: cashu::Amount) -> Result<()> {
        let rid = RecordId::from_table_key(Self::TABLE, uid);
        let before: Option<MintOpDBEntry> = self
            .db
            .query("UPDATE $rid SET minted = $new WHERE minted == $old RETURN BEFORE")
            .bind(("rid", rid))
            .bind(("new", new))
            .bind(("old", old))
            .await
            .map_err(|e| Error::DB(anyhow!(e)))?
            .take(0)
            .map_err(|e| Error::DB(anyhow!(e)))?;
        let Some(before) = before else {
            return Err(Error::InvalidInput(format!(
                "mintop {uid} and {old} amount not found"
            )));
        };
        debug_assert_eq!(before.minted, old, "Minted amount did not match for {uid}");
        if before.minted != old {
            tracing::error!(
                "Minted amount did not match for mintop {uid}: expected {old}, got {}",
                before.minted,
            );
        }
        Ok(())
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
            let rid = RecordId::from_table_key(Self::FOREIGNS_TABLE, proof.y()?.to_string());
            entries.push(ForeignProofEntry {
                id: rid,
                proof,
                mint_pk,
                mint_url: mint_url.clone(),
            });
        }
        let _: Vec<ForeignProofEntry> = self
            .db
            .insert(Self::FOREIGNS_TABLE)
            .content(entries)
            .await
            .map_err(|e| Error::DB(anyhow!(e)))?;
        Ok(())
    }

    async fn list(&self) -> Result<Vec<((secp256k1::PublicKey, cashu::MintUrl), cashu::Proof)>> {
        let statement = String::from("SELECT * FROM type::table($table)");
        let entries: Vec<ForeignProofEntry> = self
            .db
            .query(statement)
            .bind(("table", Self::FOREIGNS_TABLE))
            .await
            .map_err(|e| Error::DB(anyhow!(e)))?
            .take(0)
            .map_err(|e| Error::DB(anyhow!(e)))?;
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
            let id = RecordId::from_table_key(Self::HTLCS_TABLE, proof.y()?.to_string());
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
            .insert(Self::HTLCS_TABLE)
            .content(entries)
            .await
            .map_err(|e| Error::DB(anyhow!(e)))?;
        Ok(())
    }

    async fn search_htlc(
        &self,
        hash: &Sha256Hash,
    ) -> Result<Vec<((secp256k1::PublicKey, cashu::MintUrl), cashu::Proof)>> {
        let htlcs: Vec<ForeignOnlineHtlcProofEntry> = self
            .db
            .query("SELECT * FROM type::table($table) WHERE hash = $hash")
            .bind(("table", Self::HTLCS_TABLE))
            .bind(("hash", *hash))
            .await
            .map_err(|e| Error::DB(anyhow!(e)))?
            .take(0)
            .map_err(|e| Error::DB(anyhow!(e)))?;
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
            let rid = RecordId::from_table_key(Self::HTLCS_TABLE, y.to_string());
            let _: Option<ForeignOnlineHtlcProofEntry> = self
                .db
                .delete(rid)
                .await
                .map_err(|e| Error::DB(anyhow!(e)))?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct ForeignOfflineRepository {
    db: Surreal<Any>,
}

impl ForeignOfflineRepository {
    const FPS_TABLE: &'static str = "offline-fps";
    const PROOFS_TABLE: &'static str = "offline-proofs";

    pub async fn new(config: surreal::DBConnConfig) -> SurrealResult<Self> {
        let db_connection = Surreal::<Any>::init();
        db_connection.connect(config.connection).await?;
        db_connection.use_ns(config.namespace).await?;
        db_connection.use_db(config.database).await?;
        Ok(Self { db: db_connection })
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
            let rid = RecordId::from_table_key(Self::FPS_TABLE, hash.to_string());
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
                .map_err(|e| Error::DB(anyhow!(e)))?;
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
        let rid = RecordId::from_table_key(Self::FPS_TABLE, hash.to_string());
        let entry: Option<ForeignFingerprintEntry> = self
            .db
            .select(rid)
            .await
            .map_err(|e| Error::DB(anyhow!(e)))?;
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
            .bind(("table", Self::FPS_TABLE))
            .bind(("ys", ys.to_vec()))
            .await
            .map_err(|e| Error::DB(anyhow!(e)))?
            .take(0)
            .map_err(|e| Error::DB(anyhow!(e)))?;
        Ok(())
    }
    async fn store_proofs(
        &self,
        (mint_pk, mint_url): (secp256k1::PublicKey, cashu::MintUrl),
        proofs: Vec<cashu::Proof>,
    ) -> Result<()> {
        let mut entries: Vec<ForeignProofEntry> = Vec::with_capacity(proofs.len());
        for proof in proofs.into_iter() {
            let rid = RecordId::from_table_key(Self::PROOFS_TABLE, proof.y()?.to_string());
            let entry = ForeignProofEntry {
                id: rid,
                proof,
                mint_pk,
                mint_url: mint_url.clone(),
            };
            entries.push(entry);
        }
        let _: Vec<ForeignProofEntry> = self
            .db
            .insert(Self::PROOFS_TABLE)
            .content(entries)
            .await
            .map_err(|e| Error::DB(anyhow!(e)))?;
        Ok(())
    }

    async fn load_proofs(
        &self,
        (mint_pk, mint_url): &(secp256k1::PublicKey, cashu::MintUrl),
    ) -> Result<Vec<cashu::Proof>> {
        let entries: Vec<ForeignProofEntry> = self
            .db
            .query("SELECT * FROM type::table($table) WHERE mint_url = $mint_url AND mint_pk = $mint_pk")
            .bind(("table", Self::PROOFS_TABLE))
            .bind(("mint_url", mint_url.clone()))
            .bind(("mint_pk", *mint_pk))
            .await
            .map_err(|e| Error::DB(anyhow!(e)))?
            .take(0)
            .map_err(|e| Error::DB(anyhow!(e)))?;
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
            .bind(("table", Self::PROOFS_TABLE))
            .bind(("ys", ys.to_vec()))
            .await
            .map_err(|e| Error::DB(anyhow!(e)))?
            .take(0)
            .map_err(|e| Error::DB(anyhow!(e)))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::credit::MintOpRepository;
    use crate::foreign::OfflineRepository;
    use crate::persistence::Repository;
    use bcr_common::core_tests;
    use bitcoin::hashes::Hash;
    use std::str::FromStr;

    async fn init_debit_mem_db() -> DebitRepository {
        let sdb = Surreal::<Any>::init();
        sdb.connect("mem://").await.unwrap();
        sdb.use_ns("test").await.unwrap();
        sdb.use_db("test").await.unwrap();
        DebitRepository { db: sdb }
    }

    #[tokio::test]
    async fn test_mint_quote() {
        let db = init_debit_mem_db().await;

        let quote = debit::MintQuote {
            qid: Uuid::new_v4().to_string(),
            ebill_id: core_tests::random_bill_id(),
            clowder_qid: Uuid::new_v4(),
            mint_complete: false,
        };
        db.store_quote(quote.clone()).await.unwrap();

        let list = db.list_quotes().await.unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].qid, quote.qid);

        db.update_quote(quote).await.unwrap();
    }

    async fn init_foreignoffline_mem_db() -> ForeignOfflineRepository {
        let sdb = Surreal::<Any>::init();
        sdb.connect("mem://").await.unwrap();
        sdb.use_ns("test").await.unwrap();
        sdb.use_db("test").await.unwrap();
        ForeignOfflineRepository { db: sdb }
    }

    #[tokio::test]
    async fn offline_search_fps() {
        let db = init_foreignoffline_mem_db().await;

        let alpha_pk = core_tests::generate_random_keypair().public_key();
        let alpha = (
            alpha_pk,
            cashu::MintUrl::from_str("http://example.com").unwrap(),
        );
        let y = cashu::PublicKey::from(core_tests::generate_random_keypair().public_key());
        let c = cashu::PublicKey::from(core_tests::generate_random_keypair().public_key());
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
                y: cashu::PublicKey::from(core_tests::generate_random_keypair().public_key()),
                c: cashu::PublicKey::from(core_tests::generate_random_keypair().public_key()),
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

    async fn init_mintops_mem_db() -> DBMintOps {
        let sdb = Surreal::<Any>::init();
        sdb.connect("mem://").await.unwrap();
        sdb.use_ns("test").await.unwrap();
        sdb.use_db("test").await.unwrap();
        DBMintOps { db: sdb }
    }

    #[tokio::test]
    async fn store_mintop() {
        let db = init_mintops_mem_db().await;
        let keys = core_tests::generate_random_ecash_keyset();
        let kid = keys.0.id;
        let kp = core_tests::generate_random_keypair();
        let op = credit::MintOperation {
            uid: Uuid::new_v4(),
            kid,
            pub_key: kp.public_key().into(),
            target: cashu::Amount::ZERO,
            minted: cashu::Amount::ZERO,
            bill_id: bcr_common::core_tests::random_bill_id(),
        };
        db.store(op).await.unwrap();
    }
    #[tokio::test]
    async fn store_mintop_twice() {
        let db = init_mintops_mem_db().await;
        let keys = core_tests::generate_random_ecash_keyset();
        let kid = keys.0.id;
        let kp = core_tests::generate_random_keypair();
        let op = credit::MintOperation {
            uid: Uuid::new_v4(),
            kid,
            pub_key: kp.public_key().into(),
            target: cashu::Amount::ZERO,
            minted: cashu::Amount::ZERO,
            bill_id: bcr_common::core_tests::random_bill_id(),
        };
        db.store(op.clone()).await.unwrap();
        let res = db.store(op).await;
        assert!(matches!(res, Err(Error::InvalidInput(_))));
    }

    #[tokio::test]
    async fn load_mintop() {
        let db = init_mintops_mem_db().await;
        let keys = core_tests::generate_random_ecash_keyset();
        let kid = keys.0.id;
        let kp = core_tests::generate_random_keypair();
        let op = credit::MintOperation {
            uid: Uuid::new_v4(),
            kid,
            pub_key: kp.public_key().into(),
            target: cashu::Amount::ZERO,
            minted: cashu::Amount::ZERO,
            bill_id: bcr_common::core_tests::random_bill_id(),
        };
        db.store(op.clone()).await.unwrap();
        let res = db.load(op.uid).await.unwrap();
        assert_eq!(res.kid, kid);
        assert_eq!(res.pub_key, kp.public_key().into());
    }

    #[tokio::test]
    async fn update_mintop() {
        let db = init_mintops_mem_db().await;
        let keys = core_tests::generate_random_ecash_keyset();
        let kid = keys.0.id;
        let kp = core_tests::generate_random_keypair();
        let op = credit::MintOperation {
            uid: Uuid::new_v4(),
            kid,
            pub_key: kp.public_key().into(),
            target: cashu::Amount::ZERO,
            minted: cashu::Amount::ZERO,
            bill_id: bcr_common::core_tests::random_bill_id(),
        };
        db.store(op.clone()).await.unwrap();
        db.update(op.uid, cashu::Amount::ZERO, cashu::Amount::from(100u64))
            .await
            .unwrap();
        let res = db.load(op.uid).await.unwrap();
        assert_eq!(res.kid, kid);
        assert_eq!(res.minted, cashu::Amount::from(100u64));
    }

    #[tokio::test]
    async fn list_mintops() {
        let db = init_mintops_mem_db().await;
        let keys = core_tests::generate_random_ecash_keyset();
        let kid = keys.0.id;
        let kp = core_tests::generate_random_keypair();
        let op1 = credit::MintOperation {
            uid: Uuid::new_v4(),
            kid,
            pub_key: kp.public_key().into(),
            target: cashu::Amount::ZERO,
            minted: cashu::Amount::ZERO,
            bill_id: bcr_common::core_tests::random_bill_id(),
        };
        db.store(op1.clone()).await.unwrap();
        let op2 = credit::MintOperation {
            uid: Uuid::new_v4(),
            kid,
            pub_key: kp.public_key().into(),
            target: cashu::Amount::ZERO,
            minted: cashu::Amount::ZERO,
            bill_id: bcr_common::core_tests::random_bill_id(),
        };
        db.store(op2.clone()).await.unwrap();
        let res = db.list(kid).await.unwrap();
        assert_eq!(res.len(), 2);
        let rids: Vec<_> = res.iter().map(|op| op.uid).collect();
        assert!(rids.contains(&op1.uid));
        assert!(rids.contains(&op2.uid));
    }
}
