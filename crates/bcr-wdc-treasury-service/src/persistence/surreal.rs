use std::str::FromStr;

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
    ebill,
    error::{Error, Result},
    foreign, onchain, vault,
};

// ----- end imports

////////////////////////////////////////////////////////////////// SurrealDB-safe wrappers

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct BlindedMessageDB {
    amount: cashu::Amount,
    keyset_id: cashu::Id,
    blinded_secret: cashu::PublicKey,
    witness: Option<cashu::Witness>,
}
impl From<cashu::BlindedMessage> for BlindedMessageDB {
    fn from(m: cashu::BlindedMessage) -> Self {
        Self {
            amount: m.amount,
            keyset_id: m.keyset_id,
            blinded_secret: m.blinded_secret,
            witness: m.witness,
        }
    }
}
impl From<BlindedMessageDB> for cashu::BlindedMessage {
    fn from(m: BlindedMessageDB) -> Self {
        Self {
            amount: m.amount,
            keyset_id: m.keyset_id,
            blinded_secret: m.blinded_secret,
            witness: m.witness,
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct BlindSignatureDB {
    amount: cashu::Amount,
    keyset_id: cashu::Id,
    c: cashu::PublicKey,
    dleq: Option<cashu::BlindSignatureDleq>,
}
impl From<cashu::BlindSignature> for BlindSignatureDB {
    fn from(s: cashu::BlindSignature) -> Self {
        Self {
            amount: s.amount,
            keyset_id: s.keyset_id,
            c: s.c,
            dleq: s.dleq,
        }
    }
}
impl From<BlindSignatureDB> for cashu::BlindSignature {
    fn from(s: BlindSignatureDB) -> Self {
        Self {
            amount: s.amount,
            keyset_id: s.keyset_id,
            c: s.c,
            dleq: s.dleq,
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "status")]
enum MintStatusDB {
    Pending { blinds: Vec<BlindedMessageDB> },
    Paid { signatures: Vec<BlindSignatureDB> },
    Expired,
}
impl From<onchain::MintStatus> for MintStatusDB {
    fn from(s: onchain::MintStatus) -> Self {
        match s {
            onchain::MintStatus::Pending { blinds } => MintStatusDB::Pending {
                blinds: blinds.into_iter().map(Into::into).collect(),
            },
            onchain::MintStatus::Paid { signatures } => MintStatusDB::Paid {
                signatures: signatures.into_iter().map(Into::into).collect(),
            },
            onchain::MintStatus::Expired => MintStatusDB::Expired,
        }
    }
}
impl From<MintStatusDB> for onchain::MintStatus {
    fn from(s: MintStatusDB) -> Self {
        match s {
            MintStatusDB::Pending { blinds } => onchain::MintStatus::Pending {
                blinds: blinds.into_iter().map(Into::into).collect(),
            },
            MintStatusDB::Paid { signatures } => onchain::MintStatus::Paid {
                signatures: signatures.into_iter().map(Into::into).collect(),
            },
            MintStatusDB::Expired => onchain::MintStatus::Expired,
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct OnChainMintOperationDB {
    qid: Uuid,
    kid: cashu::Id,
    recipient: bitcoin::Address<bitcoin::address::NetworkUnchecked>,
    target: bitcoin::Amount,
    expiry: crate::TStamp,
    status: MintStatusDB,
}
impl From<onchain::OnChainMintOperation> for OnChainMintOperationDB {
    fn from(op: onchain::OnChainMintOperation) -> Self {
        Self {
            qid: op.qid,
            kid: op.kid,
            recipient: op.recipient,
            target: op.target,
            expiry: op.expiry,
            status: op.status.into(),
        }
    }
}
impl From<OnChainMintOperationDB> for onchain::OnChainMintOperation {
    fn from(op: OnChainMintOperationDB) -> Self {
        Self {
            qid: op.qid,
            kid: op.kid,
            recipient: op.recipient,
            target: op.target,
            expiry: op.expiry,
            status: op.status.into(),
        }
    }
}

///////////////////////////////////////////////////////////////////////////////// OnChain DB
#[derive(Debug, Clone)]
pub struct DBOnChain {
    db: Surreal<Any>,
}

impl DBOnChain {
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
impl onchain::Repository for DBOnChain {
    async fn store_quote(&self, quote: onchain::MintQuote) -> Result<()> {
        let rid = RecordId::from_table_key(Self::QUOTES_TABLE, quote.qid.clone());
        let _: Option<onchain::MintQuote> = self
            .db
            .insert(rid)
            .content(quote)
            .await
            .map_err(|e| Error::DB(anyhow!(e)))?;
        Ok(())
    }

    async fn update_quote(&self, quote: onchain::MintQuote) -> Result<()> {
        let rid = RecordId::from_table_key(Self::QUOTES_TABLE, quote.qid.clone());
        let _: Option<onchain::MintQuote> = self
            .db
            .update(rid)
            .content(quote)
            .await
            .map_err(|e| Error::DB(anyhow!(e)))?;
        Ok(())
    }

    async fn list_quotes(&self) -> Result<Vec<onchain::MintQuote>> {
        let statement = String::from("SELECT * FROM type::table($table)");
        let entries: Vec<onchain::MintQuote> = self
            .db
            .query(statement)
            .bind(("table", Self::QUOTES_TABLE))
            .await
            .map_err(|e| Error::DB(anyhow!(e)))?
            .take(0)
            .map_err(|e| Error::DB(anyhow!(e)))?;
        Ok(entries)
    }

    async fn store_onchain_mintop(&self, op: onchain::OnChainMintOperation) -> Result<()> {
        let rid = RecordId::from_table_key(Self::MINTS_TABLE, op.qid);
        let db_op = OnChainMintOperationDB::from(op);
        let _: Option<OnChainMintOperationDB> = self
            .db
            .insert(rid)
            .content(db_op)
            .await
            .map_err(|e| Error::DB(anyhow!(e)))?;
        Ok(())
    }

    async fn load_onchain_mintop(&self, qid: Uuid) -> Result<onchain::OnChainMintOperation> {
        let rid = RecordId::from_table_key(Self::MINTS_TABLE, qid);
        let result: Option<OnChainMintOperationDB> = self
            .db
            .select(rid)
            .await
            .map_err(|e| Error::DB(anyhow!(e)))?;
        result
            .map(Into::into)
            .ok_or_else(|| Error::ResourceNotFound(qid.to_string()))
    }

    async fn list_onchain_pending_mintops(&self) -> Result<Vec<Uuid>> {
        let entry: Vec<Uuid> = self
            .db
            .query("SELECT qid FROM type::table($table) WHERE status.status == $status")
            .bind(("table", Self::MINTS_TABLE))
            .bind(("status", onchain::MintStatusDiscriminants::Pending))
            .await
            .map_err(|e| Error::DB(anyhow!(e)))?
            .take("qid")
            .map_err(|e| Error::DB(anyhow!(e)))?;
        Ok(entry)
    }

    async fn update_onchain_mintop_status(
        &self,
        qid: Uuid,
        status: onchain::MintStatus,
    ) -> Result<()> {
        let rid = RecordId::from_table_key(Self::MINTS_TABLE, qid);
        let db_status = MintStatusDB::from(status);
        let entry: Option<OnChainMintOperationDB> = self
            .db
            .query("UPDATE $rid SET status = $status")
            .bind(("rid", rid))
            .bind(("status", db_status))
            .await
            .map_err(|e| Error::DB(anyhow!(e)))?
            .take(0)
            .map_err(|e| Error::DB(anyhow!(e)))?;
        if entry.is_none() {
            return Err(Error::ResourceNotFound(qid.to_string()));
        }
        Ok(())
    }

    async fn store_onchain_meltop(&self, op: onchain::OnchainMeltOperation) -> Result<()> {
        let rid = RecordId::from_table_key(Self::MELTS_TABLE, op.qid);
        let _: Option<onchain::OnchainMeltOperation> = self
            .db
            .insert(rid)
            .content(op)
            .await
            .map_err(|e| Error::DB(anyhow!(e)))?;
        Ok(())
    }
    async fn load_onchain_meltop(&self, qid: Uuid) -> Result<onchain::OnchainMeltOperation> {
        let rid = RecordId::from_table_key(Self::MELTS_TABLE, qid);
        let result: Option<onchain::OnchainMeltOperation> = self
            .db
            .select(rid)
            .await
            .map_err(|e| Error::DB(anyhow!(e)))?;
        result.ok_or_else(|| Error::ResourceNotFound(qid.to_string()))
    }
    async fn update_onchain_meltop_status(
        &self,
        qid: Uuid,
        status: onchain::MeltStatus,
    ) -> Result<()> {
        let rid = RecordId::from_table_key(Self::MELTS_TABLE, qid);
        let entry: Option<onchain::OnchainMeltOperation> = self
            .db
            .query("UPDATE $rid SET status = $status")
            .bind(("rid", rid))
            .bind(("status", status))
            .await
            .map_err(|e| Error::DB(anyhow!(e)))?
            .take(0)
            .map_err(|e| Error::DB(anyhow!(e)))?;
        if entry.is_none() {
            return Err(Error::ResourceNotFound(qid.to_string()));
        }
        Ok(())
    }
}

///////////////////////////////////////////////////////////////////////////////// Ebill DB
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct EbillMintOpDBEntry {
    id: RecordId,
    kid: cashu::Id,
    pub_key: cashu::PublicKey,
    target: cashu::Amount,
    minted: cashu::Amount,
    bill_id: core::BillId,
}

fn convert_to_ebillmintopdbentry(entry: ebill::MintOperation, table: &str) -> EbillMintOpDBEntry {
    let ebill::MintOperation {
        uid,
        kid,
        pub_key,
        target,
        minted,
        bill_id,
    } = entry;
    let id = RecordId::from_table_key(table, uid);
    EbillMintOpDBEntry {
        id,
        kid,
        pub_key,
        target,
        minted,
        bill_id,
    }
}

impl std::convert::From<EbillMintOpDBEntry> for ebill::MintOperation {
    fn from(entry: EbillMintOpDBEntry) -> Self {
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
pub struct DBEbill {
    db: Surreal<surrealdb::engine::any::Any>,
}

impl DBEbill {
    const MINT_OPS: &'static str = "mint_ops";

    pub async fn new(cfg: surreal::DBConnConfig) -> SurrealResult<Self> {
        let db_connection = Surreal::<Any>::init();
        db_connection.connect(cfg.connection).await?;
        db_connection.use_ns(cfg.namespace).await?;
        db_connection.use_db(cfg.database).await?;
        Ok(Self { db: db_connection })
    }
}

#[async_trait]
impl ebill::Repository for DBEbill {
    async fn mint_store(&self, mint_op: ebill::MintOperation) -> Result<()> {
        let uid = mint_op.uid;
        let entry = convert_to_ebillmintopdbentry(mint_op, Self::MINT_OPS);
        let res: SurrealResult<Option<EbillMintOpDBEntry>> =
            self.db.insert(&entry.id).content(entry).await;
        match res {
            Ok(..) => Ok(()),
            Err(SurrealError::Db(SurrealDBError::RecordExists { .. })) => {
                Err(Error::InvalidInput(format!("mintop already exist {uid}")))
            }
            Err(e) => Err(Error::DB(anyhow!(e))),
        }
    }

    async fn mint_load(&self, uid: Uuid) -> Result<ebill::MintOperation> {
        let rid = RecordId::from_table_key(Self::MINT_OPS, uid);
        let res: SurrealResult<Option<EbillMintOpDBEntry>> = self.db.select(rid).await;
        match res {
            Ok(Some(entry)) => Ok(ebill::MintOperation::from(entry)),
            Ok(None) => Err(Error::ResourceNotFound(uid.to_string())),
            Err(e) => Err(Error::DB(anyhow!(e))),
        }
    }

    async fn mint_list(&self, kid: cashu::Id) -> Result<Vec<ebill::MintOperation>> {
        let ops: Vec<EbillMintOpDBEntry> = self
            .db
            .query("SELECT * FROM type::table($table) WHERE kid == $kid")
            .bind(("table", Self::MINT_OPS))
            .bind(("kid", kid))
            .await
            .map_err(|e| Error::DB(anyhow!(e)))?
            .take(0)
            .map_err(|e| Error::DB(anyhow!(e)))?;

        let ops = ops.into_iter().map(ebill::MintOperation::from).collect();
        Ok(ops)
    }
    async fn mint_update_field(
        &self,
        uid: Uuid,
        old: cashu::Amount,
        new: cashu::Amount,
    ) -> Result<()> {
        let rid = RecordId::from_table_key(Self::MINT_OPS, uid);
        let before: Option<EbillMintOpDBEntry> = self
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
                "mintop {uid}: amount did not match expected {old}, got {}",
                before.minted,
            );
        }
        Ok(())
    }
}

//////////////////////////////////////////////////////////////////////////////// Foreign DB
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct ForeignProofDBEntry {
    id: RecordId,
    proof: cashu::Proof,
    mint_id: secp256k1::PublicKey,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct ForeignOnlineHtlcProofDBEntry {
    id: RecordId,
    proof: cashu::Proof,
    mint_id: secp256k1::PublicKey,
    hash: Sha256Hash,
}

#[derive(Debug, Clone)]
pub struct DBForeignOnline {
    db: Surreal<Any>,
}

impl DBForeignOnline {
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
#[async_trait]
impl foreign::OnlineRepository for DBForeignOnline {
    async fn store(&self, mint_id: secp256k1::PublicKey, proofs: Vec<cashu::Proof>) -> Result<()> {
        let mut entries: Vec<ForeignProofDBEntry> = Vec::with_capacity(proofs.len());
        for proof in proofs.into_iter() {
            let rid = RecordId::from_table_key(Self::FOREIGNS_TABLE, proof.y()?.to_string());
            entries.push(ForeignProofDBEntry {
                id: rid,
                proof,
                mint_id,
            });
        }
        let _: Vec<ForeignProofDBEntry> = self
            .db
            .insert(Self::FOREIGNS_TABLE)
            .content(entries)
            .await
            .map_err(|e| Error::DB(anyhow!(e)))?;
        Ok(())
    }

    async fn list(&self) -> Result<Vec<(secp256k1::PublicKey, cashu::Proof)>> {
        let statement = String::from("SELECT * FROM type::table($table)");
        let entries: Vec<ForeignProofDBEntry> = self
            .db
            .query(statement)
            .bind(("table", Self::FOREIGNS_TABLE))
            .await
            .map_err(|e| Error::DB(anyhow!(e)))?
            .take(0)
            .map_err(|e| Error::DB(anyhow!(e)))?;
        let mut ret_val = Vec::with_capacity(entries.len());
        for entry in entries {
            let ForeignProofDBEntry { mint_id, proof, .. } = entry;
            ret_val.push((mint_id, proof));
        }
        Ok(ret_val)
    }

    async fn store_htlc(
        &self,
        mint_id: secp256k1::PublicKey,
        hash: Sha256Hash,
        proofs: Vec<cashu::Proof>,
    ) -> Result<()> {
        let mut entries: Vec<ForeignOnlineHtlcProofDBEntry> = Vec::with_capacity(proofs.len());
        for proof in proofs {
            let id = RecordId::from_table_key(Self::HTLCS_TABLE, proof.y()?.to_string());
            let entry = ForeignOnlineHtlcProofDBEntry {
                hash,
                id,
                proof,
                mint_id,
            };
            entries.push(entry);
        }
        let _: Vec<ForeignOnlineHtlcProofDBEntry> = self
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
    ) -> Result<Vec<(secp256k1::PublicKey, cashu::Proof)>> {
        let htlcs: Vec<ForeignOnlineHtlcProofDBEntry> = self
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
            .map(|ForeignOnlineHtlcProofDBEntry { proof, mint_id, .. }| (mint_id, proof))
            .collect();
        Ok(ret_val)
    }

    async fn remove_htlcs(&self, ys: &[cashu::PublicKey]) -> Result<()> {
        for y in ys {
            let rid = RecordId::from_table_key(Self::HTLCS_TABLE, y.to_string());
            let _: Option<ForeignOnlineHtlcProofDBEntry> = self
                .db
                .delete(rid)
                .await
                .map_err(|e| Error::DB(anyhow!(e)))?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct DBForeignOffline {
    db: Surreal<Any>,
}

impl DBForeignOffline {
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
struct ForeignFingerprintDBEntry {
    id: RecordId,
    amount: u64,
    keyset_id: cashu::Id,
    y: cashu::PublicKey,
    c: cashu::PublicKey,
    witness: Option<cashu::Witness>,
    dleq: Option<cashu::ProofDleq>,
    mint_id: secp256k1::PublicKey,
}

#[async_trait]
impl foreign::OfflineRepository for DBForeignOffline {
    async fn store_fps(
        &self,
        mint_id: secp256k1::PublicKey,
        fps: Vec<wire_keys::ProofFingerprint>,
        hash: Vec<Sha256Hash>,
    ) -> Result<()> {
        for (hash, fp) in hash.into_iter().zip(fps) {
            let rid = RecordId::from_table_key(Self::FPS_TABLE, hash.to_string());
            let entry = ForeignFingerprintDBEntry {
                id: rid.clone(),
                amount: fp.amount,
                keyset_id: fp.keyset_id,
                y: fp.y,
                c: fp.c,
                witness: fp.witness,
                dleq: fp.dleq,
                mint_id,
            };
            let _: Option<ForeignFingerprintDBEntry> = self
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
    ) -> Result<Option<(secp256k1::PublicKey, wire_keys::ProofFingerprint)>> {
        let rid = RecordId::from_table_key(Self::FPS_TABLE, hash.to_string());
        let entry: Option<ForeignFingerprintDBEntry> = self
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
        Ok(Some((entry.mint_id, fp)))
    }

    async fn remove_fps(&self, ys: &[cashu::PublicKey]) -> Result<()> {
        let _: Vec<ForeignFingerprintDBEntry> = self
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
        mint_id: secp256k1::PublicKey,
        proofs: Vec<cashu::Proof>,
    ) -> Result<()> {
        let mut entries: Vec<ForeignProofDBEntry> = Vec::with_capacity(proofs.len());
        for proof in proofs.into_iter() {
            let rid = RecordId::from_table_key(Self::PROOFS_TABLE, proof.y()?.to_string());
            let entry = ForeignProofDBEntry {
                id: rid,
                proof,
                mint_id,
            };
            entries.push(entry);
        }
        let _: Vec<ForeignProofDBEntry> = self
            .db
            .insert(Self::PROOFS_TABLE)
            .content(entries)
            .await
            .map_err(|e| Error::DB(anyhow!(e)))?;
        Ok(())
    }

    async fn load_proofs(&self, mint_id: secp256k1::PublicKey) -> Result<Vec<cashu::Proof>> {
        let entries: Vec<ForeignProofDBEntry> = self
            .db
            .query("SELECT * FROM type::table($table) WHERE mint_id = $mint_id")
            .bind(("table", Self::PROOFS_TABLE))
            .bind(("mint_id", mint_id))
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
        let _: Vec<ForeignProofDBEntry> = self
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

/////////////////////////////////////////////////////////////////////////////// Vault DB
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct VaultProofDBEntry {
    id: RecordId,
    proof: cashu::Proof,
}
impl std::convert::From<VaultProofDBEntry> for cashu::Proof {
    fn from(entry: VaultProofDBEntry) -> Self {
        entry.proof
    }
}

#[derive(Debug, Clone)]
pub struct DBVault {
    db: Surreal<Any>,
}

impl DBVault {
    const PROOFS_TABLE: &'static str = "vault_proofs";

    pub async fn new(config: surreal::DBConnConfig) -> SurrealResult<Self> {
        let db_connection = Surreal::<Any>::init();
        db_connection.connect(config.connection).await?;
        db_connection.use_ns(config.namespace).await?;
        db_connection.use_db(config.database).await?;
        Ok(Self { db: db_connection })
    }
}

#[async_trait]
impl vault::Repository for DBVault {
    async fn store_proofs(&self, proofs: Vec<cashu::Proof>) -> Result<()> {
        let mut entries: Vec<VaultProofDBEntry> = Vec::with_capacity(proofs.len());
        for proof in proofs {
            let y = proof.y()?;
            let rid = RecordId::from_table_key(Self::PROOFS_TABLE, y.to_string());
            let entry = VaultProofDBEntry { id: rid, proof };
            entries.push(entry);
        }
        let _: Vec<VaultProofDBEntry> = self
            .db
            .insert(Self::PROOFS_TABLE)
            .content(entries)
            .await
            .map_err(|e| Error::DB(anyhow!(e)))?;
        Ok(())
    }

    async fn load_proofs(&self, ys: Vec<cashu::PublicKey>) -> Result<Vec<cashu::Proof>> {
        let mut proofs = Vec::with_capacity(ys.len());
        for y in ys {
            let rid = RecordId::from_table_key(Self::PROOFS_TABLE, y.to_string());
            let entry: Option<VaultProofDBEntry> = self
                .db
                .select(rid)
                .await
                .map_err(|e| Error::DB(anyhow!(e)))?;
            if let Some(entry) = entry {
                proofs.push(entry.proof);
            }
        }
        Ok(proofs)
    }

    async fn list_ys(&self) -> Result<Vec<cashu::PublicKey>> {
        let rids: Vec<RecordId> = self
            .db
            .query("SELECT VALUE id FROM type::table($table)")
            .bind(("table", Self::PROOFS_TABLE))
            .await
            .map_err(|e| Error::DB(anyhow!(e)))?
            .take(0)
            .map_err(|e| Error::DB(anyhow!(e)))?;
        let mut ys = Vec::with_capacity(rids.len());
        for rid in rids {
            let y = cashu::PublicKey::from_str(&rid.key().to_string())
                .map_err(|e| Error::DB(anyhow!(e)))?;
            ys.push(y);
        }
        Ok(ys)
    }

    async fn delete_proofs(&self, ys: &[cashu::PublicKey]) -> Result<()> {
        for y in ys {
            let rid = RecordId::from_table_key(Self::PROOFS_TABLE, y.to_string());
            let _: Option<VaultProofDBEntry> = self
                .db
                .delete(rid)
                .await
                .map_err(|e| Error::DB(anyhow!(e)))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        ebill::Repository as CreditRepo, foreign::OfflineRepository,
        onchain::Repository as DebitRepo, vault::Repository as VaultRepo,
    };
    use bcr_common::{core, core_tests};
    use bcr_wdc_utils::signatures::test_utils as signature_tests;
    use bitcoin::hashes::Hash;
    use std::str::FromStr;

    async fn init_debit_mem_db() -> DBOnChain {
        let sdb = Surreal::<Any>::init();
        sdb.connect("mem://").await.unwrap();
        sdb.use_ns("test").await.unwrap();
        sdb.use_db("test").await.unwrap();
        DBOnChain { db: sdb }
    }

    #[tokio::test]
    async fn test_mint_quote() {
        let db = init_debit_mem_db().await;
        let quote = onchain::MintQuote {
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

    #[tokio::test]
    async fn update_onchain_mintop_status() {
        let db = init_debit_mem_db().await;
        let keys = core_tests::generate_random_ecash_keyset();
        let kid = keys.0.id;
        let amounts = vec![cashu::Amount::from(100u64)];
        let blinds = signature_tests::generate_blinds(kid, &amounts)
            .into_iter()
            .map(|(blind, _, _)| blind)
            .collect();
        let op = onchain::OnChainMintOperation {
            qid: Uuid::new_v4(),
            kid,
            target: bitcoin::Amount::ZERO,
            recipient: bitcoin::Address::from_str("n28b7b8HZcrBqeabbjwGRbo8q9JLcusYFC").unwrap(),
            expiry: chrono::Utc::now() + chrono::Duration::hours(1),
            status: onchain::MintStatus::Pending { blinds },
        };
        db.store_onchain_mintop(op.clone()).await.unwrap();
        let signatures = core_tests::generate_ecash_signatures(&keys.1, &amounts);
        let status = onchain::MintStatus::Paid { signatures };
        db.update_onchain_mintop_status(op.qid, status)
            .await
            .unwrap();
        let res = db.load_onchain_mintop(op.qid).await.unwrap();
        assert!(matches!(res.status, onchain::MintStatus::Paid { .. }));
    }

    #[tokio::test]
    async fn list_onchain_pending_mintops() {
        let db = init_debit_mem_db().await;
        let keys = core_tests::generate_random_ecash_keyset();
        let kid = keys.0.id;
        let amounts = vec![cashu::Amount::from(100u64)];
        let blinds = signature_tests::generate_blinds(kid, &amounts)
            .into_iter()
            .map(|(blind, _, _)| blind)
            .collect();
        let op = onchain::OnChainMintOperation {
            qid: Uuid::new_v4(),
            kid,
            target: bitcoin::Amount::ZERO,
            recipient: bitcoin::Address::from_str("n28b7b8HZcrBqeabbjwGRbo8q9JLcusYFC").unwrap(),
            expiry: chrono::Utc::now() + chrono::Duration::hours(1),
            status: onchain::MintStatus::Pending { blinds },
        };
        db.store_onchain_mintop(op.clone()).await.unwrap();
        let pending = db.list_onchain_pending_mintops().await.unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0], op.qid);
    }

    async fn init_foreignoffline_mem_db() -> DBForeignOffline {
        let sdb = Surreal::<Any>::init();
        sdb.connect("mem://").await.unwrap();
        sdb.use_ns("test").await.unwrap();
        sdb.use_db("test").await.unwrap();
        DBForeignOffline { db: sdb }
    }

    #[tokio::test]
    async fn offline_search_fps() {
        let db = init_foreignoffline_mem_db().await;

        let alpha_id = core::generate_random_keypair().public_key();
        let y = cashu::PublicKey::from(core::generate_random_keypair().public_key());
        let c = cashu::PublicKey::from(core::generate_random_keypair().public_key());
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
                y: cashu::PublicKey::from(core::generate_random_keypair().public_key()),
                c: cashu::PublicKey::from(core::generate_random_keypair().public_key()),
                witness: None,
                dleq: None,
            },
        ];
        let hash = vec![
            Sha256Hash::from_slice(&[0u8; 32]).unwrap(),
            Sha256Hash::from_slice(&[1u8; 32]).unwrap(),
        ];
        db.store_fps(alpha_id.clone(), fps, hash.clone())
            .await
            .unwrap();
        let result = db.search_fp(&hash[0]).await.unwrap();
        assert!(result.is_some());
        let (mint, fp) = result.unwrap();
        assert_eq!(mint, alpha_id);
        assert_eq!(fp.y, y);
    }

    async fn init_credit_mem_db() -> DBEbill {
        let sdb = Surreal::<Any>::init();
        sdb.connect("mem://").await.unwrap();
        sdb.use_ns("test").await.unwrap();
        sdb.use_db("test").await.unwrap();
        DBEbill { db: sdb }
    }

    #[tokio::test]
    async fn credit_mint_store_ok() {
        let db = init_credit_mem_db().await;
        let keys = core_tests::generate_random_ecash_keyset();
        let kid = keys.0.id;
        let kp = core::generate_random_keypair();
        let op = ebill::MintOperation {
            uid: Uuid::new_v4(),
            kid,
            pub_key: kp.public_key().into(),
            target: cashu::Amount::ZERO,
            minted: cashu::Amount::ZERO,
            bill_id: bcr_common::core_tests::random_bill_id(),
        };
        db.mint_store(op).await.unwrap();
    }
    #[tokio::test]
    async fn credit_mint_store_twice() {
        let db = init_credit_mem_db().await;
        let keys = core_tests::generate_random_ecash_keyset();
        let kid = keys.0.id;
        let kp = core::generate_random_keypair();
        let op = ebill::MintOperation {
            uid: Uuid::new_v4(),
            kid,
            pub_key: kp.public_key().into(),
            target: cashu::Amount::ZERO,
            minted: cashu::Amount::ZERO,
            bill_id: bcr_common::core_tests::random_bill_id(),
        };
        db.mint_store(op.clone()).await.unwrap();
        let res = db.mint_store(op).await;
        assert!(matches!(res, Err(Error::InvalidInput(_))));
    }

    #[tokio::test]
    async fn credit_mint_update_field() {
        let db = init_credit_mem_db().await;
        let keys = core_tests::generate_random_ecash_keyset();
        let kid = keys.0.id;
        let kp = core::generate_random_keypair();
        let op = ebill::MintOperation {
            uid: Uuid::new_v4(),
            kid,
            pub_key: kp.public_key().into(),
            target: cashu::Amount::ZERO,
            minted: cashu::Amount::ZERO,
            bill_id: bcr_common::core_tests::random_bill_id(),
        };
        db.mint_store(op.clone()).await.unwrap();
        let res = db.mint_load(op.uid).await.unwrap();
        assert_eq!(res.kid, kid);
        assert_eq!(res.pub_key, kp.public_key().into());
    }

    #[tokio::test]
    async fn update_minted_field() {
        let db = init_credit_mem_db().await;
        let keys = core_tests::generate_random_ecash_keyset();
        let kid = keys.0.id;
        let kp = core::generate_random_keypair();
        let op = ebill::MintOperation {
            uid: Uuid::new_v4(),
            kid,
            pub_key: kp.public_key().into(),
            target: cashu::Amount::ZERO,
            minted: cashu::Amount::ZERO,
            bill_id: bcr_common::core_tests::random_bill_id(),
        };
        db.mint_store(op.clone()).await.unwrap();
        db.mint_update_field(op.uid, cashu::Amount::ZERO, cashu::Amount::from(100u64))
            .await
            .unwrap();
        let res = db.mint_load(op.uid).await.unwrap();
        assert_eq!(res.kid, kid);
        assert_eq!(res.minted, cashu::Amount::from(100u64));
    }

    #[tokio::test]
    async fn credit_mint_list() {
        let db = init_credit_mem_db().await;
        let keys = core_tests::generate_random_ecash_keyset();
        let kid = keys.0.id;
        let kp = core::generate_random_keypair();
        let op1 = ebill::MintOperation {
            uid: Uuid::new_v4(),
            kid,
            pub_key: kp.public_key().into(),
            target: cashu::Amount::ZERO,
            minted: cashu::Amount::ZERO,
            bill_id: bcr_common::core_tests::random_bill_id(),
        };
        db.mint_store(op1.clone()).await.unwrap();
        let op2 = ebill::MintOperation {
            uid: Uuid::new_v4(),
            kid,
            pub_key: kp.public_key().into(),
            target: cashu::Amount::ZERO,
            minted: cashu::Amount::ZERO,
            bill_id: bcr_common::core_tests::random_bill_id(),
        };
        db.mint_store(op2.clone()).await.unwrap();
        let res = db.mint_list(kid).await.unwrap();
        assert_eq!(res.len(), 2);
        let rids: Vec<_> = res.iter().map(|op| op.uid).collect();
        assert!(rids.contains(&op1.uid));
        assert!(rids.contains(&op2.uid));
    }

    async fn init_vault_mem_db() -> DBVault {
        let sdb = Surreal::<Any>::init();
        sdb.connect("mem://").await.unwrap();
        sdb.use_ns("test").await.unwrap();
        sdb.use_db("test").await.unwrap();
        DBVault { db: sdb }
    }

    fn generate_test_proofs(n: usize) -> Vec<cashu::Proof> {
        let (_, keyset) = core_tests::generate_random_ecash_keyset();
        let amounts = vec![cashu::Amount::from(8u64); n];
        core_tests::generate_random_ecash_proofs(&keyset, &amounts)
    }

    #[tokio::test]
    async fn vault_store_load_proofs() {
        let db = init_vault_mem_db().await;
        let proofs = generate_test_proofs(3);
        let ys: Vec<cashu::PublicKey> = proofs.iter().map(|p| p.y().unwrap()).collect();
        db.store_proofs(proofs.clone()).await.unwrap();
        let loaded = db.load_proofs(vec![]).await.unwrap();
        assert!(loaded.is_empty());
        let loaded = db.load_proofs(ys).await.unwrap();
        assert_eq!(loaded.len(), 3);
        for proof in &proofs {
            assert!(loaded.contains(proof));
        }
    }

    #[tokio::test]
    async fn vault_load_proofs_partial() {
        let db = init_vault_mem_db().await;
        let proofs = generate_test_proofs(3);
        let ys: Vec<cashu::PublicKey> = proofs.iter().map(|p| p.y().unwrap()).collect();
        db.store_proofs(proofs.clone()).await.unwrap();
        let mut all_ys = ys.clone();
        let extra_y = cashu::PublicKey::from(core::generate_random_keypair().public_key());
        all_ys.push(extra_y);
        let loaded = db.load_proofs(all_ys).await.unwrap();
        assert_eq!(loaded.len(), 3);
    }

    #[tokio::test]
    async fn vault_list_ys() {
        let db = init_vault_mem_db().await;
        let ys = db.list_ys().await.unwrap();
        assert!(ys.is_empty());
        let proofs = generate_test_proofs(2);
        db.store_proofs(proofs.clone()).await.unwrap();
        let ys = db.list_ys().await.unwrap();
        assert_eq!(ys.len(), 2);
        for proof in &proofs {
            assert!(ys.contains(&proof.y().unwrap()));
        }
    }

    #[tokio::test]
    async fn vault_delete_proofs() {
        let db = init_vault_mem_db().await;
        let proofs = generate_test_proofs(3);
        let ys: Vec<cashu::PublicKey> = proofs.iter().map(|p| p.y().unwrap()).collect();
        db.store_proofs(proofs.clone()).await.unwrap();
        let to_delete = &ys[..2];
        db.delete_proofs(to_delete).await.unwrap();
        let remaining_ys = db.list_ys().await.unwrap();
        assert_eq!(remaining_ys.len(), 1);
        assert!(remaining_ys.contains(&ys[2]));
    }
}
