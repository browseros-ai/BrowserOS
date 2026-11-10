//! Real-file schema parity, atomic historical adoption, downgrade refusal and public read parity.

use std::collections::BTreeMap;
use std::error::Error;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use browseros_core::TabId;
use neo_trace::{
    Delivery, Effect, EpochMillis, OperationEnded, OperationId, OperationKind, OperationStarted,
    Outcome, PrivateReason, RedactionLimits, Redactor, RequestId, RetentionPolicy, RunEffects,
    RunQuery, RunStatus, SecretSet, SecretSetConfig, ShotRead, ShotState, TailQuery,
    TargetSensitivity, TaskId, TraceError, TraceStore, WriterConfig, spawn_writer,
};
use sea_orm::sqlx::sqlite::SqliteConnectOptions;
use sea_orm::sqlx::{self, Connection, Row, SqliteConnection};
use serde_json::{Value, json};
use tempfile::TempDir;

type TestResult<T = ()> = Result<T, Box<dyn Error>>;
const A: &str = "00000000-0000-7000-8000-000000000001";
const B: &str = "00000000-0000-7000-8000-000000000002";
const C: &str = "00000000-0000-7000-8000-000000000003";
const TABLES: [&str; 4] = ["runs", "operations", "run_tabs", "private_tabs"];

async fn connection(path: &Path) -> TestResult<SqliteConnection> {
    Ok(SqliteConnection::connect_with(
        &SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true),
    )
    .await?)
}

async fn fixture(shape: &str, seeded: bool) -> TestResult<(TempDir, PathBuf)> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("trace.db");
    let schema = match shape {
        "s2" => include_str!("fixtures/trace_schema_s2.sql"),
        "s3" => include_str!("fixtures/trace_schema_s3.sql"),
        "s4" => include_str!("fixtures/trace_schema_s4.sql"),
        "s5" => include_str!("fixtures/trace_schema_s5.sql"),
        _ => return Err("unknown test schema".into()),
    };
    let mut conn = connection(&path).await?;
    sqlx::raw_sql("PRAGMA journal_mode = WAL")
        .execute(&mut conn)
        .await?;
    sqlx::raw_sql(schema).execute(&mut conn).await?;
    sqlx::raw_sql("PRAGMA user_version = 1")
        .execute(&mut conn)
        .await?;
    if seeded {
        for (id, task, started, ended, status, request) in [
            (A, Some("task"), 10, Some(30), "ok", "repeat"),
            (B, None, 20, None, "running", "global"),
            (C, Some("task"), 5, Some(9), "ok", "repeat"),
        ] {
            sqlx::query("INSERT INTO runs (id, task_id, started_at, ended_at, status, effects, request_id, host_data, code, result, replay, replay_until, operation_count) VALUES (?, ?, ?, ?, ?, 'done', ?, '{\"owner\":1}', 'return 1', '1', '{\"ok\":true}', 10000, 3)")
                .bind(id).bind(task).bind(started).bind(ended).bind(status).bind(request).execute(&mut conn).await?;
        }
        for (run, seq, target, started, ended, effect) in [
            (A, 1, 9, 11, Some(12), "read"),
            (A, 2, 3, 12, Some(13), "write"),
            (A, 3, 3, 13, Some(14), "read"),
            (B, 1, 4, 21, None, "write"),
            (B, 2, 4, 22, None, "read"),
        ] {
            sqlx::query("INSERT INTO operations (run_id, seq, name, kind, effect, target_tab_id, started_at, ended_at, outcome, delivery) VALUES (?, ?, 'page.title', 'builtin', ?, ?, ?, ?, ?, ?)")
                .bind(run).bind(seq).bind(effect).bind(target).bind(started).bind(ended)
                .bind(ended.map(|_| "ok")).bind(ended.map(|_| "done")).execute(&mut conn).await?;
        }
        sqlx::query("INSERT INTO run_tabs (tab_id, run_id) VALUES (9, ?), (3, ?), (4, ?)")
            .bind(A)
            .bind(A)
            .bind(B)
            .execute(&mut conn)
            .await?;
        sqlx::query(
            "INSERT INTO private_tabs (tab_id, since, reason) VALUES (9, 8, 'later_reason')",
        )
        .execute(&mut conn)
        .await?;
        if shape != "s3" {
            sqlx::raw_sql("UPDATE runs SET complete = 1 WHERE ended_at IS NOT NULL")
                .execute(&mut conn)
                .await?;
            sqlx::query("UPDATE operations SET screenshot = 'stored', screenshot_bytes = 10 WHERE run_id = ? AND seq = 1")
                .bind(A).execute(&mut conn).await?;
            sqlx::query(
                "UPDATE operations SET screenshot = 'withheld' WHERE run_id = ? AND seq = 2",
            )
            .bind(A)
            .execute(&mut conn)
            .await?;
        }
        if shape == "s5" {
            sqlx::query(
                "UPDATE operations SET notes = '[\"looked\"]' WHERE run_id = ? AND seq = 1",
            )
            .bind(A)
            .execute(&mut conn)
            .await?;
        }
    }
    conn.close().await?;
    Ok((directory, path))
}

