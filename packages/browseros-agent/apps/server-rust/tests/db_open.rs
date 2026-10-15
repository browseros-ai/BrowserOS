//! `Database::open` against fresh, TS-written (0010 and 0014) and damaged files: the drizzle
//! catch-up must leave the schema and `__drizzle_migrations` exactly as the TS server would, keep
//! the runs TS's own catch-up loses, and never replay or rewrite TS bookkeeping.

mod db_common;

use db_common::{assert_same, count, fixture, open, raw, read_json, rust_dump, stage};
use sea_orm::sqlx;
use serde_json::Value;
use server_rust::db::{DbError, MigrationOutcome};
use std::path::Path;

fn without(dump: &Value, keys: &[&str]) -> Value {
    let mut dump = db_common::without_rust_ledger(dump);
    if let Some(object) = dump.as_object_mut() {
        for key in keys {
            object.remove(*key);
        }
    }
    dump
}

#[test]
fn frozen_migrations_are_byte_copies_of_the_ts_server_set() -> anyhow::Result<()> {
    let ts = db_common::manifest_dir().join("../server/src/lib/db/migrations");
    if !ts.is_dir() {
        return Ok(()); // The TS server is retired; the frozen copies are the baseline now.
    }
    let rust = db_common::manifest_dir().join("migrations/drizzle");
    let journal: Value = read_json(&ts.join("meta/_journal.json"))?;
    let mut files = vec!["meta/_journal.json".to_owned()];
    for entry in journal["entries"].as_array().into_iter().flatten() {
        files.push(format!("{}.sql", entry["tag"].as_str().unwrap_or_default()));
    }
    assert_eq!(files.len(), 16);
    for file in files {
        assert_eq!(
            std::fs::read(rust.join(&file))?,
            std::fs::read(ts.join(&file))?,
            "{file} differs from the TS server's copy"
        );
    }
    Ok(())
}

#[tokio::test]
async fn fresh_file_matches_a_ts_created_schema_and_journal() -> anyhow::Result<()> {
    let (_dir, paths) = stage(None)?;
    let (db, outcome) = open(&paths).await?;
    assert_eq!(outcome, MigrationOutcome::Created);
    let dump = rust_dump(&db, &paths.db_file).await?;
    let ts_fresh = read_json(&fixture("fresh/schema.json"))?;
    assert_same(
        &db_common::without_rust_ledger(&dump)["schema"],
        &ts_fresh["schema"],
        "schema",
    );
    assert_same(
        &dump["drizzleMigrations"],
        &ts_fresh["drizzleMigrations"],
        "drizzle journal",
    );
    assert_eq!(
        ledger(&paths.db_file).await?,
        vec!["m0001_adopt_drizzle_schema"]
    );
    db.close().await?;

    let (db, outcome) = open(&paths).await?;
    assert_eq!(outcome, MigrationOutcome::UpToDate);
    db.close().await?;
    Ok(())
}

#[tokio::test]
async fn ts_0014_file_is_adopted_without_touching_drizzle_rows() -> anyhow::Result<()> {
    let (_dir, paths) = stage(Some("v0014"))?;
    let original = std::fs::read(&paths.db_file)?;
    let (db, outcome) = open(&paths).await?;
    assert_eq!(
        outcome,
        MigrationOutcome::Migrated {
            drizzle_applied: 0,
            rust_applied: 1
        }
    );
    let expected = read_json(&fixture("v0014/expected.json"))?;
    let dump = rust_dump(&db, &paths.db_file).await?;
    assert_same(
        &dump["drizzleMigrations"],
        &expected["drizzleMigrations"],
        "journal",
    );
    db.close().await?;

    let backup = backup_path(&paths.db_file);
    assert_eq!(std::fs::read(&backup)?, original, "backup is the TS file");
    std::fs::write(&backup, b"sentinel")?;
    let (db, outcome) = open(&paths).await?;
    assert_eq!(outcome, MigrationOutcome::UpToDate);
    db.close().await?;
    assert_eq!(std::fs::read(&backup)?, b"sentinel", "backup taken once");
    Ok(())
}

#[tokio::test]
async fn ts_0010_catch_up_keeps_runs_and_matches_the_ts_result() -> anyhow::Result<()> {
    let (_dir, paths) = stage(Some("v0010"))?;
    assert_eq!(count(&paths.db_file, "scheduled_job_runs").await?, 20);
    let (db, outcome) = open(&paths).await?;
    assert_eq!(
        outcome,
        MigrationOutcome::Migrated {
            drizzle_applied: 4,
            rust_applied: 1
        }
    );
    let dump = rust_dump(&db, &paths.db_file).await?;
    let ts = read_json(&fixture("v0010/ts-catchup.json"))?;
    // Identical to what the TS server makes of the file (schema, journal, every row), except
    // that its 0011 cascade-deletes the runs.
    assert_same(
        &without(&dump, &["runs"]),
        &without(&ts, &["runs"]),
        "v0010",
    );
    assert_eq!(ts["runs"].as_array().map(Vec::len), Some(0));
    let runs = dump["runs"].as_array().map(Vec::len);
    assert_eq!(runs, Some(20), "Rust's catch-up keeps every run");

    // Foreign keys are enforced again after the catch-up.
    assert!(db.schedules().delete_job("job-a").await?);
    assert_eq!(db.schedules().list_runs().await?.len(), 3);
    db.close().await?;
    Ok(())
}

