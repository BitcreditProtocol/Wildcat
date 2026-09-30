use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

use bcr_wdc_utils::postgres::{run_data_import, DBConnConfig};
use sqlx::{ConnectOptions, PgPool};
use tokio::sync::Notify;

fn destination(pool: &PgPool) -> DBConnConfig {
    DBConnConfig {
        connection: pool.connect_options().to_url_lossy().to_string(),
        max_connections: 1,
    }
}

async fn counts(pool: &PgPool) -> (i64, i64) {
    let data = sqlx::query_scalar("SELECT count(*) FROM core_signatures")
        .fetch_one(pool)
        .await
        .unwrap();
    let markers = sqlx::query_scalar("SELECT count(*) FROM wdc_data_imports")
        .fetch_one(pool)
        .await
        .unwrap();
    (data, markers)
}

#[sqlx::test(migrations = false)]
#[ignore = "requires DATABASE_URL with CREATEDB permission"]
async fn first_import_and_completed_import_without_source(pool: PgPool) {
    let cfg = destination(&pool);
    run_data_import(&cfg, "test/v1", false, |conn| {
        Box::pin(async move {
            sqlx::query("INSERT INTO core_signatures VALUES ('one', '{}')")
                .execute(conn.unwrap())
                .await?;
            Ok(())
        })
    })
    .await
    .unwrap();
    // This callback represents a missing source and must never be invoked.
    run_data_import(&cfg, "test/v1", false, |_| {
        Box::pin(async { anyhow::bail!("Source unavailable") })
    })
    .await
    .unwrap();
    assert_eq!(counts(&pool).await, (1, 1));
}

#[sqlx::test(migrations = false)]
#[ignore = "requires DATABASE_URL with CREATEDB permission"]
async fn completed_import_applies_pending_schema_without_source(pool: PgPool) {
    // An import marker must not bypass pending embedded SQLx migrations.
    sqlx::raw_sql(
        "CREATE TABLE wdc_data_imports (id TEXT PRIMARY KEY, completed_at TIMESTAMPTZ NOT NULL DEFAULT now());
         INSERT INTO wdc_data_imports (id) VALUES ('test/v1');
         CREATE TABLE retained_data (value TEXT);
         INSERT INTO retained_data VALUES ('keep me');",
    ).execute(&pool).await.unwrap();
    run_data_import(&destination(&pool), "test/v1", false, |_| {
        Box::pin(async { anyhow::bail!("Source unavailable") })
    })
    .await
    .unwrap();
    let applied: i64 = sqlx::query_scalar("SELECT count(*) FROM _sqlx_migrations WHERE success")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(applied > 0);
    assert_eq!(counts(&pool).await, (0, 1));
    let retained: String = sqlx::query_scalar("SELECT value FROM retained_data")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(retained, "keep me");
}

#[sqlx::test(migrations = false)]
#[ignore = "requires DATABASE_URL with CREATEDB permission"]
async fn completed_import_still_fails_on_schema_errors(pool: PgPool) {
    sqlx::raw_sql(
        "CREATE TABLE wdc_data_imports (id TEXT PRIMARY KEY, completed_at TIMESTAMPTZ NOT NULL DEFAULT now());
         INSERT INTO wdc_data_imports (id) VALUES ('test/v1');
         CREATE TABLE core_commitments (incompatible_column TEXT);",
    ).execute(&pool).await.unwrap();
    let error = run_data_import(&destination(&pool), "test/v1", false, |_| {
        Box::pin(async { panic!("Schema failure must happen before source access") })
    })
    .await
    .unwrap_err();
    let message = format!("{error:#}");
    assert!(message.contains("Data import test/v1"));
    assert!(message.contains("Failed to apply destination schema migrations"));
    let markers: i64 = sqlx::query_scalar("SELECT count(*) FROM wdc_data_imports")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(markers, 1);
}

#[sqlx::test(migrations = false)]
#[ignore = "requires DATABASE_URL with CREATEDB permission"]
async fn connection_errors_identify_import_target(pool: PgPool) {
    let mut cfg = destination(&pool);
    cfg.connection = "invalid destination URL".to_owned();
    let error = run_data_import(&cfg, "test/connection", false, |_| {
        Box::pin(async { panic!("Connection failure must happen before source access") })
    })
    .await
    .unwrap_err();
    let message = format!("{error:#}");
    assert!(message.contains("Data import test/connection"));
    assert!(message.contains("Failed to connect to destination PostgreSQL"));
}

#[sqlx::test(migrations = false)]
#[ignore = "requires DATABASE_URL with CREATEDB permission"]
async fn rollback_failure_preserves_original_import_error(pool: PgPool) {
    let error = run_data_import(&destination(&pool), "test/rollback", false, |conn| {
        Box::pin(async move {
            let conn = conn.unwrap();
            sqlx::query("INSERT INTO core_signatures VALUES ('one', '{}')")
                .execute(&mut *conn)
                .await?;
            sqlx::query("SELECT pg_terminate_backend(pg_backend_pid())")
                .execute(conn)
                .await
                .expect_err("Terminating this backend must close the connection");
            anyhow::bail!("original import error")
        })
    })
    .await
    .unwrap_err();
    let message = format!("{error:#}");
    assert!(message.contains("Data import test/rollback"));
    assert!(message.contains("Rollback also failed"));
    assert!(message.contains("original import error"));
    assert_eq!(counts(&pool).await, (0, 0));
}

