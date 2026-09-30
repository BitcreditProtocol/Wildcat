//! Legacy data-import writes, kept separate from application persistence.
use std::str::FromStr;

use anyhow::{ensure, Context};
use sqlx::{Connection, PgConnection};

use super::{
    cashu, commitment_to_row, keys_utils, keyset_to_row, persistence, ProofBlob, SignatureBlob,
    TStamp,
};

pub const IMPORT_ID: &str = "surreal-to-postgres/core/v1";

pub async fn import(
    conn: &mut PgConnection,
    keys: Vec<keys_utils::MintKeysEntry>,
    signatures: Vec<(cashu::PublicKey, cashu::BlindSignature)>,
    commitments: Vec<persistence::surreal::DumpedCommitment>,
    reserved_ys: Vec<(cashu::PublicKey, TStamp)>,
    proofs: Vec<persistence::surreal::ProofDBEntry>,
) -> anyhow::Result<()> {
    for keyset in keys {
        let row = keyset_to_row(keyset)?;
        let written = sqlx::query(
            "INSERT INTO core_keys (kid, unit, active, final_expiry, blob)
             VALUES ($1, $2, $3, $4, $5) ON CONFLICT (kid) DO NOTHING",
        )
        .bind(&row.kid)
        .bind(row.unit)
        .bind(row.active)
        .bind(row.final_expiry)
        .bind(serde_json::to_value(row.blob)?)
        .execute(&mut *conn)
        .await
        .with_context(|| format!("Failed to import keyset {}", row.kid))?;
        if written.rows_affected() == 0 {
            let exists: bool =
                sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM core_keys WHERE kid = $1)")
                    .bind(&row.kid)
                    .fetch_one(&mut *conn)
                    .await?;
            ensure!(exists, "Keyset {} was not stored", row.kid);
            println!("Keeping existing keyset {}", row.kid);
        }
    }
    for (y, signature) in signatures {
        let written = sqlx::query(
            "INSERT INTO core_signatures (y, blob)
             VALUES ($1, $2) ON CONFLICT (y) DO NOTHING",
        )
        .bind(y.to_string())
        .bind(serde_json::to_value(SignatureBlob::V1(signature))?)
        .execute(&mut *conn)
        .await
        .with_context(|| format!("Failed to import signature {y}"))?;
        if written.rows_affected() == 0 {
            let exists: bool =
                sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM core_signatures WHERE y = $1)")
                    .bind(y.to_string())
                    .fetch_one(&mut *conn)
                    .await?;
            ensure!(exists, "Signature {y} was not stored");
            println!("Keeping existing signature {y}");
        }
    }
    // Keep the upstream order: commitments win over reservations, then spent
    // proofs replace either placeholder. A conflicting commitment is skipped as
    // a whole, including its outputs, using a savepoint in the import transaction.
    for commitment in commitments {
        let row = commitment_to_row(
            commitment.expiration,
            commitment.wallet_key,
            commitment.signature,
            commitment.fp_digest,
            commitment.signed,
        );
        let mut tx = conn.begin().await?;
        let written = sqlx::query(
            "INSERT INTO core_commitments (signature, expiration, blob)
             VALUES ($1, $2, $3) ON CONFLICT (signature) DO NOTHING",
        )
        .bind(&row.signature)
        .bind(row.expiration)
        .bind(serde_json::to_value(row.blob)?)
        .execute(&mut *tx)
        .await
        .with_context(|| format!("Failed to import commitment {}", row.signature))?;
        let imported = if written.rows_affected() == 0 {
            let exists: bool = sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM core_commitments WHERE signature = $1)",
            )
            .bind(&row.signature)
            .fetch_one(&mut *tx)
            .await?;
            ensure!(exists, "Commitment {} was not stored", row.signature);
            false
        } else {
            ensure!(
                written.rows_affected() == 1,
                "Unexpected commitment write count"
            );
            import_commitment_ys(&mut tx, "core_proofs", commitment.inputs, &row.signature).await?
                && import_commitment_ys(
                    &mut tx,
                    "core_commitment_outputs",
                    commitment.outputs,
                    &row.signature,
                )
                .await?
        };
        if imported {
            tx.commit().await?;
        } else {
            tx.rollback().await?;
            println!(
                "Skipping commitment {}: existing signature, input or output",
                row.signature
            );
        }
    }
    for (y, deadline) in reserved_ys {
        let written = sqlx::query(
            "INSERT INTO core_proofs (y, deadline) VALUES ($1, $2)
             ON CONFLICT (y) DO NOTHING",
        )
        .bind(y.to_string())
        .bind(deadline)
        .execute(&mut *conn)
        .await
        .with_context(|| format!("Failed to import reserved y {y}"))?;
        if written.rows_affected() == 0 {
            let exists: bool =
                sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM core_proofs WHERE y = $1)")
                    .bind(y.to_string())
                    .fetch_one(&mut *conn)
                    .await?;
            ensure!(exists, "Reserved y {y} was not stored");
            println!("Keeping existing proof state for reserved y {y}");
        }
    }
    let mut ys = Vec::with_capacity(proofs.len());
    let mut blobs = Vec::with_capacity(proofs.len());
    for proof in proofs {
        let y = cashu::PublicKey::from_str(&proof.id.key().to_string())?;
        ys.push(y.to_string());
        blobs.push(serde_json::to_value(ProofBlob::V0 {
            kid: proof.kid,
            witness: proof.witness,
            c: proof.c,
            secret: proof.secret,
        })?);
    }
    // Spent proofs take precedence over committed/reserved placeholders, but an
    // existing spent proof must never be overwritten (the original insert_v0 rule).
    let written = sqlx::query(
        "INSERT INTO core_proofs (y, blob)
         SELECT * FROM UNNEST($1::text[], $2::jsonb[])
         ON CONFLICT (y) DO UPDATE
         SET signature = NULL, deadline = NULL, blob = EXCLUDED.blob
         WHERE core_proofs.blob IS NULL",
    )
    .bind(&ys)
    .bind(&blobs)
    .execute(conn)
    .await
    .context("Failed to import proofs")?;
    ensure!(
        written.rows_affected() == ys.len() as u64,
        "Proofs are already spent; import rolled back"
    );
    Ok(())
}

