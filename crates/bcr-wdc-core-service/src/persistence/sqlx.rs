// ----- standard library imports
use std::{
    collections::{BTreeMap, HashMap},
    str::FromStr,
};
// ----- extra library imports
use anyhow::anyhow;
use async_trait::async_trait;
use bcr_common::{
    cashu::{
        self,
        nut01::{MintKeyPair, MintKeys},
    },
    cdk_common::{common::IssuerVersion, mint::MintKeySetInfo},
    client::admin::core::{BRError, RNFError},
};
use bcr_wdc_utils::{keys::KeysetEntry, postgres};
use bitcoin::bip32::DerivationPath;
use sqlx::types::Json;
use sqlx::{PgPool, Postgres, QueryBuilder};
// ----- local imports
use crate::{
    error::{Error, Result},
    persistence,
};

// ----- end imports

// ///////////////////////////////////////////////////////////////////////// Versioned blob for keysets

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "version", content = "data")]
enum KeysetBlob {
    V1(KeysetBlobV1),
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct KeysetBlobV1 {
    valid_from: u64,
    derivation_path: DerivationPath,
    derivation_path_index: Option<u32>,
    amounts: Vec<u64>,
    input_fee_ppk: u64,
    issuer_version: Option<IssuerVersion>,
    keys: HashMap<String, MintKeyPair>, // Use String for the key to make it JSON serializable
}

#[derive(sqlx::FromRow)]
struct KeysetRow {
    kid: String,
    unit: String,
    active: bool,
    final_expiry: Option<i64>,
    blob: Json<KeysetBlob>,
}

fn keyset_to_row(entry: KeysetEntry) -> Result<KeysetRow> {
    let (info, keyset) = entry;
    let final_expiry = info
        .final_expiry
        .map(i64::try_from)
        .transpose()
        .map_err(|e| Error::KeysRepository(anyhow!(e)))?;
    let jsonable_keys = keyset
        .keys
        .iter()
        .map(|(k, v)| (k.to_string(), v.clone()))
        .collect::<HashMap<String, MintKeyPair>>();
    let blob = KeysetBlob::V1(KeysetBlobV1 {
        valid_from: info.valid_from,
        derivation_path: info.derivation_path,
        derivation_path_index: info.derivation_path_index,
        amounts: info.amounts,
        input_fee_ppk: info.input_fee_ppk,
        issuer_version: info.issuer_version,
        keys: jsonable_keys,
    });
    Ok(KeysetRow {
        kid: info.id.to_string(),
        unit: info.unit.to_string(),
        active: info.active,
        final_expiry,
        blob: Json(blob),
    })
}

fn keyset_from_row(row: KeysetRow) -> Result<KeysetEntry> {
    let kid = cashu::Id::from_str(&row.kid).map_err(|e| Error::KeysRepository(anyhow!(e)))?;
    let unit =
        cashu::CurrencyUnit::from_str(&row.unit).map_err(|e| Error::KeysRepository(anyhow!(e)))?;
    let final_expiry = row
        .final_expiry
        .map(u64::try_from)
        .transpose()
        .map_err(|e| Error::KeysRepository(anyhow!(e)))?;
    let KeysetBlob::V1(blob) = row.blob.0;
    let info = MintKeySetInfo {
        id: kid,
        unit: unit.clone(),
        active: row.active,
        valid_from: blob.valid_from,
        derivation_path: blob.derivation_path,
        derivation_path_index: blob.derivation_path_index,
        amounts: blob.amounts,
        input_fee_ppk: blob.input_fee_ppk,
        final_expiry,
        issuer_version: blob.issuer_version,
    };
    let keys = blob
        .keys
        .into_iter()
        .map(|(k, v)| {
            let key = cashu::Amount::from_str(&k).expect("parsable amount");
            Ok((key, v))
        })
        .collect::<Result<BTreeMap<cashu::Amount, MintKeyPair>>>()?;
    let keyset = cashu::MintKeySet {
        id: kid,
        unit,
        keys: MintKeys::new(keys),
        input_fee_ppk: info.input_fee_ppk,
        final_expiry,
    };
    Ok((info, keyset))
}

// ///////////////////////////////////////////////////////////////////////// DBKeys

#[derive(Debug, Clone)]
pub struct DBKeys {
    pool: PgPool,
}

impl DBKeys {
    pub async fn new(cfg: postgres::DBConnConfig) -> Result<Self> {
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(cfg.max_connections)
            .connect(&cfg.connection)
            .await
            .map_err(|e| Error::KeysRepository(anyhow!(e)))?;
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .map_err(|e| Error::KeysRepository(anyhow!(e)))?;
        Ok(Self { pool })
    }