/// Logical content, rather than bytes: WAL switching/checkpointing can alter file bytes even
/// on a refused open. Quoted SQL values preserve NULL, types and complete rows in any shape.
#[derive(Debug, PartialEq, Eq)]
struct Dump {
    schema: Vec<(String, String, String, Option<String>)>,
    rows: BTreeMap<String, Vec<Vec<String>>>,
    version: i64,
}

async fn logical_dump(path: &Path) -> TestResult<Dump> {
    let mut conn = connection(path).await?;
    let schema: Vec<(String, String, String, Option<String>)> =
        sqlx::query_as("SELECT type, name, tbl_name, sql FROM sqlite_master ORDER BY name")
            .fetch_all(&mut conn)
            .await?;
    let mut rows = BTreeMap::new();
    for (kind, name, _, _) in &schema {
        if kind != "table" {
            continue;
        }
        let columns: Vec<String> =
            sqlx::query_scalar("SELECT name FROM pragma_table_info(?) ORDER BY cid")
                .bind(name)
                .fetch_all(&mut conn)
                .await?;
        let projection = columns
            .iter()
            .map(|name| format!("quote(\"{}\")", name.replace('"', "\"\"")))
            .collect::<Vec<_>>()
            .join(", ");
        let data = sqlx::query(&format!(
            "SELECT {projection} FROM \"{}\"",
            name.replace('"', "\"\"")
        ))
        .fetch_all(&mut conn)
        .await?;
        let mut data = data
            .iter()
            .map(|row| {
                (0..columns.len())
                    .map(|index| row.try_get(index))
                    .collect::<Result<Vec<String>, _>>()
            })
            .collect::<Result<Vec<_>, _>>()?;
        data.sort();
        rows.insert(name.clone(), data);
    }
    let version = sqlx::query_scalar("PRAGMA user_version")
        .fetch_one(&mut conn)
        .await?;
    conn.close().await?;
    Ok(Dump {
        schema,
        rows,
        version,
    })
}

fn without_migration(mut dump: Dump) -> Dump {
    dump.schema
        .retain(|(_, _, table, _)| table != "seaql_migrations");
    dump.rows.remove("seaql_migrations");
    dump
}

/// Replays the old build's migrate/recover sequence, frozen at fc93354bc; it ignores the new
/// migration ledger and proves rollback to that binary can still use an adopted S5 file.
async fn old_build_open(path: &Path) -> TestResult {
    let mut conn = connection(path).await?;
    sqlx::raw_sql("PRAGMA journal_mode = WAL")
        .execute(&mut conn)
        .await?;
    let version: i64 = sqlx::query_scalar("PRAGMA user_version")
        .fetch_one(&mut conn)
        .await?;
    assert!(version <= 1, "the old binary refuses a newer fence");
    let mut tx = conn.begin().await?;
    sqlx::raw_sql(include_str!("fixtures/trace_schema_s5.sql"))
        .execute(&mut *tx)
        .await?;
    sqlx::raw_sql("PRAGMA user_version = 1")
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    let mut tx = conn.begin().await?;
    sqlx::query("UPDATE operations SET outcome = 'cancelled', delivery = CASE WHEN effect = 'write' THEN 'unknown' ELSE 'not_sent' END WHERE outcome IS NULL").execute(&mut *tx).await?;
    sqlx::query("UPDATE runs SET status = 'interrupted', effects = 'unknown', code_withheld = COALESCE(code_withheld, 'unavailable') WHERE ended_at IS NULL").execute(&mut *tx).await?;
    sqlx::query("UPDATE runs SET complete = 1 WHERE complete = 0")
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    conn.close().await?;
    Ok(())
}

