//! Neo: What every test needs: a database path in its own directory, and a plain connection to look
//! at what the store wrote.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use sea_orm::{ConnectionTrait, DatabaseConnection, DbBackend, Statement};
use tempfile::TempDir;

use crate::db::migration::LEDGER;
use crate::ingest::Command;
use crate::{apply, db, writer};

/// A fresh directory and the database path in it; the directory lives as long as the guard.
pub(crate) fn scratch() -> (TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("neo-traces.sqlite");
    (dir, path)
}

/// A connection of its own to the file, beside any store's.
pub(crate) async fn inspect(path: &Path) -> DatabaseConnection {
    db::connect_writer(path).await.unwrap()
}

pub(crate) async fn exec(conn: &DatabaseConnection, sql: &str) {
    conn.execute_unprepared(sql).await.unwrap();
}

/// The single integer `sql` selects.
pub(crate) async fn count(conn: &DatabaseConnection, sql: &str) -> i64 {
    conn.query_one(Statement::from_string(DbBackend::Sqlite, sql))
        .await
        .unwrap()
        .unwrap()
        .try_get_by_index::<i64>(0)
        .unwrap()
}

/// The strings `sql` selects, in its order.
pub(crate) async fn texts(conn: &DatabaseConnection, sql: &str) -> Vec<String> {
    conn.query_all(Statement::from_string(DbBackend::Sqlite, sql))
        .await
        .unwrap()
        .iter()
        .map(|row| row.try_get_by_index::<String>(0).unwrap())
        .collect()
}

/// The names of the file's own tables or indexes (`kind`), without SQLite's.
pub(crate) async fn objects(conn: &DatabaseConnection, kind: &str) -> BTreeSet<String> {
    texts(
        conn,
        &format!(
            "SELECT name FROM sqlite_master WHERE type = '{kind}' AND name NOT LIKE 'sqlite_%'"
        ),
    )
    .await
    .into_iter()
    .collect()
}

/// The store's ledger, in the order its rows were written.
pub(crate) async fn ledger(conn: &DatabaseConnection) -> Vec<String> {
    texts(
        conn,
        &format!("SELECT version FROM {LEDGER} ORDER BY rowid"),
    )
    .await
}

/// Hand-built runtime facts keep the store tests independent of browsers and runtime internals.
pub(crate) fn started(session: &str) -> neo_runtime::RunStarted {
    use neo_runtime::{IdKey, PageId, RunRef, RunStarted, SessionKey};
    use ulid::Ulid;
    RunStarted {
        run: RunRef {
            id: Ulid::new().to_string().parse().unwrap(),
            session: Some(SessionKey::new(session)),
        },
        client: Some("test".into()),
        ids: [(IdKey::new("zuzu.task").unwrap(), "t_1".into())].into(),
        page: PageId::new("p".into()),
        timeout_ms: 1000,
        code_bytes: 10,
        started_at_ms: writer::now(),
    }
}

pub(crate) fn before(run: neo_runtime::RunId, no: u64) -> neo_runtime::RunEvent {
    use neo_runtime::{PageId, PageRef, RunEvent};
    RunEvent::BeforeCall {
        run,
        call: no,
        parent: Some(1),
        object: "frame@p".into(),
        page: Some(PageRef {
            target: PageId::new("p".into()),
            tab_id: Some(7),
        }),
        method: "click".into(),
        title: Some("Click".into()),
        selector: Some("#button".into()),
        location: None,
        started_at_ms: 1000,
        offset_us: no * 10,
    }
}

pub(crate) fn after(run: neo_runtime::RunId, no: u64) -> neo_runtime::RunEvent {
    use neo_runtime::{CallRecord, CallResult, Delivery, PageId, PageRef, RunEvent};
    use serde_json::json;
    use std::sync::Arc;
    use std::time::Duration;
    RunEvent::AfterCall {
        run,
        call: no,
        parent: Some(1),
        page: Some(PageRef {
            target: PageId::new("p".into()),
            tab_id: Some(7),
        }),
        record: Arc::new(CallRecord {
            call_id: no,
            guid: "frame@p".into(),
            method: "click".into(),
            title: Some("Click".into()),
            params: json!({"selector":"#button"}),
            location: None,
            timeout_ms: Some(500.0),
            started_at_ms: 1000,
            duration: Duration::from_micros(20),
            result: CallResult::Ok,
            delivery: Some(Delivery::Unknown),
            log: vec!["recorded detail".into()],
        }),
        params_truncated: false,
        offset_us: no * 10,
    }
}

pub(crate) fn getter(run: neo_runtime::RunId, no: u64) -> neo_runtime::RunEvent {
    use neo_runtime::{GetterField, GetterResult, RunEvent};
    RunEvent::Getter {
        run,
        no,
        object: "page@p".into(),
        field: GetterField::Url,
        page: None,
        result: GetterResult::Ok {
            summary: "https://example.com/".into(),
        },
        offset_us: no * 10,
        duration_us: 2,
    }
}

pub(crate) fn ended(run: neo_runtime::RunId, calls_made: u64) -> neo_runtime::RunEvent {
    use neo_runtime::{EndCause, RunEvent, RunStatus};
    use std::sync::Arc;
    RunEvent::RunEnded {
        run,
        status: RunStatus::Ok,
        cause: EndCause::Returned,
        error: None,
        value: None,
        value_truncated: false,
        logs: Arc::from([]),
        script: Some(Arc::from("return 1")),
        calls_made,
        unreadable_calls: 0,
        duration_us: 1000,
    }
}

pub(crate) fn event(event: neo_runtime::RunEvent) -> Command {
    Command::Event {
        event,
        stub: false,
        loss: (0, 0),
    }
}

pub(crate) async fn apply(conn: &DatabaseConnection, commands: &[Command]) {
    use sea_orm::TransactionTrait;
    let txn = conn.begin().await.unwrap();
    for command in commands {
        apply::command(&txn, command, 10_000).await.unwrap();
    }
    txn.commit().await.unwrap();
}
