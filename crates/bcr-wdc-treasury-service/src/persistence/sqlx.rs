// ----- standard library imports
use std::{ops::Deref, str::FromStr};
// ----- extra library imports
use anyhow::anyhow;
use async_trait::async_trait;
use bcr_common::{cashu, core::BillId, ecash, wire::keys as wire_keys};
use sqlx::types::Json;
use sqlx::PgPool;
use sqlx::Row;
use strum::IntoDiscriminant;
use uuid::Uuid;
// ----- local imports
use crate::{
    ebill,
    error::{Error, Result},
    foreign, onchain, vault, TStamp,
};

// ----- end imports

// ///////////////////////////////////////////////////////////////////////// Versioned blob

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "version", content = "data")]
enum EbillMintOperationBlob {
    V1(EbillMintOpBlobV1),
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct EbillMintOpBlobV1 {
    target: bitcoin::Amount,
    pub_key: secp256k1::PublicKey,
}

// ///////////////////////////////////////////////////////////////////////// DBEbill

#[derive(Debug, Clone)]
pub struct DBEbill {
    pool: PgPool,
}

impl DBEbill {
    pub fn from_pool(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl ebill::Repository for DBEbill {
    async fn mint_store(&self, mint_op: ebill::MintOperation) -> Result<()> {
        let uid = mint_op.uid;
        let blob = EbillMintOperationBlob::V1(EbillMintOpBlobV1 {
            target: bitcoin::Amount::from_sat(mint_op.target.to_u64()),
            pub_key: *mint_op.pub_key.deref(),
        });
        let blob_value = serde_json::to_value(&blob).map_err(|e| Error::DB(anyhow!(e)))?;
        let result = sqlx::query!(
            r#"
            INSERT INTO treasury_ebill_mint_ops (uid, kid, minted, bill_id, blob)
            VALUES ($1, $2, $3, $4, $5)
            ON CONFLICT (uid) DO NOTHING
            RETURNING uid
            "#,
            uid,
            mint_op.kid.to_string(),
            mint_op.minted.to_u64() as i64,
            mint_op.bill_id.to_string(),
            blob_value
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| match e.as_database_error() {
            Some(db) if db.is_unique_violation() => Error::AlreadyExists(format!("mintop {uid}")),
            _ => Error::DB(anyhow!(e)),
        })?;
        if result.is_none() {
            return Err(Error::AlreadyExists(format!("mintop {uid}")));
        }
        Ok(())
    }

    async fn mint_load(&self, uid: Uuid) -> Result<ebill::MintOperation> {
        let result = sqlx::query!(
            r#"
            SELECT uid, kid, minted, bill_id, blob as "blob: Json<EbillMintOperationBlob>"
            FROM treasury_ebill_mint_ops
            WHERE uid = $1
            "#,
            uid
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| Error::DB(anyhow!(e)))?;
        let Some(row) = result else {
            return Err(Error::ResourceNotFound(uid.to_string()));
        };
        let kid = cashu::Id::from_str(&row.kid).map_err(|e| Error::DB(anyhow!(e)))?;
        let bill_id = BillId::from_str(&row.bill_id).map_err(|e| Error::DB(anyhow!(e)))?;
        let EbillMintOperationBlob::V1(v1) = row.blob.0;
        Ok(ebill::MintOperation {
            uid: row.uid,
            kid,
            minted: cashu::Amount::from(row.minted as u64),
            bill_id,
            target: cashu::Amount::from(v1.target.to_sat()),
            pub_key: cashu::PublicKey::from(v1.pub_key),
        })
    }

    async fn mint_lookup_by_bill(&self, bill_id: BillId) -> Result<Option<ebill::MintOperation>> {
        let result = sqlx::query!(
            r#"
            SELECT uid, kid, minted, bill_id, blob as "blob: Json<EbillMintOperationBlob>"
            FROM treasury_ebill_mint_ops
            WHERE bill_id = $1
            "#,
            bill_id.to_string()
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| Error::DB(anyhow!(e)))?;
        let Some(row) = result else {
            return Ok(None);
        };
        let kid = cashu::Id::from_str(&row.kid).map_err(|e| Error::DB(anyhow!(e)))?;
        let bill_id = BillId::from_str(&row.bill_id).map_err(|e| Error::DB(anyhow!(e)))?;
        let EbillMintOperationBlob::V1(v1) = row.blob.0;
        Ok(Some(ebill::MintOperation {
            uid: row.uid,
            kid,
            minted: cashu::Amount::from(row.minted as u64),
            bill_id,
            target: cashu::Amount::from(v1.target.to_sat()),
            pub_key: cashu::PublicKey::from(v1.pub_key),
        }))
    }

    async fn mint_list(&self, kid: cashu::Id) -> Result<Vec<ebill::MintOperation>> {
        let results = sqlx::query!(
            r#"
            SELECT uid, kid, minted, bill_id, blob as "blob: Json<EbillMintOperationBlob>"
            FROM treasury_ebill_mint_ops
            WHERE kid = $1
            "#,
            kid.to_string()
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| Error::DB(anyhow!(e)))?;
        let mut ops = Vec::with_capacity(results.len());
        for row in results {
            let kid = cashu::Id::from_str(&row.kid).map_err(|e| Error::DB(anyhow!(e)))?;
            let bill_id = BillId::from_str(&row.bill_id).map_err(|e| Error::DB(anyhow!(e)))?;
            let EbillMintOperationBlob::V1(v1) = row.blob.0;
            ops.push(ebill::MintOperation {
                uid: row.uid,
                kid,
                minted: cashu::Amount::from(row.minted as u64),
                bill_id,
                target: cashu::Amount::from(v1.target.to_sat()),
                pub_key: cashu::PublicKey::from(v1.pub_key),
            });
        }
        Ok(ops)
    }

    async fn mint_update_field(
        &self,
        uid: Uuid,
        old_minted: cashu::Amount,
        new_minted: cashu::Amount,
    ) -> Result<()> {
        let result = sqlx::query!(
            r#"
            UPDATE treasury_ebill_mint_ops
            SET minted = $3
            WHERE uid = $1 AND minted = $2
            RETURNING uid
            "#,
            uid,
            old_minted.to_u64() as i64,
            new_minted.to_u64() as i64,
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| Error::DB(anyhow!(e)))?;
        if result.is_none() {
            return Err(Error::InvalidInput(format!(
                "mintop {uid} and {old} amount not found",
                old = old_minted
            )));
        }
        Ok(())
    }
}

// ///////////////////////////////////////////////////////////////////////// Versioned foreign proof blob
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "version", content = "data")]
enum ForeignProofBlob {
    V1(ecash::Proof),
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "version", content = "data")]
enum ForeignHtlcProofBlob {
    V1(ecash::Proof),
}

/////////////////////////////////////////////////////////////////////////// DBForeignOnline

#[derive(sqlx::FromRow, Debug, Clone)]
struct ForeignHtlcProofRow {
    mint_id: String,
    blob: Json<ForeignHtlcProofBlob>,
}

#[derive(Debug, Clone)]
pub struct DBForeignOnline {
    pool: PgPool,
}