async fn shape(path: &Path) -> TestResult<Value> {
    let mut conn = connection(path).await?;
    let mut tables = BTreeMap::new();
    for table in TABLES {
        let columns = sqlx::query(
            "SELECT name, type, \"notnull\", dflt_value, pk FROM pragma_table_info(?) ORDER BY name",
        )
        .bind(table)
        .fetch_all(&mut conn)
        .await?;
        let columns = columns
            .iter()
            .map(|row| {
                Ok(json!([
                    row.try_get::<String, _>("name")?,
                    row.try_get::<String, _>("type")?.to_uppercase(),
                    row.try_get::<i64, _>("notnull")?,
                    row.try_get::<Option<String>, _>("dflt_value")?,
                    row.try_get::<i64, _>("pk")?
                ]))
            })
            .collect::<Result<Vec<_>, sqlx::Error>>()?;
        let wr: i64 = sqlx::query_scalar("SELECT wr FROM pragma_table_list WHERE name = ?")
            .bind(table)
            .fetch_one(&mut conn)
            .await?;
        let fks = sqlx::query(&format!("PRAGMA foreign_key_list({table})"))
            .fetch_all(&mut conn)
            .await?;
        assert!(
            fks.is_empty(),
            "orphan-tolerant trace writes must stay unconstrained"
        );
        let mut indices = BTreeMap::new();
        for index in
            sqlx::query("SELECT name, \"unique\", origin, partial FROM pragma_index_list(?)")
                .bind(table)
                .fetch_all(&mut conn)
                .await?
        {
            let name: String = index.try_get("name")?;
            let cols: Vec<String> =
                sqlx::query_scalar("SELECT name FROM pragma_index_info(?) ORDER BY seqno")
                    .bind(&name)
                    .fetch_all(&mut conn)
                    .await?;
            let sql: Option<String> =
                sqlx::query_scalar("SELECT sql FROM sqlite_master WHERE name = ?")
                    .bind(&name)
                    .fetch_optional(&mut conn)
                    .await?
                    .flatten();
            let normalized = sql
                .unwrap_or_default()
                .to_lowercase()
                .replace(['"', '(', ')', ' ', '\n', '\t'], "");
            let predicate = normalized
                .split_once("where")
                .map(|(_, term)| term)
                .unwrap_or_default();
            indices.insert(
                name,
                json!([
                    cols,
                    index.try_get::<i64, _>("unique")?,
                    index.try_get::<String, _>("origin")?,
                    index.try_get::<i64, _>("partial")?,
                    predicate
                ]),
            );
        }
        tables.insert(
            table,
            json!({"columns": columns, "without_rowid": wr, "indices": indices}),
        );
    }
    conn.close().await?;
    Ok(json!(tables))
}

#[tokio::test]
async fn a_fresh_file_has_exactly_the_v1_schema() -> TestResult {
    let (_legacy, old) = fixture("s5", false).await?;
    let fresh = tempfile::tempdir()?;
    let path = fresh.path().join("trace.db");
    drop(TraceStore::open(&path).await?);
    assert_eq!(shape(&path).await?, shape(&old).await?);
    let dump = logical_dump(&path).await?;
    assert_eq!(dump.version, 1);
    assert_eq!(dump.rows.get("seaql_migrations").map(Vec::len), Some(1));
    assert_eq!(
        dump.rows["seaql_migrations"][0][0],
        "'m0001_trace_baseline'"
    );
    Ok(())
}

