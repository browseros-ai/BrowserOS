//! What every test needs: a database path in its own directory, and a plain connection to look
//! at what the store wrote.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use sea_orm::{ConnectionTrait, DatabaseConnection, DbBackend, Statement};
use tempfile::TempDir;

use crate::db;
use crate::db::migration::LEDGER;

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
