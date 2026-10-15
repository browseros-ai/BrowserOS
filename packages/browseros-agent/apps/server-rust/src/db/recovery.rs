//! Best-effort row salvage into a freshly migrated DB, never rewriting the preserved original.
//! Each table and each insert stands alone so damaged pages or one invalid row cannot erase
//! readable data elsewhere. The optional SQLite recovery CLI also handles broken schema pages.

use futures_util::TryStreamExt;
use sea_orm::sqlx::{
    self, Column, Connection, Row, TypeInfo, ValueRef,
    sqlite::{SqliteConnectOptions, SqliteConnection},
};
use std::{collections::BTreeMap, path::Path, time::Duration};

const TABLES: &[&str] = &[
    "providers",
    "conversations",
    "oauth_tokens",
    "scheduled_jobs",
    "scheduled_job_runs",
];

pub(super) async fn salvage(aside: &Path, fresh: &Path) -> BTreeMap<String, u64> {
    let mut counts = TABLES
        .iter()
        .map(|table| ((*table).to_owned(), 0))
        .collect();
    let Ok(mut destination) = SqliteConnection::connect_with(
        &SqliteConnectOptions::new()
            .filename(fresh)
            .foreign_keys(false)
            .busy_timeout(Duration::from_secs(5)),
    )
    .await
    else {
        tracing::error!("could not open fresh database for salvage");
        return counts;
    };
    let complete = copy_tables(aside, &mut destination, &mut counts).await;
    if !complete {
        // .recover scans pages even when sqlite_schema is broken. It is optional because some
        // installations do not ship the sqlite3 CLI; ordinary readable-table salvage still runs.
        let recovered = aside.with_extension(format!("recovery-{}.tmp", uuid::Uuid::new_v4()));
        let cleanup = RecoveryFile(recovered.clone());
        if recover_pages(aside, &recovered).await.is_ok() {
            copy_tables(&recovered, &mut destination, &mut counts).await;
        } else {
            tracing::error!(
                "SQLite page recovery unavailable or unsuccessful; retained original for manual recovery"
            );
        }
        drop(cleanup);
    }
    if let Err(error) = destination.close().await {
        tracing::error!(%error, "closing salvage connection failed");
    }
    counts
}

async fn copy_tables(
    source: &Path,
    destination: &mut SqliteConnection,
    counts: &mut BTreeMap<String, u64>,
) -> bool {
    let Ok(mut source) = SqliteConnection::connect_with(
        &SqliteConnectOptions::new().filename(source).read_only(true),
    )
    .await
    else {
        return false;
    };
    let mut complete = true;
    for table in TABLES {
        match copy_table(&mut source, destination, table, counts).await {
            Ok(()) => {}
            Err(error) => {
                complete = false;
                tracing::error!(%table, %error, "table salvage incomplete; continuing with other tables");
            }
        }
    }
    let _ = source.close().await;
    complete
}

async fn copy_table(
    source: &mut SqliteConnection,
    destination: &mut SqliteConnection,
    table: &str,
    counts: &mut BTreeMap<String, u64>,
) -> Result<(), sqlx::Error> {
    let columns: Vec<String> = sqlx::query_scalar("SELECT name FROM pragma_table_info(?)")
        .bind(table)
        .fetch_all(&mut *source)
        .await?;
    if columns.is_empty() {
        return Err(sqlx::Error::RowNotFound);
    }
    let target: Vec<String> = sqlx::query_scalar("SELECT name FROM pragma_table_info(?)")
        .bind(table)
        .fetch_all(&mut *destination)
        .await?;
    let columns = columns
        .into_iter()
        .filter(|column| target.contains(column))
        .map(|column| quote(&column))
        .collect::<Vec<_>>()
        .join(",");
    let query = format!("SELECT {columns} FROM {}", quote(table));
    let mut rows = sqlx::query(&query).fetch(&mut *source);
    while let Some(row) = rows.try_next().await? {
        let mut insert = sqlx::QueryBuilder::new(format!(
            "INSERT OR IGNORE INTO {} ({columns}) VALUES (",
            quote(table)
        ));
        let mut values = insert.separated(",");
        for (index, column) in row.columns().iter().enumerate() {
            let value = row.try_get_raw(index)?;
            if value.is_null() {
                values.push_bind(Option::<String>::None);
            } else {
                match value.type_info().name() {
                    "INTEGER" | "INT" | "BOOLEAN" => {
                        values.push_bind(row.try_get::<i64, _>(index)?);
                    }
                    "REAL" | "FLOAT" => {
                        values.push_bind(row.try_get::<f64, _>(index)?);
                    }
                    "BLOB" => {
                        values.push_bind(row.try_get::<Vec<u8>, _>(index)?);
                    }
                    _ => {
                        values.push_bind(row.try_get::<String, _>(column.name())?);
                    }
                }
            }
        }
        insert.push(")");
        match insert.build().execute(&mut *destination).await {
            Ok(result) => {
                *counts.entry(table.to_owned()).or_default() += result.rows_affected();
            }
            Err(error) => tracing::error!(%table, %error, "could not salvage one row; continuing"),
        }
    }
    Ok(())
}