impl DBForeignOnline {
    pub fn from_pool(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl foreign::OnlineRepository for DBForeignOnline {
    async fn store(&self, mint_id: secp256k1::PublicKey, proofs: Vec<cashu::Proof>) -> Result<()> {
        if proofs.is_empty() {
            return Ok(());
        }
        let mint_id = mint_id.to_string();
        let mut blobs = Vec::with_capacity(proofs.len());
        for proof in proofs {
            let blob = ForeignProofBlob::V1(proof.into());
            let jason = serde_json::to_string(&blob).map_err(|e| Error::DB(anyhow!(e)))?;
            blobs.push(jason);
        }
        sqlx::query!(
            r#"
            INSERT INTO treasury_foreign_proofs (mint_id, blobs)
            VALUES ($1, $2)
            ON CONFLICT (mint_id) DO UPDATE
            SET blobs = array_cat(treasury_foreign_proofs.blobs, EXCLUDED.blobs)
            "#,
            mint_id,
            &blobs,
        )
        .execute(&self.pool)
        .await
        .map_err(|e| Error::DB(anyhow!(e)))?;
        Ok(())
    }

    async fn list(&self, mint_id: secp256k1::PublicKey) -> Result<Vec<cashu::Proof>> {
        let result = sqlx::query!(
            r#"
            SELECT blobs
            FROM treasury_foreign_proofs
            WHERE mint_id = $1
            "#,
            mint_id.to_string()
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| Error::DB(anyhow!(e)))?;
        let Some(row) = result else {
            return Ok(Vec::new());
        };
        let mut proofs = Vec::with_capacity(row.blobs.len());
        for blob in &row.blobs {
            let blob: ForeignProofBlob =
                serde_json::from_str(blob).map_err(|e| Error::DB(anyhow!(e)))?;
            match blob {
                ForeignProofBlob::V1(proof) => {
                    proofs.push(cashu::Proof::from(proof));
                }
            }
        }
        Ok(proofs)
    }

    async fn settled_balance(
        &self,
    ) -> Result<std::collections::HashMap<secp256k1::PublicKey, cashu::Amount>> {
        let rows = sqlx::query!(
            r#"
            SELECT mint_id, blobs
            FROM treasury_foreign_proofs
            "#
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| Error::DB(anyhow!(e)))?;
        let mut balances = std::collections::HashMap::with_capacity(rows.len());
        for row in rows {
            let mint_id =
                secp256k1::PublicKey::from_str(&row.mint_id).map_err(|e| Error::DB(anyhow!(e)))?;
            let mut proofs = Vec::with_capacity(row.blobs.len());
            for blob in &row.blobs {
                let blob: ForeignProofBlob =
                    serde_json::from_str(blob).map_err(|e| Error::DB(anyhow!(e)))?;
                match blob {
                    ForeignProofBlob::V1(proof) => proofs.push(proof),
                }
            }
            let total = cashu::Amount::try_sum(proofs.iter().map(|p| p.amount))
                .expect("amounts are already checked");
            balances.insert(mint_id, total);
        }
        Ok(balances)
    }

    async fn store_htlc(
        &self,
        mint_id: secp256k1::PublicKey,
        hash: foreign::Sha256Hash,
        proofs: Vec<cashu::Proof>,
    ) -> Result<()> {
        let mint_id = mint_id.to_string();
        let hash = hash.to_string();
        let mut ys = Vec::with_capacity(proofs.len());
        let mut blobs = Vec::with_capacity(proofs.len());
        for p in proofs {
            let y = p.y().map_err(|e| Error::DB(anyhow!(e)))?;
            ys.push(y.to_string());
            let blob = ForeignHtlcProofBlob::V1(p.into());
            let jason = serde_json::to_value(&blob).map_err(|e| Error::DB(anyhow!(e)))?;
            blobs.push(jason);
        }
        let mint_ids = vec![mint_id; ys.len()];
        let hashes = vec![hash; ys.len()];
        let mut tx = self.pool.begin().await.map_err(|e| Error::DB(anyhow!(e)))?;
        let result = sqlx::query!(
            r#"
            INSERT INTO treasury_foreign_online_htlc_proofs ( y, hash, mint_id, blob)
            SELECT * FROM UNNEST($1::text[], $2::text[], $3::text[], $4::jsonb[])
            ON CONFLICT (y) DO NOTHING
            "#,
            &ys,
            &hashes,
            &mint_ids,
            &blobs,
        )
        .execute(&mut *tx)
        .await
        .map_err(|e| Error::DB(anyhow!(e)))?;
        if result.rows_affected() != ys.len() as u64 {
            tx.rollback().await.map_err(|e| Error::DB(anyhow!(e)))?;
            return Err(Error::InvalidInput(String::from("proofs already spent")));
        }
        tx.commit().await.map_err(|e| Error::DB(anyhow!(e)))?;
        Ok(())
    }

    async fn search_htlc(
        &self,
        hash: &foreign::Sha256Hash,
    ) -> Result<Vec<(secp256k1::PublicKey, cashu::Proof)>> {
        let results = sqlx::query_as!(
            ForeignHtlcProofRow,
            r#"
            SELECT mint_id, blob as "blob: Json<ForeignHtlcProofBlob>"
            FROM treasury_foreign_online_htlc_proofs
            WHERE hash = $1
            "#,
            hash.to_string()
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| Error::DB(anyhow!(e)))?;
        let mut proofs = Vec::with_capacity(results.len());
        for row in results {
            let mint_id =
                secp256k1::PublicKey::from_str(&row.mint_id).map_err(|e| Error::DB(anyhow!(e)))?;
            let ForeignHtlcProofBlob::V1(proof) = row.blob.0;
            proofs.push((mint_id, cashu::Proof::from(proof)))
        }
        Ok(proofs)
    }

    async fn remove_htlcs(&self, ys: &[cashu::PublicKey]) -> Result<()> {
        let y_strs: Vec<String> = ys.iter().map(|y| y.to_string()).collect();
        sqlx::query!(
            r#"
            DELETE FROM treasury_foreign_online_htlc_proofs WHERE y = ANY($1::text[])
            "#,
            &y_strs
        )
        .execute(&self.pool)
        .await
        .map_err(|e| Error::DB(anyhow!(e)))?;
        Ok(())
    }

    async fn store_issued(
        &self,
        hash: foreign::Sha256Hash,
        locktime: TStamp,
        proofs: Vec<cashu::Proof>,
    ) -> Result<()> {
        let hash = hash.to_string();
        let mut ys = Vec::with_capacity(proofs.len());
        let mut blobs = Vec::with_capacity(proofs.len());
        for p in proofs {
            let y = p.y().map_err(|e| Error::DB(anyhow!(e)))?;
            ys.push(y.to_string());
            let blob = ForeignHtlcProofBlob::V1(p.into());
            let jason = serde_json::to_value(&blob).map_err(|e| Error::DB(anyhow!(e)))?;
            blobs.push(jason);
        }
        let hashes = vec![hash; ys.len()];
        let locktimes = vec![locktime.unix_timestamp(); ys.len()];
        sqlx::query!(
            r#"
            INSERT INTO treasury_foreign_issued_htlc_proofs (y, hash, locktime, blob)
            SELECT * FROM UNNEST($1::text[], $2::text[], $3::bigint[], $4::jsonb[])
            ON CONFLICT (y) DO NOTHING
            "#,
            &ys,
            &hashes,
            &locktimes,
            &blobs,
        )
        .execute(&self.pool)
        .await
        .map_err(|e| Error::DB(anyhow!(e)))?;
        Ok(())
    }

    async fn list_expired_issued(&self, now: TStamp) -> Result<Vec<cashu::Proof>> {
        let rows = sqlx::query!(
            r#"
            SELECT blob FROM treasury_foreign_issued_htlc_proofs WHERE locktime < $1
            "#,
            now.unix_timestamp(),
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| Error::DB(anyhow!(e)))?;
        let mut proofs = Vec::with_capacity(rows.len());
        for row in rows {
            let blob: ForeignHtlcProofBlob =
                serde_json::from_value(row.blob).map_err(|e| Error::DB(anyhow!(e)))?;
            match blob {
                ForeignHtlcProofBlob::V1(proof) => proofs.push(cashu::Proof::from(proof)),
            }
        }
        Ok(proofs)
    }

    async fn remove_issued(&self, ys: &[cashu::PublicKey]) -> Result<()> {
        let y_strs: Vec<String> = ys.iter().map(|y| y.to_string()).collect();
        sqlx::query!(
            r#"
            DELETE FROM treasury_foreign_issued_htlc_proofs WHERE y = ANY($1::text[])
            "#,
            &y_strs
        )
        .execute(&self.pool)
        .await
        .map_err(|e| Error::DB(anyhow!(e)))?;
        Ok(())
    }

    async fn remove_issued_by_hash(&self, hash: &foreign::Sha256Hash) -> Result<()> {
        sqlx::query!(
            r#"
            DELETE FROM treasury_foreign_issued_htlc_proofs WHERE hash = $1
            "#,
            hash.to_string(),
        )
        .execute(&self.pool)
        .await
        .map_err(|e| Error::DB(anyhow!(e)))?;
        Ok(())
    }
}

// ///////////////////////////////////////////////////////////////////////// Versioned foreign offline blobs

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "version", content = "data")]
enum ForeignOfflineProofBlob {
    V1(ecash::Proof),
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "version", content = "data")]
enum ForeignFingerprintBlob {
    V1(ForeignFingerprintBlobV1),
}

