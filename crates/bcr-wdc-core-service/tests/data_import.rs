use bcr_common::{cashu, core_tests};
use bcr_wdc_core_service::persistence::{
    sqlx::{data_import, Repository as SqlxRepository},
    surreal::{DumpedCommitment, ProofDBEntry},
    Repository, SignatureOwner,
};
use bcr_wdc_utils::{keys, postgres};
use sqlx::{ConnectOptions, PgPool};

fn destination(pool: &PgPool) -> postgres::DBConnConfig {
    postgres::DBConnConfig {
        connection: pool.connect_options().to_url_lossy().to_string(),
        max_connections: 1,
    }
}

fn fixture() -> (
    keys::MintKeysEntry,
    Vec<(cashu::PublicKey, cashu::BlindSignature)>,
    Vec<ProofDBEntry>,
) {
    let (info, keyset) = core_tests::generate_random_ecash_keyset();
    let proofs = core_tests::generate_random_ecash_proofs(&keyset, &[cashu::Amount::ONE; 3]);
    let y = proofs[0].y().unwrap();
    let signature = core_tests::generate_ecash_signatures(&keyset, &[cashu::Amount::ONE]).remove(0);
    let proofs = proofs
        .into_iter()
        .map(|p| ProofDBEntry {
            id: surrealdb::RecordId::from_table_key("proofs", p.y().unwrap().to_string()),
            kid: p.keyset_id,
            c: p.c,
            secret: p.secret,
            witness: p.witness,
        })
        .collect();
    (keys::to_entry(info, keyset), vec![(y, signature)], proofs)
}

fn commitment(
    inputs: Vec<cashu::PublicKey>,
    outputs: Vec<cashu::PublicKey>,
    signature_byte: u8,
) -> DumpedCommitment {
    DumpedCommitment {
        signature: bitcoin::secp256k1::schnorr::Signature::from_slice(&[signature_byte; 64])
            .unwrap(),
        expiration: time::macros::datetime!(2030-01-01 0:00 UTC),
        wallet_key: inputs[0],
        inputs,
        outputs,
        fp_digest: [signature_byte; 32],
        signed: SignatureOwner::Alpha,
    }
}

