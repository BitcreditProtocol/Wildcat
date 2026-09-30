//! Legacy data-import writes, kept separate from application persistence.
use anyhow::{ensure, Context};
use sqlx::PgConnection;

use super::{
    cashu, ebill, onchain, onchain_meltop_to_row, onchain_mintop_to_row, EbillMintOpBlobV1,
    EbillMintOperationBlob, VaultProofBlob,
};

pub const EBILL_IMPORT_ID: &str = "surreal-to-postgres/treasury-ebill/v1";
pub const VAULT_IMPORT_ID: &str = "surreal-to-postgres/treasury-vault/v1";
pub const ONCHAIN_IMPORT_ID: &str = "surreal-to-postgres/treasury-onchain/v1";

pub fn ebill_source_duplicates(ops: &[ebill::MintOperation]) -> Vec<String> {
    let mut duplicates = duplicate_values("ebill uid", ops.iter().map(|op| op.uid.to_string()));
    duplicates.extend(duplicate_values(
        "ebill bill_id",
        ops.iter().map(|op| op.bill_id.to_string()),
    ));
    duplicates
}

pub fn vault_source_duplicates(proofs: &[cashu::Proof]) -> anyhow::Result<Vec<String>> {
    let ys = proofs
        .iter()
        .map(|proof| proof.y().map(|y| y.to_string()))
        .collect::<Result<Vec<_>, _>>()
        .context("Failed to compute source vault proof ys")?;
    Ok(duplicate_values("vault y", ys))
}

fn duplicate_values(field: &str, values: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut seen = std::collections::BTreeSet::new();
    let mut duplicates = std::collections::BTreeSet::new();
    for value in values {
        if !seen.insert(value.clone()) {
            duplicates.insert(value);
        }
    }
    duplicates
        .into_iter()
        .map(|value| format!("duplicate {field}: {value}"))
        .collect()
}

pub async fn import_ebill(
    conn: &mut PgConnection,
    ops: Vec<ebill::MintOperation>,
) -> anyhow::Result<()> {
    for op in ops {
        let blob = EbillMintOperationBlob::V1(EbillMintOpBlobV1 {
            target: op.target,
            pub_key: op.pub_key,
        });
        let written = sqlx::query(
            "INSERT INTO treasury_ebill_mint_ops (uid, kid, minted, bill_id, blob)
             VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(op.uid)
        .bind(op.kid.to_string())
        .bind(i64::try_from(op.minted.to_u64())?)
        .bind(op.bill_id.to_string())
        .bind(serde_json::to_value(blob)?)
        .execute(&mut *conn)
        .await
        .with_context(|| {
            format!(
                "Failed to import ebill mintop {} (bill_id {})",
                op.uid, op.bill_id
            )
        })?;
        ensure!(
            written.rows_affected() == 1,
            "Ebill mintop {} was not stored",
            op.uid
        );
    }
    Ok(())
}

pub async fn import_vault(
    conn: &mut PgConnection,
    proofs: Vec<cashu::Proof>,
) -> anyhow::Result<()> {
    let mut ys = Vec::with_capacity(proofs.len());
    let mut blobs = Vec::with_capacity(proofs.len());
    for proof in proofs {
        ys.push(proof.y()?.to_string());
        blobs.push(serde_json::to_value(VaultProofBlob::V1(proof))?);
    }
    let written = sqlx::query(
        "INSERT INTO treasury_vault_proofs (y, blob)
         SELECT * FROM UNNEST($1::text[], $2::jsonb[])",
    )
    .bind(&ys)
    .bind(&blobs)
    .execute(conn)
    .await
    .context("Failed to import vault proofs")?;
    ensure!(
        written.rows_affected() == ys.len() as u64,
        "Vault proofs were not all stored"
    );
    Ok(())
}

pub async fn import_onchain(
    conn: &mut PgConnection,
    mintops: Vec<onchain::MintOperation>,
    meltops: Vec<onchain::MeltOperation>,
) -> anyhow::Result<()> {
    for op in mintops {
        let (qid, status, expiry, blob) = onchain_mintop_to_row(op)?;
        let written = sqlx::query(
            "INSERT INTO treasury_onchain_mint_ops (qid, expiry, status, blob)
             VALUES ($1, $2, $3, $4)",
        )
        .bind(qid)
        .bind(expiry)
        .bind(status.to_string())
        .bind(blob)
        .execute(&mut *conn)
        .await
        .with_context(|| format!("Failed to import onchain mintop {qid}"))?;
        ensure!(
            written.rows_affected() == 1,
            "Onchain mintop {qid} was not stored"
        );
    }
    for op in meltops {
        let (qid, status, expiry, ys, blob) = onchain_meltop_to_row(op)?;
        // Preserve source status without application-side expiry updates.
        let written = sqlx::query(
            "INSERT INTO treasury_onchain_melt_ops (qid, expiry, status, input_ys, blob)
             VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(qid)
        .bind(expiry)
        .bind(status.to_string())
        .bind(&ys)
        .bind(blob)
        .execute(&mut *conn)
        .await
        .with_context(|| format!("Failed to import onchain meltop {qid}"))?;
        ensure!(
            written.rows_affected() == 1,
            "Onchain meltop {qid} was not stored"
        );
    }
    Ok(())
}