#[tokio::test]
async fn an_existing_v1_file_is_adopted_with_every_row() -> TestResult {
    let (_legacy, path) = fixture("s5", true).await?;
    let (_expected, expected) = fixture("s5", true).await?;
    old_build_open(&expected).await?;
    let store = TraceStore::open(&path).await?;
    assert_eq!(
        without_migration(logical_dump(&path).await?),
        logical_dump(&expected).await?,
        "only documented crash recovery and the migration ledger may change"
    );
    let detail = store.run(A.parse()?).await?.ok_or("missing run")?;
    assert_eq!(detail.summary.status, RunStatus::Ok);
    assert_eq!(detail.summary.tabs, [TabId(3), TabId(9)]);
    assert_eq!(detail.host_data, Some(json!({"owner":1})));
    assert_eq!(detail.code.as_deref(), Some("return 1"));
    assert_eq!(detail.result, Some(json!(1)));
    assert_eq!(detail.operations.len(), 3);
    assert_eq!(detail.operations[0].notes, ["looked"]);
    assert_eq!(detail.operations[0].screenshot, Some(ShotState::Stored));
    assert_eq!(
        store.screenshot(OperationId::new(A.parse()?, 1)).await?,
        ShotRead::Expired,
        "a missing JPEG reads expired"
    );
    assert_eq!(
        store.screenshot(OperationId::new(A.parse()?, 2)).await?,
        ShotRead::Withheld
    );
    let interrupted = store
        .run(B.parse()?)
        .await?
        .ok_or("missing interrupted run")?;
    assert_eq!(interrupted.summary.status, RunStatus::Interrupted);
    assert_eq!(interrupted.summary.effects, Some(RunEffects::Unknown));
    assert!(interrupted.summary.complete);
    assert_eq!(interrupted.operations[0].delivery, Some(Delivery::Unknown));
    assert_eq!(interrupted.operations[1].delivery, Some(Delivery::NotSent));
    assert_eq!(store.open_writes(TabId(4)).await?, 0);
    assert_eq!(
        store
            .run_by_request(&RequestId::parse("repeat")?, Some(&TaskId::parse("task")?))
            .await?
            .map(|run| run.id),
        Some(C.parse()?)
    );
    assert_eq!(
        store
            .run_by_request(&RequestId::parse("global")?, None)
            .await?
            .map(|run| run.id),
        Some(B.parse()?)
    );
    assert_eq!(
        store
            .replay(A.parse()?, EpochMillis::from_millis(100))
            .await?,
        Some(json!({"ok":true}))
    );
    let first = store
        .runs(RunQuery {
            limit: 1,
            ..Default::default()
        })
        .await?;
    assert_eq!(first.items[0].id, B.parse()?);
    assert_eq!(
        first.next_cursor.as_deref(),
        Some(format!("20:{B}").as_str())
    );
    let second = store
        .runs(RunQuery {
            limit: 1,
            cursor: first.next_cursor,
            ..Default::default()
        })
        .await?;
    let old_cursor = store
        .runs(RunQuery {
            limit: 1,
            cursor: Some(format!("20:{B}")),
            ..Default::default()
        })
        .await?;
    assert_eq!(second, old_cursor);
    assert_eq!(second.items[0].id, A.parse()?);
    assert_eq!(
        store
            .private_tabs()
            .await?
            .get(&TabId(9))
            .map(|flag| flag.reason),
        Some(PrivateReason::Host)
    );
    let act = store
        .task_last_act(&TaskId::parse("task")?)
        .await?
        .ok_or("missing act")?;
    assert_eq!((act.tab, act.at), (TabId(3), EpochMillis::from_millis(13)));
    assert_eq!(
        store.task_last_tab(&TaskId::parse("task")?).await?,
        Some(TabId(3))
    );
    assert_eq!(
        store
            .trace_tail(TailQuery {
                task_id: Some(TaskId::parse("task")?),
                ..Default::default()
            })
            .await?
            .operations
            .len(),
        3
    );
    Ok(())
}

#[tokio::test]
async fn an_s4_file_gains_notes_and_takes_operation_ends() -> TestResult {
    let (_dir, path) = fixture("s4", true).await?;
    let store = TraceStore::open(&path).await?;
    let sink = spawn_writer(
        &store,
        Redactor::new(
            Arc::new(SecretSet::new(SecretSetConfig::default())),
            RedactionLimits::default(),
        ),
        WriterConfig::default(),
        None,
    );
    let id = OperationId::new(A.parse()?, 4);
    sink.op_started(OperationStarted {
        id,
        parent: None,
        name: "page.title".into(),
        kind: OperationKind::Builtin,
        effect: Effect::Read,
        tab: Some(TabId(3)),
        args: Value::Null,
        policy: Arc::default(),
        also_mask: None,
        at: EpochMillis::from_millis(40),
    });
    sink.op_notes(id, vec!["adopted note".into()]);
    sink.op_ended(OperationEnded {
        id,
        outcome: Outcome::Ok,
        delivery: Delivery::NotSent,
        sent_at: None,
        result: None,
        error: None,
        page_after: None,
        target: TargetSensitivity::Ordinary,
        also_mask: None,
        at: EpochMillis::from_millis(41),
    });
    sink.flush().await?;
    assert_eq!(
        store.operation(id).await?.ok_or("missing operation")?.notes,
        ["adopted note"]
    );
    Ok(())
}

