use std::str::FromStr;

use bcr_common::{cashu, core_tests};
use bcr_wdc_treasury_service::{ebill, onchain, persistence::sqlx::data_import};
use bcr_wdc_utils::postgres;
use sqlx::{ConnectOptions, PgPool};
use uuid::Uuid;

fn destination(pool: &PgPool) -> postgres::DBConnConfig {
    postgres::DBConnConfig {
        connection: pool.connect_options().to_url_lossy().to_string(),
        max_connections: 1,
    }
}

fn fixture() -> (
    ebill::MintOperation,
    Vec<cashu::Proof>,
    onchain::MintOperation,
    onchain::MeltOperation,
) {
    let (_, keyset) = core_tests::generate_random_ecash_keyset();
    let proofs = core_tests::generate_random_ecash_proofs(&keyset, &[cashu::Amount::ONE; 2]);
    let op = ebill::MintOperation {
        uid: Uuid::new_v4(),
        kid: keyset.id.into(),
        pub_key: proofs[0].c,
        target: cashu::Amount::ONE,
        minted: cashu::Amount::ZERO,
        bill_id: core_tests::random_bill_id(),
    };
    let expiry = time::OffsetDateTime::now_utc() - time::Duration::hours(1);
    let mint = onchain::MintOperation {
        qid: Uuid::new_v4(),
        kid: keyset.id.into(),
        recipient: bitcoin::Address::from_str("n28b7b8HZcrBqeabbjwGRbo8q9JLcusYFC").unwrap(),
        target: bitcoin::Amount::from_sat(1),
        expiry,
        status: onchain::MintStatus::Pending { blinds: vec![] },
    };
    let melt = onchain::MeltOperation {
        qid: Uuid::new_v4(),
        address: "n28b7b8HZcrBqeabbjwGRbo8q9JLcusYFC".to_owned(),
        target: bitcoin::Amount::from_sat(1),
        available: cashu::Amount::ONE,
        fees: cashu::Amount::ZERO,
        expiry,
        wallet_key: proofs[0].c,
        input_ys: vec![proofs[0].y().unwrap()],
        fp_digest: [0; 32],
        commitment: bitcoin::secp256k1::schnorr::Signature::from_slice(&[1; 64]).unwrap(),
        status: onchain::MeltStatus::Pending,
    };
    (op, proofs, mint, melt)
}

async fn markers(pool: &PgPool) -> Vec<String> {
    sqlx::query_scalar("SELECT id FROM wdc_data_imports ORDER BY id")
        .fetch_all(pool)
        .await
        .unwrap()
}

