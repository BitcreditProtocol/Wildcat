// ----- standard library imports
// ----- extra library imports
use anyhow::anyhow;
use async_trait::async_trait;
use bcr_common::{cashu, client::admin::core::BRError};
use bcr_wdc_utils::postgres;
use sqlx::types::Json;
use sqlx::PgPool;
// ----- local imports
use crate::{
    error::{Error, Result},
    persistence,
};

// ----- end imports

// ///////////////////////////////////////////////////////////////////////// Versioned blob for signatures

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "version", content = "data")]
enum SignatureBlob {
    V1(cashu::BlindSignature),
}

// ///////////////////////////////////////////////////////////////////////// DBSignatures

#[derive(Debug, Clone)]
pub struct DBSignatures {
    pool: PgPool,
}

impl DBSignatures {
    pub async fn new(cfg: postgres::DBConnConfig) -> Result<Self> {
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(cfg.max_connections)
            .connect(&cfg.connection)
            .await
            .map_err(|e| Error::SignaturesRepository(anyhow!(e)))?;
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .map_err(|e| Error::SignaturesRepository(anyhow!(e)))?;
        Ok(Self { pool })
    }

    pub fn from_pool(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl persistence::SignaturesRepository for DBSignatures {
    async fn store(&self, y: cashu::PublicKey, signature: cashu::BlindSignature) -> Result<()> {
        let blob = SignatureBlob::V1(signature);
        let blob_value =
            serde_json::to_value(&blob).map_err(|e| Error::SignaturesRepository(anyhow!(e)))?;
        let result = sqlx::query!(
            r#"
            INSERT INTO signatures (y, blob)
            VALUES ($1, $2)
            ON CONFLICT (y) DO NOTHING
            RETURNING y
            "#,
            y.to_string(),
            blob_value
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| Error::SignaturesRepository(anyhow!(e)))?;
        if result.is_none() {
            return Err(Error::Conflict(format!("signature already exists: {y}")));
        }
        Ok(())
    }

    async fn load(&self, blind: &cashu::BlindedMessage) -> Result<Option<cashu::BlindSignature>> {
        let result = sqlx::query!(
            r#"
            SELECT blob as "blob: Json<SignatureBlob>"
            FROM signatures
            WHERE y = $1
            "#,
            blind.blinded_secret.to_string()
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| Error::SignaturesRepository(anyhow!(e)))?;
        let Some(row) = result else {
            return Ok(None);
        };
        match row.blob.0 {
            SignatureBlob::V1(signature) => Ok(Some(signature)),
        }
    }
}

// ///////////////////////////////////////////////////////////////////////// Versioned blob for proofs

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "version", content = "data")]
enum ProofBlob {
    V0 {
        kid: cashu::Id,
        witness: Option<cashu::Witness>,
        c: cashu::PublicKey,
        secret: cashu::secret::Secret,
    },
    V1(cashu::Proof),
}

// ///////////////////////////////////////////////////////////////////////// DBProofs

#[derive(Debug, Clone)]
pub struct DBProofs {
    pool: PgPool,
}

impl DBProofs {
    //https://www.postgresql.org/docs/current/errcodes-appendix.html
    const PG_UNIQUE_VIOLATION: &str = "23505";

    pub async fn new(cfg: postgres::DBConnConfig) -> Result<Self> {
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(cfg.max_connections)
            .connect(&cfg.connection)
            .await
            .map_err(|e| Error::ProofRepository(anyhow!(e)))?;
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .map_err(|e| Error::ProofRepository(anyhow!(e)))?;
        Ok(Self { pool })
    }

    pub fn from_pool(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl persistence::ProofRepository for DBProofs {
    async fn insert(&self, tokens: Vec<cashu::Proof>) -> Result<()> {
        if tokens.is_empty() {
            return Ok(());
        }
        let mut y_strs = Vec::with_capacity(tokens.len());
        let mut blob_values = Vec::with_capacity(tokens.len());
        for token in tokens {
            let y = token.y().map_err(|e| Error::ProofRepository(anyhow!(e)))?;
            let blob = ProofBlob::V1(token);
            let blob_value =
                serde_json::to_value(&blob).map_err(|e| Error::ProofRepository(anyhow!(e)))?;
            y_strs.push(y.to_string());
            blob_values.push(blob_value);
        }
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| Error::ProofRepository(anyhow!(e)))?;
        let result = sqlx::query(
            "INSERT INTO proofs (y, blob) SELECT * FROM UNNEST($1::text[], $2::jsonb[])",
        )
        .bind(&y_strs[..])
        .bind(&blob_values[..])
        .execute(&mut *tx)
        .await;
        if let Err(e) = result {
            if let Some(db_err) = e.as_database_error() {
                let unique_violation = Some(Self::PG_UNIQUE_VIOLATION.into());
                if db_err.code() == unique_violation {
                    return Err(Error::InvalidInput(BRError::Generic(String::from(
                        "proofs are already spent",
                    ))));
                }
            }
            return Err(Error::ProofRepository(anyhow!(e)));
        }
        tx.commit()
            .await
            .map_err(|e| Error::ProofRepository(anyhow!(e)))?;
        Ok(())
    }

    async fn remove(&self, tokens: &[cashu::PublicKey]) -> Result<()> {
        let y_strs: Vec<String> = tokens.iter().map(|y| y.to_string()).collect();
        sqlx::query(
            r#"
            DELETE FROM proofs WHERE y = ANY($1::text[])
            "#,
        )
        .bind(&y_strs[..])
        .execute(&self.pool)
        .await
        .map_err(|e| Error::ProofRepository(anyhow!(e)))?;
        Ok(())
    }

    async fn contains(&self, y: cashu::PublicKey) -> Result<Option<cashu::ProofState>> {
        let result = sqlx::query!(
            r#"
            SELECT blob as "blob: Json<ProofBlob>"
            FROM proofs
            WHERE y = $1
            "#,
            y.to_string()
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| Error::ProofRepository(anyhow!(e)))?;
        let Some(row) = result else {
            return Ok(None);
        };
        match row.blob.0 {
            ProofBlob::V0 { witness, .. } => {
                let state = cashu::ProofState {
                    y,
                    state: cashu::State::Spent,
                    witness,
                };
                Ok(Some(state))
            }
            ProofBlob::V1(proof) => {
                let state = cashu::ProofState {
                    y,
                    state: cashu::State::Spent,
                    witness: proof.witness,
                };
                Ok(Some(state))
            }
        }
    }
}