/// A proof fingerprint without its `y`, which is a column of its own.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct ForeignFingerprintBlobV1 {
    keyset_id: ecash::Id,
    amount: u64,
    c: secp256k1::PublicKey,
    dleq: Option<ProofDleqBlobV1>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct ProofDleqBlobV1 {
    e: secp256k1::SecretKey,
    s: secp256k1::SecretKey,
    r: secp256k1::SecretKey,
}

impl From<cashu::ProofDleq> for ProofDleqBlobV1 {
    fn from(dleq: cashu::ProofDleq) -> Self {
        Self {
            e: *dleq.e,
            s: *dleq.s,
            r: *dleq.r,
        }
    }
}

impl From<ProofDleqBlobV1> for cashu::ProofDleq {
    fn from(blob: ProofDleqBlobV1) -> Self {
        cashu::ProofDleq::new(blob.e.into(), blob.s.into(), blob.r.into())
    }
}

impl From<wire_keys::ProofFingerprint> for ForeignFingerprintBlob {
    fn from(fp: wire_keys::ProofFingerprint) -> Self {
        Self::V1(ForeignFingerprintBlobV1 {
            keyset_id: fp.keyset_id,
            amount: fp.amount,
            c: fp.c,
            dleq: fp.dleq.map(ProofDleqBlobV1::from),
        })
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "version", content = "data")]
enum OfflineExchangeBlob {
    V1(OfflineExchangeBlobV1),
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct OfflineExchangeBlobV1 {
    evidence_digest: [u8; 32],
    proofs: Vec<ecash::Proof>,
}

/////////////////////////////////////////////////////////////////////////// DBForeignOffline

#[derive(sqlx::FromRow, Debug, Clone)]
struct ForeignFingerprintRow {
    y: String,
    mint_id: String,
    blob: Json<ForeignFingerprintBlob>,
}

impl TryFrom<ForeignFingerprintRow> for (secp256k1::PublicKey, wire_keys::ProofFingerprint) {
    type Error = Error;
    fn try_from(row: ForeignFingerprintRow) -> Result<Self> {
        let mint_id =
            secp256k1::PublicKey::from_str(&row.mint_id).map_err(|e| Error::DB(anyhow!(e)))?;
        let y = secp256k1::PublicKey::from_str(&row.y).map_err(|e| Error::DB(anyhow!(e)))?;
        let ForeignFingerprintBlob::V1(blob) = row.blob.0;
        let fp = wire_keys::ProofFingerprint {
            keyset_id: blob.keyset_id,
            amount: blob.amount,
            y,
            c: blob.c,
            dleq: blob.dleq.map(cashu::ProofDleq::from),
        };
        Ok((mint_id, fp))
    }
}

#[derive(sqlx::FromRow, Debug, Clone)]
struct OfflineExchangeRow {
    exchange_digest: Vec<u8>,
    alpha_id: String,
    expires_at: i64,
    blob: Json<OfflineExchangeBlob>,
}

impl TryFrom<OfflineExchangeRow> for foreign::OfflineExchange {
    type Error = Error;
    fn try_from(row: OfflineExchangeRow) -> Result<Self> {
        let exchange_digest: [u8; 32] = row
            .exchange_digest
            .try_into()
            .map_err(|_| Error::DB(anyhow!("exchange_digest is not 32 bytes")))?;
        let alpha_id =
            secp256k1::PublicKey::from_str(&row.alpha_id).map_err(|e| Error::DB(anyhow!(e)))?;
        let expires_at =
            TStamp::from_unix_timestamp(row.expires_at).map_err(|e| Error::DB(anyhow!(e)))?;
        let OfflineExchangeBlob::V1(blob) = row.blob.0;
        Ok(Self {
            exchange_digest,
            alpha_id,
            evidence_digest: blob.evidence_digest,
            expires_at,
            proofs: blob.proofs.into_iter().map(cashu::Proof::from).collect(),
        })
    }
}

type PgTransaction<'a> = sqlx::Transaction<'a, sqlx::Postgres>;

/// Inserts the exchange and the ys it holds.
/// `false` if the digest or any of the ys is already held: the caller rolls back.
async fn insert_offline_exchange(
    tx: &mut PgTransaction<'_>,
    exchange: &foreign::OfflineExchange,
    ys: &[cashu::PublicKey],
) -> Result<bool> {
    let blob = OfflineExchangeBlob::V1(OfflineExchangeBlobV1 {
        evidence_digest: exchange.evidence_digest,
        proofs: exchange
            .proofs
            .iter()
            .cloned()
            .map(ecash::Proof::from)
            .collect(),
    });
    let blob = serde_json::to_value(&blob).map_err(|e| Error::DB(anyhow!(e)))?;
    let result = sqlx::query!(
        r#"
        INSERT INTO treasury_foreign_offline_exchanges (exchange_digest, alpha_id, expires_at, blob)
        VALUES ($1, $2, $3, $4)
        ON CONFLICT (exchange_digest) DO NOTHING
        "#,
        &exchange.exchange_digest[..],
        exchange.alpha_id.to_string(),
        exchange.expires_at.unix_timestamp(),
        blob,
    )
    .execute(&mut **tx)
    .await
    .map_err(|e| Error::DB(anyhow!(e)))?;
    if result.rows_affected() != 1 {
        return Ok(false);
    }
    let y_strs: Vec<String> = ys.iter().map(|y| y.to_string()).collect();
    let result = sqlx::query!(
        r#"
        INSERT INTO treasury_foreign_offline_exchange_ys (y, exchange_digest)
        SELECT y, $2 FROM UNNEST($1::text[]) AS y
        ON CONFLICT (y) DO NOTHING
        "#,
        &y_strs,
        &exchange.exchange_digest[..],
    )
    .execute(&mut **tx)
    .await
    .map_err(|e| Error::DB(anyhow!(e)))?;
    Ok(result.rows_affected() == y_strs.len() as u64)
}

/// Inserts the fingerprints, keyed by hash.
/// `false` if any hash is already stored: the caller rolls back.
async fn insert_offline_fps(
    tx: &mut PgTransaction<'_>,
    mint_id: secp256k1::PublicKey,
    fps: Vec<wire_keys::ProofFingerprint>,
    hashes: Vec<foreign::Sha256Hash>,
) -> Result<bool> {
    let hash_strs: Vec<String> = hashes.iter().map(|h| h.to_string()).collect();
    let mut y_strs = Vec::with_capacity(fps.len());
    let mut blobs = Vec::with_capacity(fps.len());
    for fp in fps {
        y_strs.push(cashu::PublicKey::from(fp.y).to_string());
        let blob = ForeignFingerprintBlob::from(fp);
        blobs.push(serde_json::to_value(&blob).map_err(|e| Error::DB(anyhow!(e)))?);
    }
    let mint_ids = vec![mint_id.to_string(); y_strs.len()];
    let result = sqlx::query!(
        r#"
        INSERT INTO treasury_foreign_offline_fps (hash, y, mint_id, blob)
        SELECT * FROM UNNEST($1::text[], $2::text[], $3::text[], $4::jsonb[])
        ON CONFLICT (hash) DO NOTHING
        "#,
        &hash_strs,
        &y_strs,
        &mint_ids,
        &blobs,
    )
    .execute(&mut **tx)
    .await
    .map_err(|e| Error::DB(anyhow!(e)))?;
    Ok(result.rows_affected() == hash_strs.len() as u64)
}

#[derive(Debug, Clone)]
pub struct DBForeignOffline {
    pool: PgPool,
}

impl DBForeignOffline {
    pub fn from_pool(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Migration only: an exchange with the ys it holds, without the fingerprints
    /// `reserve_exchange` stores alongside, since those are removed once redeemed.
    pub async fn insert_exchange(
        &self,
        exchange: foreign::OfflineExchange,
        ys: Vec<cashu::PublicKey>,
    ) -> Result<()> {
        let mut tx = self.pool.begin().await.map_err(|e| Error::DB(anyhow!(e)))?;
        if !insert_offline_exchange(&mut tx, &exchange, &ys).await? {
            tx.rollback().await.map_err(|e| Error::DB(anyhow!(e)))?;
            return Err(Error::DB(anyhow!("exchange or its ys already stored")));
        }
        tx.commit().await.map_err(|e| Error::DB(anyhow!(e)))?;
        Ok(())
    }

    /// Migration only: a fingerprint not yet redeemed.
    pub async fn insert_fp(
        &self,
        hash: foreign::Sha256Hash,
        mint_id: secp256k1::PublicKey,
        fp: wire_keys::ProofFingerprint,
    ) -> Result<()> {
        let mut tx = self.pool.begin().await.map_err(|e| Error::DB(anyhow!(e)))?;
        if !insert_offline_fps(&mut tx, mint_id, vec![fp], vec![hash]).await? {
            tx.rollback().await.map_err(|e| Error::DB(anyhow!(e)))?;
            return Err(Error::DB(anyhow!("fingerprint already stored")));
        }
        tx.commit().await.map_err(|e| Error::DB(anyhow!(e)))?;
        Ok(())
    }
}

#[async_trait]
impl foreign::OfflineRepository for DBForeignOffline {
    async fn search_fp(
        &self,
        hash: &foreign::Sha256Hash,
    ) -> Result<Option<(secp256k1::PublicKey, wire_keys::ProofFingerprint)>> {
        let row = sqlx::query_as!(
            ForeignFingerprintRow,
            r#"
            SELECT y, mint_id, blob as "blob: Json<ForeignFingerprintBlob>"
            FROM treasury_foreign_offline_fps
            WHERE hash = $1
            "#,
            hash.to_string()
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| Error::DB(anyhow!(e)))?;
        row.map(TryFrom::try_from).transpose()
    }