#[sqlx::test(migrations = false)]
#[ignore = "requires DATABASE_URL with CREATEDB permission"]
async fn treasury_data_import_first_and_second_execution(pool: PgPool) {
    let (op, proofs, mint, melt) = fixture();
    let cfg = destination(&pool);
    postgres::run_data_import(&cfg, data_import::EBILL_IMPORT_ID, false, move |conn| {
        Box::pin(async move { data_import::import_ebill(conn.unwrap(), vec![op]).await })
    })
    .await
    .unwrap();
    postgres::run_data_import(&cfg, data_import::VAULT_IMPORT_ID, false, move |conn| {
        Box::pin(async move { data_import::import_vault(conn.unwrap(), proofs).await })
    })
    .await
    .unwrap();
    postgres::run_data_import(&cfg, data_import::ONCHAIN_IMPORT_ID, false, move |conn| {
        Box::pin(
            async move { data_import::import_onchain(conn.unwrap(), vec![mint], vec![melt]).await },
        )
    })
    .await
    .unwrap();
    for id in [
        data_import::EBILL_IMPORT_ID,
        data_import::VAULT_IMPORT_ID,
        data_import::ONCHAIN_IMPORT_ID,
    ] {
        postgres::run_data_import(&cfg, id, false, |_| {
            Box::pin(async { anyhow::bail!("Source unavailable") })
        })
        .await
        .unwrap();
    }
    assert_eq!(markers(&pool).await.len(), 3);
    let counts: (i64, i64, i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM treasury_ebill_mint_ops),
                (SELECT count(*) FROM treasury_vault_proofs),
                (SELECT count(*) FROM treasury_onchain_mint_ops),
                (SELECT count(*) FROM treasury_onchain_melt_ops)",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(counts, (1, 2, 1, 1));
    let status: String = sqlx::query_scalar("SELECT status FROM treasury_onchain_melt_ops")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(status, "pending");
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "requires DATABASE_URL with CREATEDB permission"]
async fn treasury_data_import_target_completion_is_independent_and_vault_does_not_overwrite(
    pool: PgPool,
) {
    let (op, proofs, mint, melt) = fixture();
    let cfg = destination(&pool);
    let existing = serde_json::json!({"existing": "vault proof"});
    sqlx::query("INSERT INTO treasury_vault_proofs VALUES ($1, $2)")
        .bind(proofs[1].y().unwrap().to_string())
        .bind(&existing)
        .execute(&pool)
        .await
        .unwrap();
    postgres::run_data_import(&cfg, data_import::EBILL_IMPORT_ID, false, move |conn| {
        Box::pin(async move { data_import::import_ebill(conn.unwrap(), vec![op]).await })
    })
    .await
    .unwrap();
    assert!(
        postgres::run_data_import(&cfg, data_import::VAULT_IMPORT_ID, false, move |conn| {
            Box::pin(async move { data_import::import_vault(conn.unwrap(), proofs).await })
        })
        .await
        .is_err()
    );
    assert_eq!(markers(&pool).await, vec![data_import::EBILL_IMPORT_ID]);
    let blobs: Vec<serde_json::Value> =
        sqlx::query_scalar("SELECT blob FROM treasury_vault_proofs")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(blobs, vec![existing]);
    postgres::run_data_import(&cfg, data_import::ONCHAIN_IMPORT_ID, false, move |conn| {
        Box::pin(
            async move { data_import::import_onchain(conn.unwrap(), vec![mint], vec![melt]).await },
        )
    })
    .await
    .unwrap();
    assert_eq!(
        markers(&pool).await,
        vec![data_import::EBILL_IMPORT_ID, data_import::ONCHAIN_IMPORT_ID]
    );
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "requires DATABASE_URL with CREATEDB permission"]
async fn treasury_data_import_ebill_error_rolls_back_earlier_operation(pool: PgPool) {
    let (first, _, _, _) = fixture();
    let (mut second, _, _, _) = fixture();
    second.minted = cashu::Amount::ONE;
    sqlx::query(
        "ALTER TABLE treasury_ebill_mint_ops ADD CONSTRAINT reject_import CHECK (minted = 0)",
    )
    .execute(&pool)
    .await
    .unwrap();
    let result = postgres::run_data_import(
        &destination(&pool),
        data_import::EBILL_IMPORT_ID,
        false,
        move |conn| {
            Box::pin(
                async move { data_import::import_ebill(conn.unwrap(), vec![first, second]).await },
            )
        },
    )
    .await;
    assert!(result.is_err());
    assert!(markers(&pool).await.is_empty());
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM treasury_ebill_mint_ops")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "requires DATABASE_URL with CREATEDB permission"]
async fn treasury_data_import_melt_error_rolls_back_mintops(pool: PgPool) {
    let (_, _, mint, melt) = fixture();
    sqlx::query("ALTER TABLE treasury_onchain_melt_ops ADD CONSTRAINT reject_import CHECK (false)")
        .execute(&pool)
        .await
        .unwrap();
    let result = postgres::run_data_import(
        &destination(&pool),
        data_import::ONCHAIN_IMPORT_ID,
        false,
        move |conn| {
            Box::pin(async move {
                data_import::import_onchain(conn.unwrap(), vec![mint], vec![melt]).await
            })
        },
    )
    .await;
    assert!(result.is_err());
    assert!(markers(&pool).await.is_empty());
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM treasury_onchain_mint_ops")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
}

#[sqlx::test(migrations = false)]
#[ignore = "requires DATABASE_URL with CREATEDB permission"]
async fn treasury_data_import_completion_is_scoped_to_destination_database(ebill: PgPool) {
    // Use three actual databases as configured in production. UUID names ensure
    // these extra fixtures cannot collide with existing databases or other tests.
    let vault_name = format!("wdc_import_vault_{}", Uuid::new_v4().simple());
    let onchain_name = format!("wdc_import_onchain_{}", Uuid::new_v4().simple());
    let mut pools = Vec::new();
    for name in [&vault_name, &onchain_name] {
        sqlx::query(&format!("CREATE DATABASE {name}"))
            .execute(&ebill)
            .await
            .unwrap();
        let options = ebill.connect_options().as_ref().clone().database(name);
        pools.push(PgPool::connect_with(options).await.unwrap());
    }
    let (op, proofs, mint, melt) = fixture();
    let ebill_cfg = destination(&ebill);
    let vault_cfg = destination(&pools[0]);
    let onchain_cfg = destination(&pools[1]);
    postgres::run_data_import(
        &ebill_cfg,
        data_import::EBILL_IMPORT_ID,
        false,
        move |conn| {
            Box::pin(async move { data_import::import_ebill(conn.unwrap(), vec![op]).await })
        },
    )
    .await
    .unwrap();
    postgres::run_data_import(&vault_cfg, data_import::VAULT_IMPORT_ID, false, |_| {
        Box::pin(async { anyhow::bail!("Vault source interrupted") })
    })
    .await
    .unwrap_err();
    postgres::run_data_import(
        &onchain_cfg,
        data_import::ONCHAIN_IMPORT_ID,
        false,
        move |conn| {
            Box::pin(async move {
                data_import::import_onchain(conn.unwrap(), vec![mint], vec![melt]).await
            })
        },
    )
    .await
    .unwrap();
    assert_eq!(markers(&ebill).await, vec![data_import::EBILL_IMPORT_ID]);
    assert!(markers(&pools[0]).await.is_empty());
    assert_eq!(
        markers(&pools[1]).await,
        vec![data_import::ONCHAIN_IMPORT_ID]
    );
    postgres::run_data_import(
        &vault_cfg,
        data_import::VAULT_IMPORT_ID,
        false,
        move |conn| Box::pin(async move { data_import::import_vault(conn.unwrap(), proofs).await }),
    )
    .await
    .unwrap();
    assert_eq!(markers(&pools[0]).await, vec![data_import::VAULT_IMPORT_ID]);
    for (pool, name) in pools.into_iter().zip([vault_name, onchain_name]) {
        pool.close().await;
        sqlx::query(&format!("DROP DATABASE {name}"))
            .execute(&ebill)
            .await
            .unwrap();
    }
}
