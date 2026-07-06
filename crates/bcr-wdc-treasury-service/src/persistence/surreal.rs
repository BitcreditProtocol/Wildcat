// ----- standard library imports
use std::str::FromStr;
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
    foreign, onchain, vault, TStamp,
};

// ----- end imports

////////////////////////////////////////////////////////////////// SurrealDB-safe wrappers

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct BlindedMessageDBEntry {
    amount: cashu::Amount,
    keyset_id: cashu::Id,
    blinded_secret: cashu::PublicKey,
    witness: Option<cashu::Witness>,
}
impl From<cashu::BlindedMessage> for BlindedMessageDBEntry {
    fn from(m: cashu::BlindedMessage) -> Self {
        Self {
            amount: m.amount,
            keyset_id: m.keyset_id,
            blinded_secret: m.blinded_secret,
            witness: m.witness,
        }
    }
}
impl From<BlindedMessageDBEntry> for cashu::BlindedMessage {
    fn from(m: BlindedMessageDBEntry) -> Self {
        Self {
            amount: m.amount,
            keyset_id: m.keyset_id,
            blinded_secret: m.blinded_secret,
            witness: m.witness,
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct BlindSignatureDBEntry {
    amount: cashu::Amount,
    keyset_id: cashu::Id,
    c: cashu::PublicKey,
    dleq: Option<cashu::BlindSignatureDleq>,
}
impl From<cashu::BlindSignature> for BlindSignatureDBEntry {
    fn from(s: cashu::BlindSignature) -> Self {
        Self {
            amount: s.amount,
            keyset_id: s.keyset_id,
            c: s.c,
            dleq: s.dleq,
        }
    }
}
impl From<BlindSignatureDBEntry> for cashu::BlindSignature {
    fn from(s: BlindSignatureDBEntry) -> Self {
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
enum MintStatusDBEntry {
    Pending {
        blinds: Vec<BlindedMessageDBEntry>,
    },
    Paid {
        signatures: Vec<BlindSignatureDBEntry>,
    },
    Expired,
}
impl From<onchain::MintStatus> for MintStatusDBEntry {
    fn from(s: onchain::MintStatus) -> Self {
        match s {
            onchain::MintStatus::Pending { blinds } => MintStatusDBEntry::Pending {
                blinds: blinds.into_iter().map(Into::into).collect(),
            },
            onchain::MintStatus::Paid { signatures } => MintStatusDBEntry::Paid {
                signatures: signatures.into_iter().map(Into::into).collect(),
            },
            onchain::MintStatus::Expired => MintStatusDBEntry::Expired,
        }
    }
}
impl From<MintStatusDBEntry> for onchain::MintStatus {
    fn from(s: MintStatusDBEntry) -> Self {
        match s {
            MintStatusDBEntry::Pending { blinds } => onchain::MintStatus::Pending {
                blinds: blinds.into_iter().map(Into::into).collect(),
            },
            MintStatusDBEntry::Paid { signatures } => onchain::MintStatus::Paid {
                signatures: signatures.into_iter().map(Into::into).collect(),
            },
            MintStatusDBEntry::Expired => onchain::MintStatus::Expired,
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct OnChainMintOperationDBEntry {
    qid: Uuid,
    kid: cashu::Id,
    recipient: bitcoin::Address<bitcoin::address::NetworkUnchecked>,
    target: bitcoin::Amount,
    expiry: crate::TStamp,
    status: MintStatusDBEntry,
}
impl From<onchain::MintOperation> for OnChainMintOperationDBEntry {
    fn from(op: onchain::MintOperation) -> Self {
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
impl From<OnChainMintOperationDBEntry> for onchain::MintOperation {
    fn from(op: OnChainMintOperationDBEntry) -> Self {
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

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct OnChainDeniedMeltOpDbEntry {
    id: RecordId,
    inputs: bitcoin::Amount,
    created: TStamp,
}
impl From<onchain::DeniedMeltOperation> for OnChainDeniedMeltOpDbEntry {
    fn from(op: onchain::DeniedMeltOperation) -> Self {
        Self {
            id: RecordId::from_table_key(DBOnChain::DENIED_TABLE, op.qid),
            inputs: op.inputs,
            created: op.created,
        }
    }
}
impl From<OnChainDeniedMeltOpDbEntry> for onchain::DeniedMeltOperation {
    fn from(db: OnChainDeniedMeltOpDbEntry) -> Self {
        let qid = Uuid::try_from(db.id.key().clone()).expect("key is a uuid");
        Self {
            qid,
            inputs: db.inputs,
            created: db.created,
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "status")]
enum MeltStatusDBEntry {
    Pending,
    Paid { tx: String },
    Expired,
    Canceled,
}
impl From<onchain::MeltStatus> for MeltStatusDBEntry {
    fn from(s: onchain::MeltStatus) -> Self {
        match s {
            onchain::MeltStatus::Pending => MeltStatusDBEntry::Pending,
            onchain::MeltStatus::Paid { tx } => MeltStatusDBEntry::Paid { tx: tx.to_string() },
            onchain::MeltStatus::Expired => MeltStatusDBEntry::Expired,
            onchain::MeltStatus::Canceled => MeltStatusDBEntry::Canceled,
        }
    }
}
impl From<MeltStatusDBEntry> for onchain::MeltStatus {
    fn from(s: MeltStatusDBEntry) -> Self {
        match s {
            MeltStatusDBEntry::Pending => onchain::MeltStatus::Pending,
            MeltStatusDBEntry::Paid { tx } => onchain::MeltStatus::Paid {
                tx: bitcoin::Txid::from_str(&tx).expect("tx <--> String"),
            },
            MeltStatusDBEntry::Expired => onchain::MeltStatus::Expired,
            MeltStatusDBEntry::Canceled => onchain::MeltStatus::Canceled,
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct OnChainMeltOpDbEntry {
    id: RecordId,
    address: String,
    available: cashu::Amount,
    target: bitcoin::Amount,
    fees: cashu::Amount,
    expiry: TStamp,
    commitment: String,
    input_ys: Vec<String>,
    fp_digest: [u8; 32],
    status: MeltStatusDBEntry,
    wallet_key: String,
}
impl From<OnChainMeltOpDbEntry> for onchain::MeltOperation {
    fn from(entry: OnChainMeltOpDbEntry) -> Self {
        Self {
            qid: Uuid::try_from(entry.id.key().clone()).expect("key is a uuid"),
            address: entry.address,
            available: entry.available,
            target: entry.target,
            fees: entry.fees,
            expiry: entry.expiry,
            status: entry.status.into(),
            commitment: secp256k1::schnorr::Signature::from_str(&entry.commitment)
                .expect("commitment <--> String"),
            wallet_key: cashu::PublicKey::from_str(&entry.wallet_key)
                .expect("wallet_key <--> String"),
            input_ys: entry
                .input_ys
                .into_iter()
                .map(|s| cashu::PublicKey::from_str(&s).expect("input_ys <--> String"))
                .collect(),
            fp_digest: entry.fp_digest,
        }
    }
}
fn convert_to_onchainmeltop(op: onchain::MeltOperation, table: &str) -> OnChainMeltOpDbEntry {
    let id = RecordId::from_table_key(table, op.qid);
    OnChainMeltOpDbEntry {
        id,
        address: op.address,
        available: op.available,
        target: op.target,
        fees: op.fees,
        expiry: op.expiry,
        commitment: op.commitment.to_string(),
        input_ys: op.input_ys.into_iter().map(|y| y.to_string()).collect(),
        fp_digest: op.fp_digest,
        status: op.status.into(),
        wallet_key: op.wallet_key.to_string(),
    }
}

///////////////////////////////////////////////////////////////////////////////// OnChain DB
#[derive(Debug, Clone)]
pub struct DBOnChain {
    db: Surreal<Any>,
}

impl DBOnChain {
    const MELTS_TABLE: &'static str = "onchain_melts";
    const MINTS_TABLE: &'static str = "onchain_mints";
    const DENIED_TABLE: &'static str = "onchain_denied";

    pub async fn new(config: surreal::DBConnConfig) -> SurrealResult<Self> {
        let db_connection = Surreal::<Any>::init();
        db_connection.connect(config.connection).await?;
        db_connection.use_ns(config.namespace).await?;
        db_connection.use_db(config.database).await?;
        Ok(Self { db: db_connection })
    }

    async fn mintops_mark_expired(&self, now: TStamp) -> SurrealResult<()> {
        self.db
            .query(
                "
            UPDATE type::table($table)
            SET status = $expired
            WHERE status != $expired AND expiry < $now
            ",
            )
            .bind(("table", Self::MINTS_TABLE))
            .bind(("expired", onchain::MintStatus::Expired))
            .bind(("now", now))
            .await?;
        Ok(())
    }

    async fn meltops_mark_expired(&self, now: TStamp) -> SurrealResult<()> {
        self.db
            .query(
                "
            UPDATE type::table($table)
            SET status = $expired
            WHERE status != $expired AND expiry < $now
            ",
            )
            .bind(("table", Self::MELTS_TABLE))
            .bind(("expired", MeltStatusDBEntry::Expired))
            .bind(("now", now))
            .await?;
        Ok(())
    }
}

#[async_trait]
impl onchain::Repository for DBOnChain {
    async fn store_mintop(&self, op: onchain::MintOperation) -> Result<()> {
        let rid = RecordId::from_table_key(Self::MINTS_TABLE, op.qid);
        let db_op = OnChainMintOperationDBEntry::from(op);
        let _: Option<OnChainMintOperationDBEntry> = self
            .db
            .insert(rid)
            .content(db_op)
            .await
            .map_err(|e| Error::DB(anyhow!(e)))?;
        Ok(())
    }

    async fn load_mintop(&self, qid: Uuid) -> Result<onchain::MintOperation> {
        let rid = RecordId::from_table_key(Self::MINTS_TABLE, qid);
        let result: Option<OnChainMintOperationDBEntry> = self
            .db
            .select(rid)
            .await
            .map_err(|e| Error::DB(anyhow!(e)))?;
        result
            .map(Into::into)
            .ok_or_else(|| Error::ResourceNotFound(qid.to_string()))
    }

    async fn list_pending_mintops(&self, now: TStamp) -> Result<Vec<Uuid>> {
        self.mintops_mark_expired(now)
            .await
            .map_err(|e| Error::DB(anyhow!(e)))?;
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

    async fn update_mintop_status(&self, qid: Uuid, status: onchain::MintStatus) -> Result<()> {
        let rid = RecordId::from_table_key(Self::MINTS_TABLE, qid);
        let db_status = MintStatusDBEntry::from(status);
        let entry: Option<OnChainMintOperationDBEntry> = self
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

    async fn store_meltop(&self, op: onchain::MeltOperation, now: TStamp) -> Result<()> {
        self.meltops_mark_expired(now)
            .await
            .map_err(|e| Error::DB(anyhow!(e)))?;
        let entry = convert_to_onchainmeltop(op, Self::MELTS_TABLE);
        let mut query = self
            .db
            .query(
                "
            BEGIN;
            LET $noys = array::is_empty(
                SELECT input_ys
                FROM type::table($table)
                WHERE
                    status.status = $status
                    AND
                    !array::is_empty(array::intersect(input_ys, $input_ys))

            );
            if $noys {
                INSERT $content
            };
            SELECT * FROM $newrid;
            COMMIT
            ",
            )
            .bind(("table", Self::MELTS_TABLE))
            .bind(("status", onchain::MeltStatusDiscriminants::Pending))
            .bind(("input_ys", entry.input_ys.clone()))
            .bind(("newrid", entry.id.clone()))
            .bind(("content", entry))
            .await
            .map_err(|e| Error::DB(anyhow!(e)))?;
        let inserted: Option<OnChainMeltOpDbEntry> = query
            .take(query.num_statements() - 1)
            .map_err(|e| Error::DB(anyhow!(e)))?;
        if inserted.is_none() {
            Err(Error::InvalidInput(String::from(
                "meltop: inputs already locked in",
            )))
        } else {
            Ok(())
        }
    }

    async fn load_meltop(&self, qid: Uuid) -> Result<onchain::MeltOperation> {
        let rid = RecordId::from_table_key(Self::MELTS_TABLE, qid);
        let result: OnChainMeltOpDbEntry = self
            .db
            .select(rid)
            .await
            .map_err(|e| Error::DB(anyhow!(e)))?
            .ok_or_else(|| Error::ResourceNotFound(qid.to_string()))?;
        Ok(result.into())
    }

    async fn update_meltop_status(&self, qid: Uuid, status: onchain::MeltStatus) -> Result<()> {
        let rid = RecordId::from_table_key(Self::MELTS_TABLE, qid);
        let entry: Option<OnChainMeltOpDbEntry> = self
            .db
            .query("UPDATE $rid SET status = $status")
            .bind(("rid", rid))
            .bind(("status", MeltStatusDBEntry::from(status)))
            .await
            .map_err(|e| Error::DB(anyhow!(e)))?
            .take(0)
            .map_err(|e| Error::DB(anyhow!(e)))?;
        if entry.is_none() {
            return Err(Error::ResourceNotFound(qid.to_string()));
        }
        Ok(())
    }

    async fn list_pending_meltops(&self, now: TStamp) -> Result<Vec<Uuid>> {
        self.meltops_mark_expired(now)
            .await
            .map_err(|e| Error::DB(anyhow!(e)))?;
        let entries: Vec<RecordId> = self
            .db
            .query("SELECT id FROM type::table($table) WHERE status.status = $status")
            .bind(("table", Self::MELTS_TABLE))
            .bind(("status", onchain::MeltStatusDiscriminants::Pending))
            .await
            .map_err(|e| Error::DB(anyhow!(e)))?
            .take("id")
            .map_err(|e| Error::DB(anyhow!(e)))?;
        let ids = entries
            .into_iter()
            .map(|id| Uuid::try_from(id.key().clone()).expect("key is a uuid"))
            .collect();
        Ok(ids)
    }

    async fn store_denied_meltop(&self, op: onchain::DeniedMeltOperation) -> Result<()> {
        let entry = OnChainDeniedMeltOpDbEntry::from(op);
        let _: Option<OnChainDeniedMeltOpDbEntry> = self
            .db
            .insert(entry.id.clone())
            .content(entry)
            .await
            .map_err(|e| Error::DB(anyhow!(e)))?;
        Ok(())
    }

    async fn list_denied_meltops(&self) -> Result<Vec<onchain::DeniedMeltOperation>> {
        let entries: Vec<OnChainDeniedMeltOpDbEntry> = self
            .db
            .query("SELECT * FROM type::table($table)")
            .bind(("table", Self::DENIED_TABLE))
            .await
            .map_err(|e| Error::DB(anyhow!(e)))?
            .take(0)
            .map_err(|e| Error::DB(anyhow!(e)))?;
        let meltops = entries.into_iter().map(Into::into).collect();
        Ok(meltops)
    }

    async fn delete_denied_meltop(&self, qid: Uuid) -> Result<()> {
        let rid = RecordId::from_table_key(Self::DENIED_TABLE, qid);
        let _: Option<OnChainDeniedMeltOpDbEntry> = self
            .db
            .delete(rid)
            .await
            .map_err(|e| Error::DB(anyhow!(e)))?;
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
    pub(super) db: Surreal<surrealdb::engine::any::Any>,
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

    pub async fn dump(&self) -> Result<Vec<ebill::MintOperation>> {
        let ops: Vec<EbillMintOpDBEntry> = self
            .db
            .query("SELECT * FROM type::table($table)")
            .bind(("table", Self::MINT_OPS))
            .await
            .map_err(|e| Error::DB(anyhow!(e)))?
            .take(0)
            .map_err(|e| Error::DB(anyhow!(e)))?;
        let ops = ops.into_iter().map(ebill::MintOperation::from).collect();
        Ok(ops)
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
    // storing as String, otherwise array::group() merges them into one entry
    mint_id: String,
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
                mint_id: mint_id.to_string(),
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

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct ForeignFingerprintDBEntry {
    id: RecordId,
    amount: u64,
    keyset_id: cashu::Id,
    y: cashu::PublicKey,
    c: cashu::PublicKey,
    dleq: Option<cashu::ProofDleq>,
    mint_id: String,
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
                dleq: fp.dleq,
                mint_id: mint_id.to_string(),
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
            dleq: entry.dleq,
        };
        let mint_id = secp256k1::PublicKey::from_str(&entry.mint_id).expect("mint_id <--> String");
        Ok(Some((mint_id, fp)))
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
                mint_id: mint_id.to_string(),
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
            .bind(("mint_id", mint_id.to_string()))
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
    async fn list_foreign_pks(&self) -> Result<Vec<secp256k1::PublicKey>> {
        #[derive(Debug, Default, Clone, serde::Deserialize)]
        struct MintIdEntries {
            mint_id: Vec<String>,
        }
        let entries: Option<MintIdEntries> = self
            .db
            .query("SELECT array::group(mint_id) AS mint_id FROM type::table($table) GROUP ALL")
            .bind(("table", Self::PROOFS_TABLE))
            .await
            .map_err(|e| Error::DB(anyhow!(e)))?
            .take(0)
            .map_err(|e| Error::DB(anyhow!(e)))?;
        let mint_ids: Vec<secp256k1::PublicKey> = entries
            .unwrap_or_default()
            .mint_id
            .into_iter()
            .map(|m| secp256k1::PublicKey::from_str(&m).expect("mint_id <--> String"))
            .collect();
        Ok(mint_ids)
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
    pub(super) db: Surreal<Any>,
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

    pub async fn dump(&self) -> Result<Vec<cashu::Proof>> {
        let entries: Vec<VaultProofDBEntry> = self
            .db
            .query("SELECT * FROM type::table($table)")
            .bind(("table", Self::PROOFS_TABLE))
            .await
            .map_err(|e| Error::DB(anyhow!(e)))?
            .take(0)
            .map_err(|e| Error::DB(anyhow!(e)))?;
        let proofs = entries.into_iter().map(Into::into).collect();
        Ok(proofs)
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
    use crate::foreign::OfflineRepository;
    use bcr_common::{core, core_tests};
    use bcr_wdc_utils::signatures::test_utils as signature_tests;
    use bitcoin::hashes::Hash;
    use std::str::FromStr;

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
                dleq: None,
            },
            wire_keys::ProofFingerprint {
                amount: 10,
                keyset_id: cashu::Id::from_bytes(&[1; 33]).unwrap(),
                y: cashu::PublicKey::from(core::generate_random_keypair().public_key()),
                c: cashu::PublicKey::from(core::generate_random_keypair().public_key()),
                dleq: None,
            },
        ];
        let hash = vec![
            Sha256Hash::from_slice(&[0u8; 32]).unwrap(),
            Sha256Hash::from_slice(&[1u8; 32]).unwrap(),
        ];
        db.store_fps(alpha_id, fps, hash.clone()).await.unwrap();
        let result = db.search_fp(&hash[0]).await.unwrap();
        assert!(result.is_some());
        let (mint, fp) = result.unwrap();
        assert_eq!(mint, alpha_id);
        assert_eq!(fp.y, y);
    }

    #[tokio::test]
    async fn offline_list_foreign() {
        let db = init_foreignoffline_mem_db().await;
        let amounts = vec![cashu::Amount::from(8u64), cashu::Amount::from(16u64)];
        let (_, keyset) = core_tests::generate_random_ecash_keyset();
        let alpha1 = core::generate_random_keypair().public_key();
        let proofs: Vec<cashu::Proof> = core_tests::generate_random_ecash_proofs(&keyset, &amounts);
        db.store_proofs(alpha1, proofs.clone()).await.unwrap();
        let alpha2 = core::generate_random_keypair().public_key();
        let proofs: Vec<cashu::Proof> = core_tests::generate_random_ecash_proofs(&keyset, &amounts);
        db.store_proofs(alpha2, proofs).await.unwrap();
        let result = db.list_foreign_pks().await.unwrap();
        assert_eq!(result.len(), 2);
        assert!(result.contains(&alpha1));
        assert!(result.contains(&alpha2));
    }

    #[tokio::test]
    async fn offline_load_proofs() {
        let db = init_foreignoffline_mem_db().await;
        let amounts = vec![cashu::Amount::from(8u64), cashu::Amount::from(16u64)];
        let (_, keyset) = core_tests::generate_random_ecash_keyset();
        let alpha = core::generate_random_keypair().public_key();
        let proofs: Vec<cashu::Proof> = core_tests::generate_random_ecash_proofs(&keyset, &amounts);
        db.store_proofs(alpha, proofs.clone()).await.unwrap();
        let result = db.load_proofs(alpha).await.unwrap();
        assert_eq!(result.len(), proofs.len());
        for proof in proofs {
            assert!(result.contains(&proof));
        }
    }
}
