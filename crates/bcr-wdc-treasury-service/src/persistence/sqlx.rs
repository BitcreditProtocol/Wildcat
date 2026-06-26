#![allow(dead_code)]
// ----- standard library imports
use std::str::FromStr;
// ----- extra library imports
use anyhow::anyhow;
use async_trait::async_trait;
use bcr_common::{cashu, core::BillId};
use bcr_wdc_utils::postgres;
use sqlx::types::Json;
use sqlx::PgPool;
use uuid::Uuid;
// ----- local imports
use crate::{
    ebill,
    error::{Error, Result},
};

// ----- end imports

// ///////////////////////////////////////////////////////////////////////// Versioned blob

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "version", content = "data")]
enum MintOperationBlob {
    V1(MintOpBlobV1),
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct MintOpBlobV1 {
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
        let blob = MintOperationBlob::V1(MintOpBlobV1 {
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
        .map_err(|e| Error::DB(anyhow!(e)))?;
        if result.is_none() {
            return Err(Error::InvalidInput(format!("mintop already exists {uid}")));
        }
        Ok(())
    }

    async fn mint_load(&self, uid: Uuid) -> Result<ebill::MintOperation> {
        let result = sqlx::query!(
            r#"
            SELECT uid, kid, minted, blob as "blob: Json<MintOperationBlob>"
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
        let MintOperationBlob::V1(v1) = row.blob.0;
        Ok(ebill::MintOperation {
            uid: row.uid,
            kid,
            minted: cashu::Amount::from(row.minted as u64),
            bill_id: v1.bill_id,
            target: v1.target,
            pub_key: v1.pub_key,
        })
    }

    async fn mint_list(&self, kid: cashu::Id) -> Result<Vec<ebill::MintOperation>> {
        let results = sqlx::query!(
            r#"
            SELECT uid, kid, minted, blob as "blob: Json<MintOperationBlob>"
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
            let MintOperationBlob::V1(v1) = row.blob.0;
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