    async fn remove_fps(&self, ys: &[cashu::PublicKey]) -> Result<()> {
        let y_strs: Vec<String> = ys.iter().map(|y| y.to_string()).collect();
        sqlx::query!(
            r#"
            DELETE FROM treasury_foreign_offline_fps WHERE y = ANY($1::text[])
            "#,
            &y_strs
        )
        .execute(&self.pool)
        .await
        .map_err(|e| Error::DB(anyhow!(e)))?;
        Ok(())
    }

    async fn store_proofs(
        &self,
        mint_id: secp256k1::PublicKey,
        proofs: Vec<cashu::Proof>,
    ) -> Result<()> {
        if proofs.is_empty() {
            return Ok(());
        }
        let mut ys = Vec::with_capacity(proofs.len());
        let mut blobs = Vec::with_capacity(proofs.len());
        for p in proofs {
            let y = p.y().map_err(|e| Error::DB(anyhow!(e)))?;
            ys.push(y.to_string());
            let blob = ForeignOfflineProofBlob::V1(p.into());
            blobs.push(serde_json::to_value(&blob).map_err(|e| Error::DB(anyhow!(e)))?);
        }
        let mint_ids = vec![mint_id.to_string(); ys.len()];
        let mut tx = self.pool.begin().await.map_err(|e| Error::DB(anyhow!(e)))?;
        let result = sqlx::query!(
            r#"
            INSERT INTO treasury_foreign_offline_proofs (y, mint_id, blob)
            SELECT * FROM UNNEST($1::text[], $2::text[], $3::jsonb[])
            ON CONFLICT (y) DO NOTHING
            "#,
            &ys,
            &mint_ids,
            &blobs,
        )
        .execute(&mut *tx)
        .await
        .map_err(|e| Error::DB(anyhow!(e)))?;
        if result.rows_affected() != ys.len() as u64 {
            tx.rollback().await.map_err(|e| Error::DB(anyhow!(e)))?;
            return Err(Error::InvalidInput(String::from("proofs already stored")));
        }
        tx.commit().await.map_err(|e| Error::DB(anyhow!(e)))?;
        Ok(())
    }