#[tokio::test]
async fn an_s3_file_is_adopted_with_its_flags_and_request_history() -> TestResult {
    let (_dir, path) = fixture("s3", true).await?;
    let store = TraceStore::open(&path).await?;
    assert!(
        store
            .run_summary(A.parse()?)
            .await?
            .ok_or("missing run")?
            .complete
    );
    assert!(
        store
            .operation(OperationId::new(A.parse()?, 1))
            .await?
            .ok_or("missing op")?
            .screenshot
            .is_none()
    );
    assert!(store.private_tabs().await?.contains_key(&TabId(9)));
    assert_eq!(
        store
            .run_by_request(&RequestId::parse("global")?, None)
            .await?
            .map(|row| row.id),
        Some(B.parse()?)
    );
    let (_old, old) = fixture("s5", false).await?;
    assert_eq!(shape(&path).await?, shape(&old).await?);
    Ok(())
}

async fn refused_untouched(path: &Path) -> TestResult {
    let before = logical_dump(path).await?;
    assert!(
        matches!(TraceStore::open(path).await, Err(TraceError::Corrupt(_))),
        "unsupported schemas fail clearly"
    );
    assert_eq!(
        logical_dump(path).await?,
        before,
        "refusal must leave all schema, rows and markers intact"
    );
    Ok(())
}

#[tokio::test]
async fn a_session_era_file_is_refused_untouched() -> TestResult {
    let (_dir, path) = fixture("s2", false).await?;
    refused_untouched(&path).await
}

#[tokio::test]
async fn a_newer_user_version_is_refused_untouched() -> TestResult {
    let (_dir, path) = fixture("s5", true).await?;
    let mut conn = connection(&path).await?;
    sqlx::raw_sql("PRAGMA user_version = 2")
        .execute(&mut conn)
        .await?;
    conn.close().await?;
    refused_untouched(&path).await
}

#[tokio::test]
async fn a_diverged_migration_history_is_refused_untouched() -> TestResult {
    let (_dir, path) = fixture("s5", true).await?;
    let mut conn = connection(&path).await?;
    sqlx::raw_sql("CREATE TABLE seaql_migrations (version TEXT PRIMARY KEY, applied_at BIGINT NOT NULL); INSERT INTO seaql_migrations VALUES ('m9999_other', 1)").execute(&mut conn).await?;
    conn.close().await?;
    refused_untouched(&path).await
}

#[tokio::test]
async fn open_is_idempotent() -> TestResult {
    let (_dir, path) = fixture("s5", true).await?;
    drop(TraceStore::open(&path).await?);
    let before = logical_dump(&path).await?;
    drop(TraceStore::open(&path).await?);
    assert_eq!(logical_dump(&path).await?, before);
    Ok(())
}

#[tokio::test]
async fn a_failed_adoption_leaves_the_file_as_it_was() -> TestResult {
    let (_dir, path) = fixture("s4", true).await?;
    let mut conn = connection(&path).await?;
    sqlx::raw_sql("CREATE TABLE seaql_migrations (version TEXT PRIMARY KEY, applied_at BIGINT NOT NULL); CREATE TRIGGER reject_migration BEFORE INSERT ON seaql_migrations BEGIN SELECT RAISE(ABORT, 'reject migration record'); END").execute(&mut conn).await?;
    conn.close().await?;
    let before = logical_dump(&path).await?;
    assert!(TraceStore::open(&path).await.is_err());
    assert_eq!(
        logical_dump(&path).await?,
        before,
        "ALTER ADD notes must roll back with the failed migration record"
    );
    Ok(())
}

#[tokio::test]
async fn the_previous_build_still_opens_an_adopted_file() -> TestResult {
    let (_dir, path) = fixture("s5", true).await?;
    drop(TraceStore::open(&path).await?);
    let before = logical_dump(&path).await?;
    old_build_open(&path).await?;
    assert_eq!(logical_dump(&path).await?, before);
    Ok(())
}

#[tokio::test]
async fn a_refused_open_closes_every_connection() -> TestResult {
    let (_dir, path) = fixture("s2", false).await?;
    let before = logical_dump(&path).await?;
    assert!(TraceStore::open(&path).await.is_err());
    let aside = path.with_extension("aside");
    tokio::fs::rename(&path, &aside).await?;
    for suffix in ["-wal", "-shm"] {
        let _ = tokio::fs::remove_file(format!("{}{suffix}", path.display())).await;
    }
    let fresh = TraceStore::open(&path).await?;
    assert!(fresh.runs(RunQuery::default()).await?.items.is_empty());
    assert_eq!(logical_dump(&aside).await?, before);
    let mut conn = connection(&path).await?;
    sqlx::query("INSERT INTO private_tabs VALUES (5, 1, 'host')")
        .execute(&mut conn)
        .await?;
    conn.close().await?;
    assert!(
        fresh.private_tabs().await?.contains_key(&TabId(5)),
        "the replacement remains usable"
    );
    Ok(())
}