    pub fn from_pool(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl persistence::KeysRepository for DBKeys {
    async fn store(&self, entry: KeysetEntry) -> Result<()> {
        let row = keyset_to_row(entry)?;
        let kid = row.kid.clone();
        let json_blob =
            serde_json::to_value(&row.blob).map_err(|e| Error::KeysRepository(anyhow!(e)))?;
        let result = sqlx::query!(
            r#"
            INSERT INTO keys (kid, unit, active, final_expiry, blob)
            VALUES ($1, $2, $3, $4, $5)
            ON CONFLICT (kid) DO NOTHING
            RETURNING kid
            "#,
            row.kid,
            row.unit,
            row.active,
            row.final_expiry,
            json_blob,
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| Error::KeysRepository(anyhow!(e)))?;
        if result.is_none() {
            return Err(Error::Conflict(format!("keyset already exists: {kid}")));
        }
        Ok(())
    }

    async fn info(&self, kid: cashu::Id) -> Result<Option<MintKeySetInfo>> {
        let row = sqlx::query_as!(
            KeysetRow,
            r#"
            SELECT kid, unit, active, final_expiry, blob as "blob: Json<KeysetBlob>"
            FROM keys
            WHERE kid = $1
            "#,
            kid.to_string()
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| Error::KeysRepository(anyhow!(e)))?;
        row.map(keyset_from_row)
            .transpose()
            .map(|entry| entry.map(|(info, _)| info))
    }

    async fn keyset(&self, kid: cashu::Id) -> Result<Option<cashu::MintKeySet>> {
        let row = sqlx::query_as!(
            KeysetRow,
            r#"
            SELECT kid, unit, active, final_expiry, blob as "blob: Json<KeysetBlob>"
            FROM keys
            WHERE kid = $1
            "#,
            kid.to_string()
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| Error::KeysRepository(anyhow!(e)))?;
        row.map(keyset_from_row)
            .transpose()
            .map(|entry| entry.map(|(_, keyset)| keyset))
    }

    async fn list_info(
        &self,
        unit: Option<cashu::CurrencyUnit>,
        min_expiration_tstamp: Option<u64>,
        max_expiration_tstamp: Option<u64>,
    ) -> Result<Vec<MintKeySetInfo>> {
        let min_expiration_tstamp = min_expiration_tstamp
            .map(i64::try_from)
            .transpose()
            .map_err(|e| Error::KeysRepository(anyhow!(e)))?;
        let max_expiration_tstamp = max_expiration_tstamp
            .map(i64::try_from)
            .transpose()
            .map_err(|e| Error::KeysRepository(anyhow!(e)))?;
        let mut qb: QueryBuilder<'_, Postgres> =
            QueryBuilder::new("SELECT kid, unit, active, final_expiry, blob FROM keys");
        let any_filter =
            unit.is_some() || min_expiration_tstamp.is_some() || max_expiration_tstamp.is_some();
        if any_filter {
            qb.push(" WHERE ");
            let mut separated = qb.separated(" AND ");
            if let Some(unit) = unit {
                separated
                    .push("unit = ")
                    .push_bind_unseparated(unit.to_string());
            }
            if let Some(min_expiration_tstamp) = min_expiration_tstamp {
                separated
                    .push("final_expiry >= ")
                    .push_bind_unseparated(min_expiration_tstamp);
            }
            if let Some(max_expiration_tstamp) = max_expiration_tstamp {
                separated
                    .push("final_expiry <= ")
                    .push_bind_unseparated(max_expiration_tstamp);
            }
        }
        let rows = qb
            .build_query_as::<KeysetRow>()
            .fetch_all(&self.pool)
            .await
            .map_err(|e| Error::KeysRepository(anyhow!(e)))?;
        rows.into_iter()
            .map(keyset_from_row)
            .map(|entry| entry.map(|(info, _)| info))
            .collect()
    }

    async fn list_keyset(&self) -> Result<Vec<cashu::MintKeySet>> {
        let rows = sqlx::query_as!(
            KeysetRow,
            r#"
            SELECT kid, unit, active, final_expiry, blob as "blob: Json<KeysetBlob>"
            FROM keys
            "#,
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| Error::KeysRepository(anyhow!(e)))?;
        rows.into_iter()
            .map(keyset_from_row)
            .map(|entry| entry.map(|(_, keyset)| keyset))
            .collect()
    }

    async fn deactivate(&self, kid: cashu::Id) -> Result<cashu::Id> {
        let result = sqlx::query!(
            r#"
            UPDATE keys
            SET active = false
            WHERE kid = $1
            RETURNING kid
            "#,
            kid.to_string()
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| Error::KeysRepository(anyhow!(e)))?;
        if result.is_some() {
            Ok(kid)
        } else {
            Err(Error::ResourceNotFound(RNFError::KeysetId(kid)))
        }
    }

    async fn infos_for_expiration_date(&self, expire: u64) -> Result<Vec<MintKeySetInfo>> {
        let expire = i64::try_from(expire).map_err(|e| Error::KeysRepository(anyhow!(e)))?;
        let rows = sqlx::query_as!(
            KeysetRow,
            r#"
            SELECT kid, unit, active, final_expiry, blob as "blob: Json<KeysetBlob>"
            FROM keys
            WHERE final_expiry >= $1
            ORDER BY final_expiry ASC
            "#,
            expire
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| Error::KeysRepository(anyhow!(e)))?;
        rows.into_iter()
            .map(keyset_from_row)
            .map(|entry| entry.map(|(info, _)| info))
            .collect()
    }
}

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
        let result = sqlx::query!(
            "INSERT INTO proofs (y, blob) SELECT * FROM UNNEST($1::text[], $2::jsonb[]) ON CONFLICT (y) DO NOTHING",
        &y_strs,
        &blob_values,
        )
        .execute(&mut *tx)
        .await;
        let result = result.map_err(|e| Error::ProofRepository(anyhow!(e)))?;
        if result.rows_affected() != y_strs.len() as u64 {
            tx.rollback()
                .await
                .map_err(|e| Error::ProofRepository(anyhow!(e)))?;
            return Err(Error::InvalidInput(BRError::Generic(String::from(
                "proofs already spent",
            ))));
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