    async fn load_proofs(&self, mint_id: secp256k1::PublicKey) -> Result<Vec<cashu::Proof>> {
        let rows = sqlx::query!(
            r#"
            SELECT blob as "blob: Json<ForeignOfflineProofBlob>"
            FROM treasury_foreign_offline_proofs
            WHERE mint_id = $1
            "#,
            mint_id.to_string()
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| Error::DB(anyhow!(e)))?;
        let proofs = rows
            .into_iter()
            .map(|row| {
                let ForeignOfflineProofBlob::V1(proof) = row.blob.0;
                cashu::Proof::from(proof)
            })
            .collect();
        Ok(proofs)
    }

    async fn unsettled_balance(
        &self,
    ) -> Result<std::collections::HashMap<secp256k1::PublicKey, cashu::Amount>> {
        let rows = sqlx::query!(
            r#"
            SELECT mint_id, blob as "blob: Json<ForeignOfflineProofBlob>"
            FROM treasury_foreign_offline_proofs
            "#
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| Error::DB(anyhow!(e)))?;
        let mut balances: std::collections::HashMap<secp256k1::PublicKey, cashu::Amount> =
            std::collections::HashMap::new();
        for row in rows {
            let mint_id =
                secp256k1::PublicKey::from_str(&row.mint_id).map_err(|e| Error::DB(anyhow!(e)))?;
            let ForeignOfflineProofBlob::V1(proof) = row.blob.0;
            let total = balances.entry(mint_id).or_insert(cashu::Amount::ZERO);
            *total = total
                .checked_add(proof.amount)
                .ok_or_else(|| Error::DB(anyhow!("unsettled balance overflow")))?;
        }
        Ok(balances)
    }

    async fn remove_proofs(&self, ys: &[cashu::PublicKey]) -> Result<()> {
        let y_strs: Vec<String> = ys.iter().map(|y| y.to_string()).collect();
        sqlx::query!(
            r#"
            DELETE FROM treasury_foreign_offline_proofs WHERE y = ANY($1::text[])
            "#,
            &y_strs
        )
        .execute(&self.pool)
        .await
        .map_err(|e| Error::DB(anyhow!(e)))?;
        Ok(())
    }

    async fn list_foreign_pks(&self) -> Result<Vec<secp256k1::PublicKey>> {
        let rows = sqlx::query!(
            r#"
            SELECT DISTINCT mint_id FROM treasury_foreign_offline_proofs
            "#
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| Error::DB(anyhow!(e)))?;
        rows.into_iter()
            .map(|row| {
                secp256k1::PublicKey::from_str(&row.mint_id).map_err(|e| Error::DB(anyhow!(e)))
            })
            .collect()
    }

    async fn claim_redemption(&self, digest: [u8; 32]) -> Result<bool> {
        let result = sqlx::query!(
            r#"
            INSERT INTO treasury_foreign_offline_redemptions (digest)
            VALUES ($1)
            ON CONFLICT (digest) DO NOTHING
            "#,
            &digest[..],
        )
        .execute(&self.pool)
        .await
        .map_err(|e| Error::DB(anyhow!(e)))?;
        Ok(result.rows_affected() == 1)
    }

    async fn reserve_exchange(
        &self,
        exchange: foreign::OfflineExchange,
        fps: Vec<wire_keys::ProofFingerprint>,
        hashes: Vec<foreign::Sha256Hash>,
    ) -> Result<bool> {
        let ys: Vec<cashu::PublicKey> = fps.iter().map(|fp| fp.y.into()).collect();
        let mut tx = self.pool.begin().await.map_err(|e| Error::DB(anyhow!(e)))?;
        if !insert_offline_exchange(&mut tx, &exchange, &ys).await? {
            tx.rollback().await.map_err(|e| Error::DB(anyhow!(e)))?;
            return Ok(false);
        }
        if !insert_offline_fps(&mut tx, exchange.alpha_id, fps, hashes).await? {
            tx.rollback().await.map_err(|e| Error::DB(anyhow!(e)))?;
            return Err(Error::DB(anyhow!(
                "reserve exchange: fingerprint already stored"
            )));
        }
        tx.commit().await.map_err(|e| Error::DB(anyhow!(e)))?;
        Ok(true)
    }

    async fn search_exchange(
        &self,
        ys: &[cashu::PublicKey],
    ) -> Result<Option<foreign::OfflineExchange>> {
        let y_strs: Vec<String> = ys.iter().map(|y| y.to_string()).collect();
        let row = sqlx::query_as!(
            OfflineExchangeRow,
            r#"
            SELECT e.exchange_digest, e.alpha_id, e.expires_at,
                   e.blob as "blob: Json<OfflineExchangeBlob>"
            FROM treasury_foreign_offline_exchange_ys x
            JOIN treasury_foreign_offline_exchanges e ON e.exchange_digest = x.exchange_digest
            WHERE x.y = ANY($1::text[])
            LIMIT 1
            "#,
            &y_strs
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| Error::DB(anyhow!(e)))?;
        row.map(TryFrom::try_from).transpose()
    }
}

// ///////////////////////////////////////////////////////////////////////// Versioned vault proof blob

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "version", content = "data")]
enum VaultProofBlob {
    V1(ecash::Proof),
}
// ///////////////////////////////////////////////////////////////////////// DBVault

#[derive(Debug, Clone)]
pub struct DBVault {
    pool: PgPool,
}