#[tokio::test]
async fn stored_values_the_build_does_not_know_keep_their_rules() -> TestResult {
    let (_dir, path) = fixture("s5", true).await?;
    let store = TraceStore::open(&path).await?;
    let mut conn = connection(&path).await?;
    sqlx::query("UPDATE runs SET status = 'later_status' WHERE id = ?")
        .bind(A)
        .execute(&mut conn)
        .await?;
    conn.close().await?;
    assert_eq!(
        store
            .private_tabs()
            .await?
            .get(&TabId(9))
            .map(|row| row.reason),
        Some(PrivateReason::Host)
    );
    assert!(matches!(
        store.run(A.parse()?).await,
        Err(TraceError::Corrupt(_))
    ));
    assert!(
        store.run(C.parse()?).await?.is_some(),
        "one bad row does not prevent independent reads"
    );
    Ok(())
}

#[tokio::test]
async fn a_newer_migration_is_refused_even_when_its_fence_says_compatible() -> TestResult {
    let (_dir, path) = fixture("s5", true).await?;
    drop(TraceStore::open(&path).await?);
    let mut conn = connection(&path).await?;
    sqlx::raw_sql("ALTER TABLE operations ADD COLUMN later_field TEXT; INSERT INTO seaql_migrations VALUES ('m9999_later', 1)").execute(&mut conn).await?;
    conn.close().await?;
    refused_untouched(&path).await?;
    let error = TraceStore::open(&path)
        .await
        .err()
        .ok_or("newer file opened")?;
    assert!(matches!(error,TraceError::Corrupt(message) if message.contains("m9999_later")));
    Ok(())
}

#[tokio::test]
async fn a_store_with_no_stored_shots_sweeps_cleanly() -> TestResult {
    let (_dir, path) = fixture("s5", true).await?;
    let mut conn = connection(&path).await?;
    sqlx::query("UPDATE operations SET screenshot = NULL, screenshot_bytes = NULL")
        .execute(&mut conn)
        .await?;
    conn.close().await?;
    let store = TraceStore::open(&path).await?;
    let sink = spawn_writer(
        &store,
        Redactor::new(
            Arc::new(SecretSet::new(SecretSetConfig::default())),
            RedactionLimits::default(),
        ),
        WriterConfig::default(),
        None,
    );
    let swept = sink
        .sweep(RetentionPolicy::default(), EpochMillis::from_millis(40))
        .await?;
    assert_eq!(swept.screenshots, 0);
    sink.flush().await?;
    assert_eq!(store.runs(RunQuery::default()).await?.items.len(), 3);
    Ok(())
}

#[tokio::test]
async fn a_runs_tabs_come_back_sorted_from_one_statement() -> TestResult {
    let (_dir, path) = fixture("s5", true).await?;
    let store = TraceStore::open(&path).await?;
    let expected = vec![TabId(3), TabId(9)];
    assert_eq!(
        store.run(A.parse()?).await?.ok_or("run")?.summary.tabs,
        expected
    );
    assert_eq!(
        store.run_summary(A.parse()?).await?.ok_or("summary")?.tabs,
        expected
    );
    assert_eq!(
        store
            .runs(RunQuery {
                request_id: Some(RequestId::parse("repeat")?),
                ..Default::default()
            })
            .await?
            .items[0]
            .tabs,
        expected
    );
    // A is the first run for this new request; the duplicate-key fixture above also proves earliest ordering.
    let mut conn = connection(&path).await?;
    sqlx::query("UPDATE runs SET request_id = 'tabs' WHERE id = ?")
        .bind(A)
        .execute(&mut conn)
        .await?;
    conn.close().await?;
    assert_eq!(
        store
            .run_by_request(&RequestId::parse("tabs")?, None)
            .await?
            .ok_or("request")?
            .tabs,
        expected
    );
    assert_eq!(
        store
            .trace_tail(TailQuery::default())
            .await?
            .runs
            .iter()
            .find(|run| run.id.to_string() == A)
            .ok_or("tail")?
            .tabs,
        expected
    );
    Ok(())
}