#[sqlx::test(migrations = false)]
#[ignore = "requires DATABASE_URL with CREATEDB permission"]
async fn concurrent_attempts_only_import_once(pool: PgPool) {
    let cfg = destination(&pool);
    let started = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let calls = Arc::new(AtomicUsize::new(0));
    let first_cfg = cfg.clone();
    let first_started = started.clone();
    let first_release = release.clone();
    let first_calls = calls.clone();
    let first = tokio::spawn(async move {
        run_data_import(&first_cfg, "test/v1", false, move |conn| {
            Box::pin(async move {
                first_calls.fetch_add(1, Ordering::SeqCst);
                sqlx::query("INSERT INTO core_signatures VALUES ('one', '{}')")
                    .execute(conn.unwrap())
                    .await?;
                first_started.notify_one();
                first_release.notified().await;
                Ok(())
            })
        })
        .await
    });
    tokio::time::timeout(std::time::Duration::from_secs(5), started.notified())
        .await
        .unwrap();
    let second_calls = calls.clone();
    let second = tokio::spawn(async move {
        run_data_import(&cfg, "test/v1", false, move |_| {
            Box::pin(async move {
                second_calls.fetch_add(1, Ordering::SeqCst);
                anyhow::bail!("Second importer should have skipped")
            })
        })
        .await
    });
    // Observe a real waiting lock rather than relying on task scheduling.
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let waiting: bool = sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM pg_locks
                 WHERE locktype = 'advisory' AND NOT granted
                   AND database = (SELECT oid FROM pg_database WHERE datname = current_database()))",
            )
            .fetch_one(&pool)
            .await
            .unwrap();
            if waiting {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    release.notify_one();
    first.await.unwrap().unwrap();
    second.await.unwrap().unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(counts(&pool).await, (1, 1));
}

#[sqlx::test(migrations = false)]
#[ignore = "requires DATABASE_URL with CREATEDB permission"]
async fn unexpected_write_error_rolls_back_data_and_marker(pool: PgPool) {
    let cfg = destination(&pool);
    let failed = run_data_import(&cfg, "test/v1", false, |conn| {
        Box::pin(async move {
            let conn = conn.unwrap();
            sqlx::query("INSERT INTO core_signatures VALUES ('one', '{}')")
                .execute(&mut *conn)
                .await?;
            sqlx::query("INSERT INTO core_signatures VALUES ('bad', NULL)")
                .execute(conn)
                .await?;
            Ok(())
        })
    })
    .await;
    assert!(failed.is_err());
    assert_eq!(counts(&pool).await, (0, 0));
    run_data_import(&cfg, "test/v1", false, |_| Box::pin(async { Ok(()) }))
        .await
        .unwrap();
    assert_eq!(counts(&pool).await, (0, 1));
}

#[sqlx::test(migrations = false)]
#[ignore = "requires DATABASE_URL with CREATEDB permission"]
async fn validation_failure_rolls_back_data_and_marker(pool: PgPool) {
    let failed = run_data_import(&destination(&pool), "test/v1", false, |conn| {
        Box::pin(async move {
            sqlx::query("INSERT INTO core_signatures VALUES ('one', '{}')")
                .execute(conn.unwrap())
                .await?;
            anyhow::bail!("Required validation failed")
        })
    })
    .await;
    assert!(failed.is_err());
    assert_eq!(counts(&pool).await, (0, 0));
}

#[sqlx::test(migrations = false)]
#[ignore = "requires DATABASE_URL with CREATEDB permission"]
async fn interrupted_import_releases_lock_and_can_retry(pool: PgPool) {
    let cfg = destination(&pool);
    let task_cfg = cfg.clone();
    let started = Arc::new(Notify::new());
    let task_started = started.clone();
    let task = tokio::spawn(async move {
        run_data_import(&task_cfg, "test/v1", false, move |conn| {
            Box::pin(async move {
                sqlx::query("INSERT INTO core_signatures VALUES ('one', '{}')")
                    .execute(conn.unwrap())
                    .await?;
                task_started.notify_one();
                std::future::pending().await
            })
        })
        .await
    });
    tokio::time::timeout(std::time::Duration::from_secs(5), started.notified())
        .await
        .unwrap();
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        run_data_import(&cfg, "test/v1", false, |conn| {
            Box::pin(async move {
                sqlx::query("INSERT INTO core_signatures VALUES ('one', '{}')")
                    .execute(conn.unwrap())
                    .await?;
                Ok(())
            })
        }),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(counts(&pool).await, (1, 1));
}

#[sqlx::test(migrations = false)]
#[ignore = "requires DATABASE_URL with CREATEDB permission"]
async fn dry_run_does_not_connect_or_create_schema_or_state(pool: PgPool) {
    let mut cfg = destination(&pool);
    cfg.connection = "invalid destination URL".to_owned();
    run_data_import(&cfg, "test/v1", true, |conn| {
        Box::pin(async move {
            assert!(conn.is_none());
            println!("DRY RUN: source data reported");
            Ok(())
        })
    })
    .await
    .unwrap();
    let tables: i64 =
        sqlx::query_scalar("SELECT count(*) FROM pg_tables WHERE schemaname = 'public'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(tables, 0);
}

#[sqlx::test(migrations = false)]
#[ignore = "requires DATABASE_URL with CREATEDB permission"]
async fn completion_marker_write_error_rolls_back_imported_data(pool: PgPool) {
    sqlx::query(
        "CREATE TABLE wdc_data_imports (
            id TEXT PRIMARY KEY CHECK (id <> 'test/v1'),
            completed_at TIMESTAMPTZ NOT NULL DEFAULT now()
         )",
    )
    .execute(&pool)
    .await
    .unwrap();
    let result = run_data_import(&destination(&pool), "test/v1", false, |conn| {
        Box::pin(async move {
            sqlx::query("INSERT INTO core_signatures VALUES ('one', '{}')")
                .execute(conn.unwrap())
                .await?;
            Ok(())
        })
    })
    .await;
    assert!(result.is_err());
    assert_eq!(counts(&pool).await, (0, 0));
}