impl DBVault {
    pub fn from_pool(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl vault::Repository for DBVault {
    async fn store_proofs(&self, proofs: Vec<cashu::Proof>) -> Result<()> {
        let mut y_strs = Vec::with_capacity(proofs.len());
        let mut blob_values = Vec::with_capacity(proofs.len());
        for proof in proofs {
            let y = proof.y().map_err(|e| Error::DB(anyhow!(e)))?;
            let blob = VaultProofBlob::V1(proof.into());
            let blob_value = serde_json::to_value(&blob).map_err(|e| Error::DB(anyhow!(e)))?;
            y_strs.push(y.to_string());
            blob_values.push(blob_value);
        }
        sqlx::query!(
            r#"
            INSERT INTO treasury_vault_proofs (y, blob)
            SELECT * FROM UNNEST($1::text[], $2::jsonb[])
            ON CONFLICT (y) DO UPDATE SET blob = EXCLUDED.blob
            "#,
            &y_strs,
            &blob_values
        )
        .execute(&self.pool)
        .await
        .map_err(|e| Error::DB(anyhow!(e)))?;
        Ok(())
    }

    async fn load_proofs(&self, ys: Vec<cashu::PublicKey>) -> Result<Vec<cashu::Proof>> {
        let y_strs: Vec<String> = ys.into_iter().map(|y| y.to_string()).collect();
        let results = sqlx::query(
            r#"
            SELECT blob FROM treasury_vault_proofs WHERE y = ANY($1::text[])
            "#,
        )
        .bind(&y_strs)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| Error::DB(anyhow!(e)))?;
        let proofs = results
            .into_iter()
            .map(|row| {
                let blob: sqlx::types::Json<VaultProofBlob> = row.get("blob");
                match blob.0 {
                    VaultProofBlob::V1(proof) => cashu::Proof::from(proof),
                }
            })
            .collect();
        Ok(proofs)
    }

    async fn list_ys(&self) -> Result<Vec<cashu::PublicKey>> {
        let results = sqlx::query!(
            r#"
            SELECT y FROM treasury_vault_proofs
            "#
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| Error::DB(anyhow!(e)))?;
        let mut ys = Vec::with_capacity(results.len());
        for row in results {
            let y = cashu::PublicKey::from_str(&row.y).map_err(|e| Error::DB(anyhow!(e)))?;
            ys.push(y);
        }
        Ok(ys)
    }

    async fn delete_proofs(&self, ys: &[cashu::PublicKey]) -> Result<()> {
        let y_strs: Vec<String> = ys.iter().map(|y| y.to_string()).collect();
        sqlx::query(
            r#"
            DELETE FROM treasury_vault_proofs WHERE y = ANY($1::text[])
            "#,
        )
        .bind(&y_strs)
        .execute(&self.pool)
        .await
        .map_err(|e| Error::DB(anyhow!(e)))?;
        Ok(())
    }
}

// ///////////////////////////////////////////////////////////////////////// Versioned onchain blobs

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "version", content = "data")]
enum OnChainMintOpBlob {
    V1(OnChainMintOpBlobV1),
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct OnChainMintOpBlobV1 {
    blinds: Option<Vec<ecash::BlindedMessage>>,
    signatures: Option<Vec<ecash::BlindSignature>>,
    recipient: bitcoin::Address<bitcoin::address::NetworkUnchecked>,
    kid: ecash::Id,
    target: bitcoin::Amount,
}

fn onchain_mintop_to_row(
    op: onchain::MintOperation,
) -> Result<(
    Uuid,
    onchain::MintStatusDiscriminants,
    TStamp,
    serde_json::Value,
)> {
    let mut blob_v1 = OnChainMintOpBlobV1 {
        blinds: None,
        signatures: None,
        recipient: op.recipient,
        kid: ecash::Id::from(op.kid),
        target: op.target,
    };
    let status = op.status.discriminant();
    match op.status {
        onchain::MintStatus::Pending { blinds } => {
            blob_v1.blinds = Some(blinds.into_iter().map(Into::into).collect());
        }
        onchain::MintStatus::Paid { signatures } => {
            blob_v1.signatures = Some(signatures.into_iter().map(Into::into).collect());
        }
        _ => {}
    };
    let blob = OnChainMintOpBlob::V1(blob_v1);
    let blob_value = serde_json::to_value(&blob).map_err(|e| Error::DB(anyhow!(e)))?;
    Ok((op.qid, status, op.expiry, blob_value))
}

fn onchain_mintop_from_row(
    qid: Uuid,
    op_status: onchain::MintStatusDiscriminants,
    expiry: TStamp,
    blob: OnChainMintOpBlob,
) -> Result<onchain::MintOperation> {
    let (recipient, kid, target, blinds, signatures) = match blob {
        OnChainMintOpBlob::V1(blob) => (
            blob.recipient,
            blob.kid,
            blob.target,
            blob.blinds,
            blob.signatures,
        ),
    };
    let status = match op_status {
        onchain::MintStatusDiscriminants::Pending => {
            let blinds = blinds
                .ok_or_else(|| Error::DB(anyhow!("Missing blinds in Pending mintop blob")))?;
            onchain::MintStatus::Pending {
                blinds: blinds.into_iter().map(Into::into).collect(),
            }
        }
        onchain::MintStatusDiscriminants::Paid => {
            let signatures = signatures
                .ok_or_else(|| Error::DB(anyhow!("Missing signatures in Paid mintop blob")))?;
            onchain::MintStatus::Paid {
                signatures: signatures.into_iter().map(Into::into).collect(),
            }
        }
        onchain::MintStatusDiscriminants::Expired => onchain::MintStatus::Expired,
    };
    let op = onchain::MintOperation {
        qid,
        expiry,
        status,
        recipient,
        kid: cashu::Id::from(kid),
        target,
    };
    Ok(op)
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "version", content = "data")]
enum OnChainMeltOpBlob {
    V1(OnChainMeltOpBlobV1),
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct OnChainMeltOpBlobV1 {
    pub address: String,
    pub target: bitcoin::Amount,
    pub available: bitcoin::Amount,
    pub fees: bitcoin::Amount,
    // network fees = available - target - fees
    pub wallet_key: secp256k1::PublicKey,
    pub fp_digest: [u8; 32],
    pub commitment: secp256k1::schnorr::Signature,
    pub tx: Option<bitcoin::Txid>,
}

fn onchain_meltop_to_row(
    op: onchain::MeltOperation,
) -> Result<(
    Uuid,
    onchain::MeltStatusDiscriminants,
    TStamp,
    Vec<String>,
    serde_json::Value,
)> {
    let mut blob_v1 = OnChainMeltOpBlobV1 {
        address: op.address,
        available: bitcoin::Amount::from_sat(op.available.to_u64()),
        target: op.target,
        commitment: op.commitment,
        fees: bitcoin::Amount::from_sat(op.fees.to_u64()),
        wallet_key: *op.wallet_key.deref(),
        fp_digest: op.fp_digest,
        tx: None,
    };
    let status = op.status.discriminant();
    if let onchain::MeltStatus::Paid { tx } = op.status {
        blob_v1.tx = Some(tx);
    }
    let ys = op.input_ys.iter().map(|y| y.to_string()).collect();
    let blob = OnChainMeltOpBlob::V1(blob_v1);
    let blob_value = serde_json::to_value(&blob).map_err(|e| Error::DB(anyhow!(e)))?;
    Ok((op.qid, status, op.expiry, ys, blob_value))
}

fn onchain_meltop_from_row(
    qid: Uuid,
    op_status: onchain::MeltStatusDiscriminants,
    expiry: TStamp,
    ys: Vec<String>,
    blob: OnChainMeltOpBlob,
) -> Result<onchain::MeltOperation> {
    let input_ys: Vec<cashu::PublicKey> = ys
        .into_iter()
        .map(|y_str| cashu::PublicKey::from_str(&y_str).map_err(|e| Error::DB(anyhow!(e))))
        .collect::<Result<_>>()?;
    let (address, target, available, fees, wallet_key, fp_digest, commitment, tx) = match blob {
        OnChainMeltOpBlob::V1(blob) => (
            blob.address,
            blob.target,
            blob.available,
            blob.fees,
            blob.wallet_key,
            blob.fp_digest,
            blob.commitment,
            blob.tx,
        ),
    };
    let status = match op_status {
        onchain::MeltStatusDiscriminants::Pending => onchain::MeltStatus::Pending,
        onchain::MeltStatusDiscriminants::Paid => {
            if tx.is_none() {
                return Err(Error::DB(anyhow!("Missing tx in Paid meltop blob")));
            }
            onchain::MeltStatus::Paid { tx: tx.unwrap() }
        }
        onchain::MeltStatusDiscriminants::Expired => onchain::MeltStatus::Expired,
        onchain::MeltStatusDiscriminants::Canceled => onchain::MeltStatus::Canceled,
    };
    let op = onchain::MeltOperation {
        qid,
        address,
        target,
        available: cashu::Amount::from(available.to_sat()),
        fees: cashu::Amount::from(fees.to_sat()),
        wallet_key: cashu::PublicKey::from(wallet_key),
        commitment,
        fp_digest,
        expiry,
        input_ys,
        status,
    };
    Ok(op)
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "version", content = "data")]
enum OnChainDeniedMeltOpBlob {
    V1(OnChainDeniedMeltOpBlobV1),
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct OnChainDeniedMeltOpBlobV1 {
    pub inputs: bitcoin::Amount,
    #[serde(with = "time::serde::rfc3339")]
    pub created: TStamp,
}

// ///////////////////////////////////////////////////////////////////////// DBOnChain

#[derive(Debug, Clone)]
pub struct DBOnChain {
    pool: PgPool,
}

impl DBOnChain {
    pub fn from_pool(pool: PgPool) -> Self {
        Self { pool }
    }