async fn recover_pages(aside: &Path, recovered: &Path) -> Result<(), Box<dyn std::error::Error>> {
    // Work on a copy: sqlite3 may create WAL/SHM even for recovery, and damaged SQLite
    // headers can prevent read-only opens until those sidecars exist. The aside stays intact.
    let scratch = recovered.with_extension("source.tmp");
    let scratch_cleanup = RecoveryFile(scratch.clone());
    tokio::fs::copy(aside, &scratch).await?;
    let wal = std::path::PathBuf::from(format!("{}-wal", aside.display()));
    if tokio::fs::try_exists(&wal).await? {
        tokio::fs::copy(wal, format!("{}-wal", scratch.display())).await?;
    }
    // A corrupt schema-root page tag loses table names in .recover's lost_and_found. These
    // are the only two valid SQLite table B-tree tags. Try them on scratch only, accepting a
    // repair solely when SQLite's full integrity_check succeeds. No row bytes are guessed.
    let original = tokio::fs::read(&scratch).await?;
    if original.len() > 100 && !matches!(original[100], 5 | 13) {
        for tag in [13, 5] {
            let mut candidate = original.clone();
            candidate[100] = tag;
            tokio::fs::write(&scratch, candidate).await?;
            if let Ok(mut connection) =
                SqliteConnection::connect_with(&SqliteConnectOptions::new().filename(&scratch))
                    .await
            {
                let check = sqlx::query_scalar::<_, String>("PRAGMA integrity_check")
                    .fetch_all(&mut connection)
                    .await;
                connection.close().await?;
                if check.is_ok_and(|rows| rows == ["ok"]) {
                    tokio::fs::copy(&scratch, recovered).await?;
                    drop(scratch_cleanup);
                    return Ok(());
                }
            }
        }
        tokio::fs::write(&scratch, original).await?;
    }
    // No shell, and neither recovered SQL (which contains tokens) nor CLI stderr is logged.
    let output = tokio::time::timeout(
        Duration::from_secs(60),
        tokio::process::Command::new("sqlite3")
            .arg(&scratch)
            .arg(".recover")
            .kill_on_drop(true)
            .output(),
    )
    .await??;
    if output.stdout.is_empty() {
        return Err("SQLite recovery returned no data".into());
    }
    let sql = String::from_utf8(output.stdout)?;
    let mut connection = SqliteConnection::connect_with(
        &SqliteConnectOptions::new()
            .filename(recovered)
            .create_if_missing(true)
            .foreign_keys(false),
    )
    .await?;
    // A nonzero .recover status can still carry useful recovered rows. Execute its output and
    // keep partial results even if the final statement fails; the table copier checks each row.
    let _ = sqlx::raw_sql(&sql).execute(&mut connection).await;
    connection.close().await?;
    drop(scratch_cleanup);
    Ok(())
}

fn quote(identifier: &str) -> String {
    format!("\"{}\"", identifier.replace('"', "\"\""))
}

struct RecoveryFile(std::path::PathBuf);
impl Drop for RecoveryFile {
    fn drop(&mut self) {
        for suffix in ["", "-wal", "-shm", "-journal"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", self.0.display()));
        }
    }
}