#[tokio::test]
async fn foreign_key_violations_fall_back_to_the_ts_catch_up() -> anyhow::Result<()> {
    let (_dir, paths) = stage(Some("v0010"))?;
    let mut connection = raw(&paths.db_file).await?;
    sqlx::query("PRAGMA foreign_keys = OFF")
        .execute(&mut connection)
        .await?;
    sqlx::query(
        "INSERT INTO scheduled_job_runs (id, job_id, status, started_at, created_at, updated_at) \
         VALUES ('orphan', 'ghost-job', 'failed', 1, 1, 1)",
    )
    .execute(&mut connection)
    .await?;
    drop(connection);

    let (db, outcome) = open(&paths).await?;
    assert_eq!(
        outcome,
        MigrationOutcome::Migrated {
            drizzle_applied: 4,
            rust_applied: 1
        }
    );
    let runs: Vec<String> = db
        .schedules()
        .list_runs()
        .await?
        .into_iter()
        .map(|run| run.id)
        .collect();
    assert_eq!(
        runs,
        vec!["orphan"],
        "TS-identical replay: only the orphan survives"
    );
    db.close().await?;
    Ok(())
}

#[tokio::test]
async fn corrupt_file_is_moved_aside_and_recreated() -> anyhow::Result<()> {
    let (dir, paths) = stage(None)?;
    std::fs::create_dir_all(dir.path().join("db"))?;
    let garbage = vec![0x42_u8; 8192];
    std::fs::write(&paths.db_file, &garbage)?;
    let (db, outcome) = open(&paths).await?;
    assert_eq!(outcome, MigrationOutcome::Created);
    db.close().await?;
    let aside: Vec<_> = std::fs::read_dir(dir.path().join("db"))?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| {
                    name.starts_with("browseros.sqlite.corrupt-") && name.ends_with(".bak")
                })
        })
        .collect();
    assert_eq!(aside.len(), 1);
    assert_eq!(std::fs::read(&aside[0])?, garbage);
    Ok(())
}

#[tokio::test]
async fn newer_drizzle_journal_is_reported_and_left_alone() -> anyhow::Result<()> {
    let (_dir, paths) = stage(Some("v0014"))?;
    let mut connection = raw(&paths.db_file).await?;
    sqlx::query(
        "INSERT INTO __drizzle_migrations (hash, created_at) VALUES ('future-hash', 9999999999999)",
    )
    .execute(&mut connection)
    .await?;
    drop(connection);
    let (db, outcome) = open(&paths).await?;
    assert_eq!(
        outcome,
        MigrationOutcome::DatabaseAhead {
            unknown: vec!["future-hash".to_owned()]
        }
    );
    assert_eq!(count(&paths.db_file, "__drizzle_migrations").await?, 16);
    assert_eq!(db.providers().list().await?.len(), 8);
    db.close().await?;
    Ok(())
}

#[tokio::test]
async fn newer_rust_ledger_is_reported() -> anyhow::Result<()> {
    let (_dir, paths) = stage(None)?;
    let (db, _) = open(&paths).await?;
    db.close().await?;
    let mut connection = raw(&paths.db_file).await?;
    sqlx::query("INSERT INTO seaql_migrations (version, applied_at) VALUES ('m9999_future', 0)")
        .execute(&mut connection)
        .await?;
    drop(connection);
    let (db, outcome) = open(&paths).await?;
    assert_eq!(
        outcome,
        MigrationOutcome::DatabaseAhead {
            unknown: vec!["m9999_future".to_owned()]
        }
    );
    db.close().await?;
    Ok(())
}

#[tokio::test]
async fn missing_columns_fail_with_a_diagnostic() -> anyhow::Result<()> {
    let (_dir, paths) = stage(None)?;
    let (db, _) = open(&paths).await?;
    db.close().await?;
    let mut connection = raw(&paths.db_file).await?;
    sqlx::query("ALTER TABLE conversations DROP COLUMN origin")
        .execute(&mut connection)
        .await?;
    drop(connection);
    match open(&paths).await {
        Err(error) => match error.downcast_ref::<DbError>() {
            Some(DbError::Migration(message)) => {
                assert!(message.contains("conversations lacks origin"), "{message}");
            }
            other => panic!("unexpected error {other:?}"),
        },
        Ok(_) => panic!("opened a file missing a column"),
    }
    Ok(())
}

async fn ledger(db_file: &Path) -> anyhow::Result<Vec<String>> {
    let mut connection = raw(db_file).await?;
    Ok(
        sqlx::query_scalar("SELECT version FROM seaql_migrations ORDER BY version")
            .fetch_all(&mut connection)
            .await?,
    )
}

fn backup_path(db_file: &Path) -> std::path::PathBuf {
    let mut name = db_file.as_os_str().to_owned();
    name.push(".pre-rust.bak");
    name.into()
}