    async fn mintops_mark_expired(&self, now: TStamp) -> Result<()> {
        sqlx::query!(
            r#"
            UPDATE treasury_onchain_mint_ops
            SET status = $2
            WHERE status != $2 AND expiry < $1
            "#,
            now,
            onchain::MintStatusDiscriminants::Expired.to_string(),
        )
        .execute(&self.pool)
        .await
        .map_err(|e| Error::DB(anyhow!(e)))?;
        Ok(())
    }

    async fn meltops_mark_expired(&self, now: TStamp) -> Result<()> {
        sqlx::query!(
            r#"
            UPDATE treasury_onchain_melt_ops
            SET status = $2
            WHERE status != $2 AND expiry < $1
            "#,
            now,
            onchain::MeltStatusDiscriminants::Expired.to_string(),
        )
        .execute(&self.pool)
        .await
        .map_err(|e| Error::DB(anyhow!(e)))?;
        Ok(())
    }
}

#[async_trait]
impl onchain::Repository for DBOnChain {
    async fn store_mintop(&self, op: onchain::MintOperation) -> Result<()> {
        let (qid, status, expiry, blob) = onchain_mintop_to_row(op)?;
        let result = sqlx::query!(
            r#"
            INSERT INTO treasury_onchain_mint_ops (qid, expiry, status, blob)
            VALUES ($1, $2, $3, $4)
            ON CONFLICT (qid) DO NOTHING
            RETURNING qid
            "#,
            qid,
            expiry,
            status.to_string(),
            blob
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| Error::DB(anyhow!(e)))?;
        if result.is_none() {
            return Err(Error::InvalidInput(format!(
                "mintop already exists {}",
                qid
            )));
        }
        Ok(())
    }

    async fn load_mintop(&self, qid: Uuid) -> Result<onchain::MintOperation> {
        let result = sqlx::query!(
            r#"
            SELECT qid, expiry, status, blob as "blob: Json<OnChainMintOpBlob>"
            FROM treasury_onchain_mint_ops
            WHERE qid = $1
            "#,
            qid
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| Error::DB(anyhow!(e)))?;
        let Some(row) = result else {
            return Err(Error::ResourceNotFound(qid.to_string()));
        };
        let status = onchain::MintStatusDiscriminants::from_str(&row.status)
            .map_err(|e| Error::DB(anyhow!(e)))?;
        let op = onchain_mintop_from_row(row.qid, status, row.expiry, row.blob.0)?;
        Ok(op)
    }

    async fn list_pending_mintops(&self, now: TStamp) -> Result<Vec<Uuid>> {
        self.mintops_mark_expired(now).await?;
        let results = sqlx::query!(
            r#"
            SELECT qid FROM treasury_onchain_mint_ops WHERE status = $1
            "#,
            onchain::MintStatusDiscriminants::Pending.to_string(),
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| Error::DB(anyhow!(e)))?;
        Ok(results.into_iter().map(|r| r.qid).collect())
    }

    async fn update_mintop_status(&self, qid: Uuid, op_status: onchain::MintStatus) -> Result<()> {
        let rows_affected = match op_status {
            onchain::MintStatus::Pending { .. } => {
                return Err(Error::InvalidInput(format!(
                    "Cannot update mintop {qid} to Pending status"
                )));
            }
            onchain::MintStatus::Paid { signatures } => {
                let signatures: Vec<ecash::BlindSignature> =
                    signatures.into_iter().map(Into::into).collect();
                let sigs_val =
                    serde_json::to_value(&signatures).map_err(|e| Error::DB(anyhow!(e)))?;
                sqlx::query!(
                    r#"
                    UPDATE treasury_onchain_mint_ops
                    SET status = $3,
                        blob = jsonb_set(blob, '{data,signatures}', to_jsonb($2::jsonb), true)
                    WHERE qid = $1 AND blob->>'version' = 'V1'
                    "#,
                    qid,
                    sigs_val,
                    onchain::MintStatusDiscriminants::Paid.to_string(),
                )
                .execute(&self.pool)
                .await
                .map_err(|e| Error::DB(anyhow!(e)))?
                .rows_affected()
            }
            onchain::MintStatus::Expired => sqlx::query!(
                r#"
                    UPDATE treasury_onchain_mint_ops
                    SET status = $2
                    WHERE qid = $1 AND blob->>'version' = 'V1'
                    "#,
                qid,
                onchain::MintStatusDiscriminants::Expired.to_string(),
            )
            .execute(&self.pool)
            .await
            .map_err(|e| Error::DB(anyhow!(e)))?
            .rows_affected(),
        };
        if rows_affected == 0 {
            return Err(Error::ResourceNotFound(qid.to_string()));
        }
        Ok(())
    }

    async fn store_meltop(&self, op: onchain::MeltOperation, now: TStamp) -> Result<()> {
        self.meltops_mark_expired(now).await?;
        let (qid, status, expiry, ys, blob) = onchain_meltop_to_row(op)?;
        let result = sqlx::query!(
            r#"
            INSERT INTO treasury_onchain_melt_ops (qid, expiry, status, input_ys, blob)
            VALUES( $1, $2, $3, $4, $5)
            RETURNING qid
            "#,
            qid,
            expiry,
            status.to_string(),
            &ys,
            &blob
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| Error::DB(anyhow!(e)))?;
        if result.is_none() {
            return Err(Error::InvalidInput(String::from(
                "meltop: inputs already locked in",
            )));
        }
        Ok(())
    }

    async fn load_meltop(&self, qid: Uuid) -> Result<onchain::MeltOperation> {
        let result = sqlx::query!(
            r#"
            SELECT qid, expiry, status, input_ys, blob as "blob: Json<OnChainMeltOpBlob>"
            FROM treasury_onchain_melt_ops
            WHERE qid = $1
            "#,
            qid
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| Error::DB(anyhow!(e)))?;
        let Some(row) = result else {
            return Err(Error::ResourceNotFound(qid.to_string()));
        };
        let status = onchain::MeltStatusDiscriminants::from_str(&row.status)
            .map_err(|e| Error::DB(anyhow!(e)))?;
        let op = onchain_meltop_from_row(row.qid, status, row.expiry, row.input_ys, row.blob.0)?;
        Ok(op)
    }

