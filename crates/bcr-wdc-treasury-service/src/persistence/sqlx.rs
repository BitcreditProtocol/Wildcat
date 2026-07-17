// ----- standard library imports
use std::str::FromStr;
// ----- extra library imports
use anyhow::anyhow;
use async_trait::async_trait;
use bcr_common::{cashu, core::BillId};
use bcr_wdc_utils::postgres;
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
    bill_id: BillId,
    target: cashu::Amount,
    pub_key: cashu::PublicKey,
}

// ///////////////////////////////////////////////////////////////////////// DBEbill

#[derive(Debug, Clone)]
pub struct DBEbill {
    pool: PgPool,
}

impl DBEbill {
    pub async fn new(cfg: postgres::DBConnConfig) -> Result<Self> {
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(cfg.max_connections)
            .connect(&cfg.connection)
            .await
            .map_err(|e| Error::DB(anyhow!(e)))?;
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .map_err(|e| Error::DB(anyhow!(e)))?;
        Ok(Self { pool })
    }

    pub fn from_pool(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl ebill::Repository for DBEbill {
    async fn mint_store(&self, mint_op: ebill::MintOperation) -> Result<()> {
        let uid = mint_op.uid;
        let blob = EbillMintOperationBlob::V1(EbillMintOpBlobV1 {
            bill_id: mint_op.bill_id,
            target: mint_op.target,
            pub_key: mint_op.pub_key,
        });
        let blob_value = serde_json::to_value(&blob).map_err(|e| Error::DB(anyhow!(e)))?;
        let result = sqlx::query!(
            r#"
            INSERT INTO mint_ops (uid, kid, minted, blob)
            VALUES ($1, $2, $3, $4)
            ON CONFLICT (uid) DO NOTHING
            RETURNING uid
            "#,
            uid,
            mint_op.kid.to_string(),
            mint_op.minted.to_u64() as i64,
            blob_value
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| match e.as_database_error() {
            Some(db) if db.is_unique_violation() => {
                Error::AlreadyExists(format!("mintop {uid}"))
            }
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
            SELECT uid, kid, minted, blob as "blob: Json<EbillMintOperationBlob>"
            FROM mint_ops
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
        let EbillMintOperationBlob::V1(v1) = row.blob.0;
        Ok(ebill::MintOperation {
            uid: row.uid,
            kid,
            minted: cashu::Amount::from(row.minted as u64),
            bill_id: v1.bill_id,
            target: v1.target,
            pub_key: v1.pub_key,
        })
    }

    async fn mint_lookup_by_bill(&self, bill_id: BillId) -> Result<Option<ebill::MintOperation>> {
        let result = sqlx::query!(
            r#"
            SELECT uid, kid, minted, blob as "blob: Json<MintOperationBlob>"
            FROM mint_ops
            WHERE blob->>'version' = 'V1' AND blob->'data'->>'bill_id' = $1
            LIMIT 1
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
        let MintOperationBlob::V1(v1) = row.blob.0;
        Ok(Some(ebill::MintOperation {
            uid: row.uid,
            kid,
            minted: cashu::Amount::from(row.minted as u64),
            bill_id: v1.bill_id,
            target: v1.target,
            pub_key: v1.pub_key,
        }))
    }

    async fn mint_list(&self, kid: cashu::Id) -> Result<Vec<ebill::MintOperation>> {
        let results = sqlx::query!(
            r#"
            SELECT uid, kid, minted, blob as "blob: Json<EbillMintOperationBlob>"
            FROM mint_ops
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
            let EbillMintOperationBlob::V1(v1) = row.blob.0;
            ops.push(ebill::MintOperation {
                uid: row.uid,
                kid,
                minted: cashu::Amount::from(row.minted as u64),
                bill_id: v1.bill_id,
                target: v1.target,
                pub_key: v1.pub_key,
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
            UPDATE mint_ops
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
    V1(cashu::Proof),
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "version", content = "data")]
enum ForeignHtlcProofBlob {
    V1(cashu::Proof),
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
    pub async fn new(cfg: postgres::DBConnConfig) -> Result<Self> {
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(cfg.max_connections)
            .connect(&cfg.connection)
            .await
            .map_err(|e| Error::DB(anyhow!(e)))?;
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .map_err(|e| Error::DB(anyhow!(e)))?;
        Ok(Self { pool })
    }

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
            let blob = ForeignProofBlob::V1(proof);
            let jason = serde_json::to_string(&blob).map_err(|e| Error::DB(anyhow!(e)))?;
            blobs.push(jason);
        }
        sqlx::query!(
            r#"
            INSERT INTO foreign_proofs (mint_id, blobs)
            VALUES ($1, $2)
            ON CONFLICT (mint_id) DO UPDATE
            SET blobs = array_cat(foreign_proofs.blobs, EXCLUDED.blobs)
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
            FROM foreign_proofs
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
                    proofs.push(proof);
                }
            }
        }
        Ok(proofs)
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
            let blob = ForeignHtlcProofBlob::V1(p);
            let jason = serde_json::to_value(&blob).map_err(|e| Error::DB(anyhow!(e)))?;
            blobs.push(jason);
        }
        let mint_ids = vec![mint_id; ys.len()];
        let hashes = vec![hash; ys.len()];
        let mut tx = self.pool.begin().await.map_err(|e| Error::DB(anyhow!(e)))?;
        let result = sqlx::query!(
            r#"
            INSERT INTO foreign_online_htlc_proofs ( y, hash, mint_id, blob)
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
            FROM foreign_online_htlc_proofs
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
            proofs.push((mint_id, proof))
        }
        Ok(proofs)
    }

    async fn remove_htlcs(&self, ys: &[cashu::PublicKey]) -> Result<()> {
        let y_strs: Vec<String> = ys.iter().map(|y| y.to_string()).collect();
        sqlx::query!(
            r#"
            DELETE FROM foreign_online_htlc_proofs WHERE y = ANY($1::text[])
            "#,
            &y_strs
        )
        .execute(&self.pool)
        .await
        .map_err(|e| Error::DB(anyhow!(e)))?;
        Ok(())
    }
}

// ///////////////////////////////////////////////////////////////////////// Versioned vault proof blob

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "version", content = "data")]
enum VaultProofBlob {
    V1(cashu::Proof),
}
// ///////////////////////////////////////////////////////////////////////// DBVault

#[derive(Debug, Clone)]
pub struct DBVault {
    pool: PgPool,
}

impl DBVault {
    pub async fn new(cfg: postgres::DBConnConfig) -> Result<Self> {
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(cfg.max_connections)
            .connect(&cfg.connection)
            .await
            .map_err(|e| Error::DB(anyhow!(e)))?;
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .map_err(|e| Error::DB(anyhow!(e)))?;
        Ok(Self { pool })
    }

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
            let blob = VaultProofBlob::V1(proof);
            let blob_value = serde_json::to_value(&blob).map_err(|e| Error::DB(anyhow!(e)))?;
            y_strs.push(y.to_string());
            blob_values.push(blob_value);
        }
        sqlx::query!(
            r#"
            INSERT INTO vault_proofs (y, blob)
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
            SELECT blob FROM vault_proofs WHERE y = ANY($1::text[])
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
                    VaultProofBlob::V1(proof) => proof,
                }
            })
            .collect();
        Ok(proofs)
    }

    async fn list_ys(&self) -> Result<Vec<cashu::PublicKey>> {
        let results = sqlx::query!(
            r#"
            SELECT y FROM vault_proofs
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
            DELETE FROM vault_proofs WHERE y = ANY($1::text[])
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
    blinds: Option<Vec<cashu::BlindedMessage>>,
    signatures: Option<Vec<cashu::BlindSignature>>,
    recipient: bitcoin::Address<bitcoin::address::NetworkUnchecked>,
    kid: cashu::Id,
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
        kid: op.kid,
        target: op.target,
    };
    let status = op.status.discriminant();
    match op.status {
        onchain::MintStatus::Pending { blinds } => {
            blob_v1.blinds = Some(blinds);
        }
        onchain::MintStatus::Paid { signatures } => {
            blob_v1.signatures = Some(signatures);
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
            onchain::MintStatus::Pending { blinds }
        }
        onchain::MintStatusDiscriminants::Paid => {
            let signatures = signatures
                .ok_or_else(|| Error::DB(anyhow!("Missing signatures in Paid mintop blob")))?;
            onchain::MintStatus::Paid { signatures }
        }
        onchain::MintStatusDiscriminants::Expired => onchain::MintStatus::Expired,
    };
    let op = onchain::MintOperation {
        qid,
        expiry,
        status,
        recipient,
        kid,
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
    pub available: cashu::Amount,
    pub fees: cashu::Amount,
    // network fees = available - target - fees
    pub wallet_key: cashu::PublicKey,
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
        available: op.available,
        target: op.target,
        commitment: op.commitment,
        fees: op.fees,
        wallet_key: op.wallet_key,
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
        available,
        fees,
        wallet_key,
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
    pub created: TStamp,
}

// ///////////////////////////////////////////////////////////////////////// DBOnChain

#[derive(Debug, Clone)]
pub struct DBOnChain {
    pool: PgPool,
}

impl DBOnChain {
    pub async fn new(cfg: postgres::DBConnConfig) -> Result<Self> {
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(cfg.max_connections)
            .connect(&cfg.connection)
            .await
            .map_err(|e| Error::DB(anyhow!(e)))?;
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .map_err(|e| Error::DB(anyhow!(e)))?;
        Ok(Self { pool })
    }

    pub fn from_pool(pool: PgPool) -> Self {
        Self { pool }
    }

    async fn mintops_mark_expired(&self, now: TStamp) -> Result<()> {
        sqlx::query!(
            r#"
            UPDATE onchain_mint_ops
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
            UPDATE onchain_melt_ops
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
            INSERT INTO onchain_mint_ops (qid, expiry, status, blob)
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
            FROM onchain_mint_ops
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
            SELECT qid FROM onchain_mint_ops WHERE status = $1
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
                let sigs_val =
                    serde_json::to_value(&signatures).map_err(|e| Error::DB(anyhow!(e)))?;
                sqlx::query!(
                    r#"
                    UPDATE onchain_mint_ops
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
                    UPDATE onchain_mint_ops
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
        let result = sqlx::query(
            r#"
            INSERT INTO onchain_melt_ops (qid, expiry, status, input_ys, blob)
            VALUES( $1, $2, $3, $4, $5)
            RETURNING qid
            "#,
        )
        .bind(qid)
        .bind(expiry)
        .bind(status.to_string())
        .bind(&ys)
        .bind(&blob)
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
            FROM onchain_melt_ops
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
                UPDATE onchain_melt_ops
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
                UPDATE onchain_melt_ops
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
                UPDATE onchain_melt_ops
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
            SELECT qid FROM onchain_melt_ops WHERE status = $1
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
            INSERT INTO onchain_denied_melt_ops (qid, blob)
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
            FROM onchain_denied_melt_ops
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
            DELETE FROM onchain_denied_melt_ops WHERE qid = $1
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