async fn import_commitment_ys(
    conn: &mut PgConnection,
    table: &'static str,
    keys: Vec<cashu::PublicKey>,
    signature: &str,
) -> anyhow::Result<bool> {
    let ys: Vec<String> = keys.iter().map(ToString::to_string).collect();
    let unique: std::collections::HashSet<_> = ys.iter().collect();
    ensure!(
        unique.len() == ys.len(),
        "Commitment {signature} repeats a y within {table}; reconcile the source commitment",
    );
    let signatures = vec![signature; ys.len()];
    let written = sqlx::query(&format!(
        "INSERT INTO {table} (y, signature)
         SELECT * FROM UNNEST($1::text[], $2::text[]) ON CONFLICT (y) DO NOTHING",
    ))
    .bind(&ys)
    .bind(&signatures)
    .execute(&mut *conn)
    .await
    .with_context(|| format!("Failed to import commitment {signature} into {table}"))?;
    if written.rows_affected() == ys.len() as u64 {
        return Ok(true);
    }
    // Only actual primary-key conflicts may skip a commitment. A suppressed
    // write with no corresponding row is a validation failure, not a conflict.
    let stored: i64 =
        sqlx::query_scalar(&format!("SELECT count(*) FROM {table} WHERE y = ANY($1)"))
            .bind(&ys)
            .fetch_one(conn)
            .await?;
    ensure!(
        stored == unique.len() as i64,
        "Commitment {signature} has missing rows in {table}"
    );
    Ok(false)
}