    async fn update_meltop_status(&self, qid: Uuid, status: onchain::MeltStatus) -> Result<()> {
        let rows_affected = match status {
            onchain::MeltStatus::Pending => {
                return Err(Error::InvalidInput(format!(
                    "Cannot update meltop {qid} to Pending status"
                )));
            }
            onchain::MeltStatus::Paid { tx } => sqlx::query!(
                r#"
                UPDATE treasury_onchain_melt_ops
                SET status = $3,
                    blob = jsonb_set(blob, '{data,tx}', to_jsonb($2::text), true)
                WHERE qid = $1 AND blob->>'version' = 'V1'
                "#,
                qid,
                tx.to_string(),
                onchain::MeltStatusDiscriminants::Paid.to_string()
            )
            .execute(&self.pool)
            .await
            .map_err(|e| Error::DB(anyhow!(e)))?
            .rows_affected(),
            onchain::MeltStatus::Expired => sqlx::query!(
                r#"
                UPDATE treasury_onchain_melt_ops
                SET status = $2
                WHERE qid = $1 AND blob->>'version' = 'V1'
                "#,
                qid,
                onchain::MeltStatusDiscriminants::Expired.to_string(),
            )
            .execute(&self.pool)
            .await
            .map_err(|e| Error::DB(anyhow!(e)))?
            .rows_affected(),
            onchain::MeltStatus::Canceled => sqlx::query!(
                r#"
                UPDATE treasury_onchain_melt_ops
                SET status = $2
                WHERE qid = $1 AND blob->>'version' = 'V1'
                "#,
                qid,
                onchain::MeltStatusDiscriminants::Canceled.to_string()
            )
            .execute(&self.pool)
            .await
            .map_err(|e| Error::DB(anyhow!(e)))?
            .rows_affected(),
        };
        if rows_affected == 0 {
            return Err(Error::ResourceNotFound(qid.to_string()));
        }
        Ok(())
    }

    async fn list_pending_meltops(&self, now: TStamp) -> Result<Vec<Uuid>> {
        self.meltops_mark_expired(now).await?;
        let results = sqlx::query!(
            r#"
            SELECT qid FROM treasury_onchain_melt_ops WHERE status = $1
            "#,
            onchain::MeltStatusDiscriminants::Pending.to_string(),
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| Error::DB(anyhow!(e)))?;
        Ok(results.into_iter().map(|r| r.qid).collect())
    }

    async fn store_denied_meltop(&self, op: onchain::DeniedMeltOperation) -> Result<()> {
        let blob = OnChainDeniedMeltOpBlob::V1(OnChainDeniedMeltOpBlobV1 {
            inputs: op.inputs,
            created: op.created,
        });
        let blob_value = serde_json::to_value(&blob).map_err(|e| Error::DB(anyhow!(e)))?;
        let result = sqlx::query!(
            r#"
            INSERT INTO treasury_onchain_denied_melt_ops (qid, blob)
            VALUES ($1, $2)
            ON CONFLICT (qid) DO NOTHING
            RETURNING qid
            "#,
            op.qid,
            blob_value
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| Error::DB(anyhow!(e)))?;
        if result.is_none() {
            return Err(Error::InvalidInput(format!(
                "denied meltop already exists {}",
                op.qid
            )));
        }
        Ok(())
    }

    async fn list_denied_meltops(&self) -> Result<Vec<onchain::DeniedMeltOperation>> {
        let results = sqlx::query!(
            r#"
            SELECT qid, blob as "blob: Json<OnChainDeniedMeltOpBlob>"
            FROM treasury_onchain_denied_melt_ops
            "#
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| Error::DB(anyhow!(e)))?;
        let mut ops = Vec::with_capacity(results.len());
        for row in results {
            let OnChainDeniedMeltOpBlob::V1(blob) = row.blob.0;
            ops.push(onchain::DeniedMeltOperation {
                qid: row.qid,
                inputs: blob.inputs,
                created: blob.created,
            });
        }
        Ok(ops)
    }

    async fn delete_denied_meltop(&self, qid: Uuid) -> Result<()> {
        let result = sqlx::query!(
            r#"
            DELETE FROM treasury_onchain_denied_melt_ops WHERE qid = $1
            "#,
            qid
        )
        .execute(&self.pool)
        .await
        .map_err(|e| Error::DB(anyhow!(e)))?;
        if result.rows_affected() == 0 {
            return Err(Error::ResourceNotFound(qid.to_string()));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bcr_common::core_tests;

    fn random_proof() -> cashu::Proof {
        let (_, keyset) = core_tests::generate_random_ecash_keyset();
        let amounts = vec![cashu::Amount::from(8u64)];
        core_tests::generate_random_ecash_proofs(&keyset, &amounts)
            .pop()
            .unwrap()
    }

    #[test]
    fn proof_blobs_serialize_like_cashu() {
        let proof = random_proof();
        let cashu_json = serde_json::to_value(&proof).unwrap();

        let stored = serde_json::to_value(ForeignProofBlob::V1(proof.clone().into())).unwrap();
        assert_eq!(stored["data"], cashu_json);
        let ForeignProofBlob::V1(back) = serde_json::from_value(stored).unwrap();
        assert_eq!(cashu::Proof::from(back), proof);

        let stored = serde_json::to_value(ForeignHtlcProofBlob::V1(proof.clone().into())).unwrap();
        assert_eq!(stored["data"], cashu_json);
        let ForeignHtlcProofBlob::V1(back) = serde_json::from_value(stored).unwrap();
        assert_eq!(cashu::Proof::from(back), proof);

        let stored = serde_json::to_value(VaultProofBlob::V1(proof.clone().into())).unwrap();
        assert_eq!(stored["data"], cashu_json);
        let VaultProofBlob::V1(back) = serde_json::from_value(stored).unwrap();
        assert_eq!(cashu::Proof::from(back), proof);
    }

    #[test]
    fn mintop_blob_serializes_like_cashu() {
        let proof = random_proof();
        let blind = cashu::BlindedMessage::new(proof.amount, proof.keyset_id, proof.c);
        let signature = cashu::BlindSignature {
            amount: proof.amount,
            keyset_id: proof.keyset_id,
            c: proof.c,
            dleq: None,
        };
        let recipient =
            bitcoin::Address::from_str("bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq").unwrap();
        let expiry = TStamp::from_unix_timestamp(1_700_000_000).unwrap();
        let ops = [
            (
                onchain::MintStatus::Pending {
                    blinds: vec![blind.clone()],
                },
                "blinds",
                serde_json::to_value(vec![blind]).unwrap(),
            ),
            (
                onchain::MintStatus::Paid {
                    signatures: vec![signature.clone()],
                },
                "signatures",
                serde_json::to_value(vec![signature]).unwrap(),
            ),
        ];
        for (status, field, cashu_json) in ops {
            let op = onchain::MintOperation {
                qid: Uuid::new_v4(),
                expiry,
                status,
                recipient: recipient.clone(),
                kid: proof.keyset_id,
                target: bitcoin::Amount::from_sat(8),
            };
            let (qid, discriminant, expiry, blob) = onchain_mintop_to_row(op.clone()).unwrap();
            assert_eq!(blob["data"][field], cashu_json);
            let blob: OnChainMintOpBlob = serde_json::from_value(blob).unwrap();
            let back = onchain_mintop_from_row(qid, discriminant, expiry, blob).unwrap();
            assert_eq!(
                serde_json::to_value(back.status).unwrap(),
                serde_json::to_value(op.status).unwrap()
            );
        }
    }
}