fn proof_y(proof: &ProofDBEntry) -> cashu::PublicKey {
    proof.id.key().to_string().parse().unwrap()
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "requires DATABASE_URL with CREATEDB permission"]
async fn core_data_import_preserves_conflicts_and_second_run_skips(pool: PgPool) {
    let (keyset, signatures, proofs) = fixture();
    let repository = SqlxRepository::from_pool(pool.clone());
    let mut existing_keyset = keyset.clone();
    existing_keyset.active = false;
    repository.keys_store(existing_keyset).await.unwrap();
    let mut existing_signature = signatures[0].1.clone();
    existing_signature.amount = cashu::Amount::from(2_u64);
    repository
        .signature_store(signatures[0].0, existing_signature.clone())
        .await
        .unwrap();
    sqlx::query("INSERT INTO core_commitments VALUES ('commitment', now(), '{}')")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO core_proofs (y, signature, deadline) VALUES ($1, 'commitment', now()), ($2, NULL, now())")
        .bind(proofs[0].id.key().to_string()).bind(proofs[1].id.key().to_string())
        .execute(&pool).await.unwrap();
    let cfg = destination(&pool);
    postgres::run_data_import(&cfg, data_import::IMPORT_ID, false, move |conn| {
        Box::pin(async move {
            data_import::import(
                conn.unwrap(),
                vec![keyset],
                signatures,
                vec![],
                vec![],
                proofs,
            )
            .await
        })
    })
    .await
    .unwrap();
    let consumed: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM core_proofs WHERE blob->>'version' = 'V0' AND signature IS NULL AND deadline IS NULL",
    ).fetch_one(&pool).await.unwrap();
    assert_eq!(consumed, 3);
    let active: bool = sqlx::query_scalar("SELECT active FROM core_keys")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(!active);
    let blob: serde_json::Value = sqlx::query_scalar("SELECT blob FROM core_signatures")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        blob["data"],
        serde_json::to_value(existing_signature).unwrap()
    );
    postgres::run_data_import(&cfg, data_import::IMPORT_ID, false, |_| {
        Box::pin(async { anyhow::bail!("Source unavailable") })
    })
    .await
    .unwrap();
    let markers: i64 = sqlx::query_scalar("SELECT count(*) FROM wdc_data_imports")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(markers, 1);
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "requires DATABASE_URL with CREATEDB permission"]
async fn core_data_import_existing_spent_proof_rolls_back_everything(pool: PgPool) {
    let (keyset, signatures, proofs) = fixture();
    let pending = commitment(vec![proof_y(&proofs[0])], vec![proofs[0].c], 1);
    let reservations = vec![(proof_y(&proofs[1]), pending.expiration)];
    let existing = serde_json::json!({"existing": "spent"});
    sqlx::query("INSERT INTO core_proofs (y, blob) VALUES ($1, $2)")
        .bind(proofs[2].id.key().to_string())
        .bind(&existing)
        .execute(&pool)
        .await
        .unwrap();
    let result = postgres::run_data_import(
        &destination(&pool),
        data_import::IMPORT_ID,
        false,
        move |conn| {
            Box::pin(async move {
                data_import::import(
                    conn.unwrap(),
                    vec![keyset],
                    signatures,
                    vec![pending],
                    reservations,
                    proofs,
                )
                .await
            })
        },
    )
    .await;
    assert!(result.is_err());
    let counts: (i64, i64, i64, i64, i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM core_keys), (SELECT count(*) FROM core_signatures),
                (SELECT count(*) FROM core_commitments), (SELECT count(*) FROM core_proofs),
                (SELECT count(*) FROM core_commitment_outputs), (SELECT count(*) FROM wdc_data_imports)",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(counts, (0, 0, 0, 1, 0, 0));
    let stored: serde_json::Value = sqlx::query_scalar("SELECT blob FROM core_proofs")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(stored, existing);
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "requires DATABASE_URL with CREATEDB permission"]
async fn core_data_import_key_and_signature_write_errors_fail(pool: PgPool) {
    for table in ["core_keys", "core_signatures"] {
        sqlx::query(&format!(
            "ALTER TABLE {table} ADD CONSTRAINT reject_import CHECK (false)"
        ))
        .execute(&pool)
        .await
        .unwrap();
        let (keyset, signatures, proofs) = fixture();
        let result = postgres::run_data_import(
            &destination(&pool),
            data_import::IMPORT_ID,
            false,
            move |conn| {
                Box::pin(async move {
                    data_import::import(
                        conn.unwrap(),
                        vec![keyset],
                        signatures,
                        vec![],
                        vec![],
                        proofs,
                    )
                    .await
                })
            },
        )
        .await;
        assert!(result.is_err());
        let counts: (i64, i64, i64) = sqlx::query_as(
            "SELECT (SELECT count(*) FROM core_keys), (SELECT count(*) FROM core_signatures),
                    (SELECT count(*) FROM wdc_data_imports)",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(counts, (0, 0, 0));
        sqlx::query(&format!(
            "ALTER TABLE {table} DROP CONSTRAINT reject_import"
        ))
        .execute(&pool)
        .await
        .unwrap();
    }
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "requires DATABASE_URL with CREATEDB permission"]
async fn core_data_import_imports_pending_states_and_spent_wins(pool: PgPool) {
    let (keyset, signatures, proofs) = fixture();
    let ys: Vec<_> = proofs.iter().map(proof_y).collect();
    let pending = commitment(ys[..2].to_vec(), vec![proofs[0].c], 1);
    let signature = pending.signature;
    let reservations = ys.iter().map(|y| (*y, pending.expiration)).collect();
    let spent = vec![proofs[0].clone()];
    let cfg = destination(&pool);
    postgres::run_data_import(&cfg, data_import::IMPORT_ID, false, move |conn| {
        Box::pin(async move {
            data_import::import(
                conn.unwrap(),
                vec![keyset],
                signatures,
                vec![pending],
                reservations,
                spent,
            )
            .await
        })
    })
    .await
    .unwrap();

    let states: Vec<(Option<String>, bool, bool)> = sqlx::query_as(
        "SELECT signature, deadline IS NOT NULL, blob IS NOT NULL
         FROM core_proofs WHERE y = ANY($1) ORDER BY array_position($1::text[], y)",
    )
    .bind(ys.iter().map(ToString::to_string).collect::<Vec<_>>())
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        states,
        vec![
            (None, false, true),
            (Some(signature.to_string()), false, false),
            (None, true, false)
        ]
    );
    let repository = SqlxRepository::from_pool(pool.clone());
    let stored = repository.commitment_load(&signature).await.unwrap();
    assert_eq!(stored.inputs, vec![ys[1]]);
    assert_eq!(stored.outputs, vec![proofs[0].c]);
    assert_eq!(stored.signed, SignatureOwner::Alpha);
    assert_eq!(stored.fp_digest, [1; 32]);
    postgres::run_data_import(&cfg, data_import::IMPORT_ID, false, |_| {
        Box::pin(async { anyhow::bail!("Source unavailable") })
    })
    .await
    .unwrap();
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "requires DATABASE_URL with CREATEDB permission"]
async fn core_data_import_conflicting_commitments_leave_no_partial_rows(pool: PgPool) {
    let (_, _, proofs) = fixture();
    let ys: Vec<_> = proofs.iter().map(proof_y).collect();
    let first = commitment(vec![ys[0]], vec![proofs[0].c], 1);
    let input_conflict = commitment(vec![ys[1], ys[0]], vec![proofs[1].c], 2);
    let output_conflict = commitment(vec![ys[2]], vec![proofs[2].c, proofs[0].c], 3);
    let signature_conflict = commitment(vec![ys[1]], vec![proofs[1].c], 1);
    postgres::run_data_import(
        &destination(&pool),
        data_import::IMPORT_ID,
        false,
        move |conn| {
            Box::pin(async move {
                data_import::import(
                    conn.unwrap(),
                    vec![],
                    vec![],
                    vec![first, input_conflict, output_conflict, signature_conflict],
                    vec![],
                    vec![],
                )
                .await
            })
        },
    )
    .await
    .unwrap();
    let counts: (i64, i64, i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM core_commitments), (SELECT count(*) FROM core_proofs),
                (SELECT count(*) FROM core_commitment_outputs), (SELECT count(*) FROM wdc_data_imports)",
    ).fetch_one(&pool).await.unwrap();
    assert_eq!(counts, (1, 1, 1, 1));
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "requires DATABASE_URL with CREATEDB permission"]
async fn core_data_import_commitment_and_reservation_write_errors_roll_back(pool: PgPool) {
    for phase in ["commitment", "input", "output", "reservation"] {
        let table = match phase {
            "commitment" => "core_commitments",
            "output" => "core_commitment_outputs",
            _ => "core_proofs",
        };
        // Let the commitment's savepoint finish before rejecting a reservation,
        // proving released savepoints still roll back with the outer import.
        let check = if phase == "reservation" {
            "deadline IS NULL"
        } else {
            "false"
        };
        sqlx::query(&format!(
            "ALTER TABLE {table} ADD CONSTRAINT reject_import CHECK ({check})"
        ))
        .execute(&pool)
        .await
        .unwrap();
        let (keyset, signatures, proofs) = fixture();
        let pending = commitment(vec![proof_y(&proofs[0])], vec![proofs[0].c], 1);
        let reservations = vec![(proof_y(&proofs[1]), pending.expiration)];
        let commitments = vec![pending];
        let result = postgres::run_data_import(
            &destination(&pool),
            data_import::IMPORT_ID,
            false,
            move |conn| {
                Box::pin(async move {
                    data_import::import(
                        conn.unwrap(),
                        vec![keyset],
                        signatures,
                        commitments,
                        reservations,
                        vec![],
                    )
                    .await
                })
            },
        )
        .await;
        assert!(result.is_err(), "{phase}");
        let counts: (i64, i64, i64, i64, i64, i64) = sqlx::query_as(
            "SELECT (SELECT count(*) FROM core_keys), (SELECT count(*) FROM core_signatures),
                    (SELECT count(*) FROM core_commitments), (SELECT count(*) FROM core_proofs),
                    (SELECT count(*) FROM core_commitment_outputs), (SELECT count(*) FROM wdc_data_imports)",
        ).fetch_one(&pool).await.unwrap();
        assert_eq!(counts, (0, 0, 0, 0, 0, 0), "{phase}");
        sqlx::query(&format!(
            "ALTER TABLE {table} DROP CONSTRAINT reject_import"
        ))
        .execute(&pool)
        .await
        .unwrap();
    }
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "requires DATABASE_URL with CREATEDB permission"]
async fn core_data_import_suppressed_writes_fail_validation(pool: PgPool) {
    sqlx::raw_sql("CREATE FUNCTION suppress_import() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RETURN NULL; END; $$")
        .execute(&pool).await.unwrap();
    for table in ["core_commitments", "core_proofs", "core_commitment_outputs"] {
        sqlx::query(&format!("CREATE TRIGGER suppress_import BEFORE INSERT ON {table} FOR EACH ROW EXECUTE FUNCTION suppress_import()"))
            .execute(&pool).await.unwrap();
        let (keyset, signatures, proofs) = fixture();
        let pending = commitment(vec![proof_y(&proofs[0])], vec![proofs[0].c], 1);
        let result = postgres::run_data_import(
            &destination(&pool),
            data_import::IMPORT_ID,
            false,
            move |conn| {
                Box::pin(async move {
                    data_import::import(
                        conn.unwrap(),
                        vec![keyset],
                        signatures,
                        vec![pending],
                        vec![],
                        vec![],
                    )
                    .await
                })
            },
        )
        .await;
        assert!(result.is_err(), "{table}");
        let counts: (i64, i64, i64, i64, i64, i64) = sqlx::query_as(
            "SELECT (SELECT count(*) FROM core_keys), (SELECT count(*) FROM core_signatures),
                    (SELECT count(*) FROM core_commitments), (SELECT count(*) FROM core_proofs),
                    (SELECT count(*) FROM core_commitment_outputs), (SELECT count(*) FROM wdc_data_imports)",
        ).fetch_one(&pool).await.unwrap();
        assert_eq!(counts, (0, 0, 0, 0, 0, 0), "{table}");
        sqlx::query(&format!("DROP TRIGGER suppress_import ON {table}"))
            .execute(&pool)
            .await
            .unwrap();
    }
}
