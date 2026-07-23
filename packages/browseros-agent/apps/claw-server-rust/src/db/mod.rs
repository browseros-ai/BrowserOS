pub mod audit_log;
pub mod entities;
mod migration;
pub mod recording_index;
pub mod session_efficiency_stats;
pub mod session_tabs;

pub use audit_log::AuditLog;
pub use recording_index::{
    AppendDocumentBatch, LegacyClaimRow, LegacyRecordingRow, RecordingIndex, RecordingStreamRow,
    SessionTabWindow, StreamMatchRow,
};
pub use session_efficiency_stats::SessionEfficiencyStatsRepository;
pub use session_tabs::SessionTabLedger;

use crate::error::{AppError, AppResult, IoPath};
use migration::Migrator;
use sea_orm::{
    DatabaseConnection, DbErr, RuntimeErr, SqlxSqliteConnector,
    sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous},
};
use sea_orm_migration::MigratorTrait;
use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    time::Duration,
};

pub const DATABASE_FILENAME: &str = "browserclaw.sqlite";

#[derive(Clone)]
pub struct Database(DatabaseConnection);

impl Database {
    /// Opens and migrates BrowserClaw's shared durable state database.
    pub async fn open(path: impl AsRef<Path>) -> AppResult<Self> {
        open_and_migrate::<Migrator>(path.as_ref()).await.map(Self)
    }

    pub(in crate::db) fn connection(&self) -> &DatabaseConnection {
        &self.0
    }
}

impl From<DbErr> for AppError {
    fn from(source: DbErr) -> Self {
        Self::Db(Box::new(source))
    }
}

/// Opens a SQLite database, applies its migrator, and recovers broken files once.
async fn open_and_migrate<M: MigratorTrait>(path: &Path) -> AppResult<DatabaseConnection> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await.with_path(parent)?;
    }

    match connect_and_migrate::<M>(path).await {
        Ok(conn) => Ok(conn),
        Err(_) => {
            back_up_database(path).await?;
            connect_and_migrate::<M>(path).await.map_err(AppError::from)
        }
    }
}

async fn connect_and_migrate<M: MigratorTrait>(path: &Path) -> Result<DatabaseConnection, DbErr> {
    let options = SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .foreign_keys(true)
        // Freshly created databases stick in incremental auto-vacuum, so
        // `PRAGMA incremental_vacuum` can return freed pages to the OS after an
        // audit-retention sweep. Existing databases are converted on first reclaim.
        .pragma("auto_vacuum", "incremental")
        .busy_timeout(Duration::from_secs(5));
    // A single connection preserves the old mutex-serialized write behavior and avoids SQLite write-upgrade contention.
    // A blocking task prevents paused Tokio clocks from expiring SQLx's acquire timeout before its SQLite worker responds.
    let runtime = tokio::runtime::Handle::current();
    let pool = tokio::task::spawn_blocking(move || {
        runtime.block_on(
            SqlitePoolOptions::new()
                .max_connections(1)
                .connect_with(options),
        )
    })
    .await
    .map_err(|error| DbErr::Custom(format!("SQLite connection task failed: {error}")))?
    .map_err(|error| DbErr::Conn(RuntimeErr::SqlxError(error)))?;
    let conn = SqlxSqliteConnector::from_sqlx_sqlite_pool(pool);
    if let Err(error) = M::up(&conn, None).await {
        conn.close().await?;
        return Err(error);
    }
    Ok(conn)
}

async fn back_up_database(path: &Path) -> AppResult<()> {
    for (source_suffix, backup_suffix) in [("", ".bak"), ("-wal", ".bak-wal"), ("-shm", ".bak-shm")]
    {
        let source = append_suffix(path, source_suffix);
        let backup = append_suffix(path, backup_suffix);
        match tokio::fs::rename(&source, &backup).await {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(AppError::Io {
                    path: Some(source),
                    source: error,
                });
            }
        }
    }
    Ok(())
}

fn append_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut value = OsString::from(path.as_os_str());
    value.push(suffix);
    value.into()
}

#[cfg(test)]
mod tests {
    use super::{
        AuditLog, DATABASE_FILENAME, Database, append_suffix, audit_log::ListDispatchesQuery,
        back_up_database, connect_and_migrate, migration::Migrator,
    };
    use sea_orm::{
        ConnectionTrait, DbBackend, Statement,
        sqlx::{
            self, Connection, Row,
            sqlite::{SqliteConnectOptions, SqliteConnection},
        },
    };
    use sea_orm_migration::{MigrationTrait, MigratorTrait, async_trait};
    use std::{collections::HashSet, path::Path};
    use tempfile::tempdir;

    // Frozen migrations from the retired TypeScript server prove existing
    // installations still upgrade into the Rust-owned baseline schema.
    const TS_0000: &str =
        include_str!("../../tests/fixtures/legacy-drizzle/0000_add_tool_dispatches.sql");
    const TS_0001: &str =
        include_str!("../../tests/fixtures/legacy-drizzle/0001_add_agent_session_events.sql");
    const TS_0002: &str =
        include_str!("../../tests/fixtures/legacy-drizzle/0002_default_created_at_in_js.sql");

    #[tokio::test]
    async fn fresh_file_has_the_complete_baseline_schema() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let db = Database::open(dir.path().join(DATABASE_FILENAME)).await?;
        let objects = db
            .connection()
            .query_all(Statement::from_string(
                DbBackend::Sqlite,
                "SELECT name FROM sqlite_master WHERE type IN ('table', 'index')".to_string(),
            ))
            .await?;
        let names = objects
            .into_iter()
            .map(|row| row.try_get::<String>("", "name"))
            .collect::<Result<HashSet<_>, _>>()?;

        for table in [
            "tool_dispatches",
            "agent_session_starts",
            "agent_session_ends",
            "tasks",
            "tab_recordings",
            "tab_claims",
            "session_tabs",
            "recording_streams",
            "recording_payloads",
            "recording_batches",
            "session_efficiency_stats",
            "seaql_migrations",
        ] {
            assert!(names.contains(table), "missing table {table}");
        }
        for index in [
            "tool_dispatches_created_at_idx",
            "tool_dispatches_agent_created_idx",
            "tool_dispatches_session_idx",
            "agent_session_starts_session_idx",
            "agent_session_starts_created_at_idx",
            "agent_session_ends_session_idx",
            "agent_session_ends_created_at_idx",
            "tasks_cursor_idx",
            "tasks_agent_cursor_idx",
            "tasks_status_cursor_idx",
            "tasks_site_cursor_idx",
            "tasks_started_idx",
            "tab_recordings_last_event_idx",
            "tab_claims_target_idx",
            "tab_claims_session_idx",
            "session_tabs_session_idx",
            "session_tabs_tab_window_idx",
            "session_tabs_one_live_owner_idx",
            "recording_streams_tab_time_idx",
            "recording_streams_retention_idx",
            "session_efficiency_stats_ended_at_idx",
        ] {
            assert!(names.contains(index), "missing index {index}");
        }

        let efficiency_columns = db
            .connection()
            .query_all(Statement::from_string(
                DbBackend::Sqlite,
                "PRAGMA table_info(session_efficiency_stats)".to_string(),
            ))
            .await?
            .into_iter()
            .map(|row| {
                Ok((
                    row.try_get::<String>("", "name")?,
                    row.try_get::<i64>("", "pk")?,
                ))
            })
            .collect::<Result<Vec<_>, sea_orm::DbErr>>()?;
        assert_eq!(
            efficiency_columns
                .iter()
                .map(|(name, _)| name.as_str())
                .collect::<HashSet<_>>(),
            HashSet::from([
                "session_id",
                "ended_at",
                "dispatch_count",
                "active_duration_ms",
                "tool_input_token_estimate",
                "tool_output_token_estimate",
                "screenshot_baseline_token_estimate",
                "efficiency_estimator_version",
                "computed_at",
            ])
        );
        assert_eq!(
            efficiency_columns
                .iter()
                .find(|(name, _)| name == "session_id")
                .map(|(_, primary_key)| *primary_key),
            Some(1)
        );

        let migrations = db
            .connection()
            .query_all(Statement::from_string(
                DbBackend::Sqlite,
                "SELECT version FROM seaql_migrations".to_string(),
            ))
            .await?;
        assert_eq!(migrations.len(), 7);
        assert_eq!(
            migrations[0].try_get::<String>("", "version")?,
            "m0001_baseline"
        );
        assert_eq!(
            migrations[1].try_get::<String>("", "version")?,
            "m0002_add_recordings_and_claims"
        );
        assert_eq!(
            migrations[2].try_get::<String>("", "version")?,
            "m0003_document_recordings_and_tab_ownership"
        );
        assert_eq!(
            migrations[3].try_get::<String>("", "version")?,
            "m0004_atomic_recording_payloads"
        );
        assert_eq!(
            migrations[4].try_get::<String>("", "version")?,
            "m0005_reclassify_task_status"
        );
        assert_eq!(
            migrations[5].try_get::<String>("", "version")?,
            "m0006_add_tool_token_estimates"
        );
        assert_eq!(
            migrations[6].try_get::<String>("", "version")?,
            "m0007_add_session_efficiency_stats"
        );
        Ok(())
    }

    struct MigratorThrough6;

    #[async_trait::async_trait]
    impl MigratorTrait for MigratorThrough6 {
        fn migrations() -> Vec<Box<dyn MigrationTrait>> {
            Migrator::migrations().into_iter().take(6).collect()
        }
    }

    #[tokio::test]
    async fn migration_7_upgrades_a_version_6_database_once() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let path = dir.path().join(DATABASE_FILENAME);
        let version_6 = connect_and_migrate::<MigratorThrough6>(&path).await?;
        version_6.close().await?;

        let upgraded = Database::open(&path).await?;
        let objects = upgraded
            .connection()
            .query_all(Statement::from_string(
                DbBackend::Sqlite,
                "SELECT name FROM sqlite_master WHERE type IN ('table', 'index')".to_string(),
            ))
            .await?;
        let names = objects
            .into_iter()
            .map(|row| row.try_get::<String>("", "name"))
            .collect::<Result<HashSet<_>, _>>()?;
        assert!(names.contains("session_efficiency_stats"));
        assert!(names.contains("session_efficiency_stats_ended_at_idx"));

        let migrations = upgraded
            .connection()
            .query_all(Statement::from_string(
                DbBackend::Sqlite,
                "SELECT version FROM seaql_migrations ORDER BY version".to_string(),
            ))
            .await?;
        assert_eq!(migrations.len(), 7);
        assert_eq!(
            migrations
                .iter()
                .filter(|row| {
                    row.try_get::<String>("", "version").as_deref()
                        == Ok("m0007_add_session_efficiency_stats")
                })
                .count(),
            1
        );
        upgraded.0.close().await?;

        let reopened = Database::open(&path).await?;
        let migration_count = reopened
            .connection()
            .query_one(Statement::from_string(
                DbBackend::Sqlite,
                "SELECT COUNT(*) AS count FROM seaql_migrations".to_string(),
            ))
            .await?
            .ok_or_else(|| anyhow::anyhow!("migration count missing"))?
            .try_get::<i64>("", "count")?;
        assert_eq!(migration_count, 7);
        Ok(())
    }

    #[tokio::test]
    async fn ts_snapshot_upgrades_in_place_and_preserves_dispatches() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let path = dir.path().join(DATABASE_FILENAME);
        let options = sqlite_options(&path);
        let mut conn = SqliteConnection::connect_with(&options).await?;
        for migration in [TS_0000, TS_0001, TS_0002] {
            sqlx::raw_sql(migration).execute(&mut conn).await?;
        }
        sqlx::raw_sql(
            r#"CREATE TABLE "__drizzle_migrations" (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                hash TEXT NOT NULL,
                created_at NUMERIC
            )"#,
        )
        .execute(&mut conn)
        .await?;
        sqlx::query(
            "INSERT INTO tool_dispatches
                (created_at, agent_id, slug, agent_label, session_id, tool_name)
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(123_i64)
        .bind("agent-id")
        .bind("agent")
        .bind("Agent")
        .bind("session-id")
        .bind("navigate")
        .execute(&mut conn)
        .await?;
        conn.close().await?;

        let audit = AuditLog::new(Database::open(&path).await?);
        let rows = audit
            .list_dispatches(ListDispatchesQuery {
                session_id: Some("session-id".to_string()),
                ..Default::default()
            })
            .await?
            .rows;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].created_at, 123);
        assert_eq!(rows[0].tool_name, "navigate");
        assert_eq!(rows[0].dispatch_id, None);
        assert!(!rows[0].has_screenshot);
        assert_eq!(rows[0].tool_input_token_estimate, 0);
        assert_eq!(rows[0].tool_output_token_estimate, 0);
        assert_eq!(rows[0].token_estimator_version, 0);

        let mut conn = SqliteConnection::connect_with(&options).await?;
        let columns = sqlx::query("PRAGMA table_info(tool_dispatches)")
            .fetch_all(&mut conn)
            .await?
            .into_iter()
            .map(|row| row.try_get::<String, _>("name"))
            .collect::<Result<HashSet<_>, _>>()?;
        assert!(columns.contains("dispatch_id"));
        assert!(columns.contains("has_screenshot"));
        assert!(columns.contains("tab_id"));
        assert!(columns.contains("tool_input_token_estimate"));
        assert!(columns.contains("tool_output_token_estimate"));
        assert!(columns.contains("token_estimator_version"));
        let drizzle_ledger: Option<i64> = sqlx::query_scalar(
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = '__drizzle_migrations'",
        )
        .fetch_optional(&mut conn)
        .await?;
        assert_eq!(drizzle_ledger, None);
        conn.close().await?;
        Ok(())
    }

    #[tokio::test]
    async fn garbage_file_is_backed_up_and_recreated() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let path = dir.path().join(DATABASE_FILENAME);
        let backup = path.with_extension("sqlite.bak");
        tokio::fs::write(&path, b"not a sqlite database").await?;

        let audit = AuditLog::new(Database::open(&path).await?);
        assert_eq!(tokio::fs::read(&backup).await?, b"not a sqlite database");
        audit
            .record_session_start("session-id", "agent-id", "agent", "Agent", "test", "1")
            .await?;
        assert!(audit.get_task_summary("session-id").await?.is_some());
        Ok(())
    }

    #[tokio::test]
    async fn database_backups_include_wal_and_shm_sidecars() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let path = dir.path().join(DATABASE_FILENAME);
        for (source_suffix, contents) in [
            ("", b"database".as_slice()),
            ("-wal", b"wal".as_slice()),
            ("-shm", b"shm".as_slice()),
        ] {
            tokio::fs::write(append_suffix(&path, source_suffix), contents).await?;
        }

        back_up_database(&path).await?;

        for (source_suffix, backup_suffix, contents) in [
            ("", ".bak", b"database".as_slice()),
            ("-wal", ".bak-wal", b"wal".as_slice()),
            ("-shm", ".bak-shm", b"shm".as_slice()),
        ] {
            assert!(!append_suffix(&path, source_suffix).exists());
            assert_eq!(
                tokio::fs::read(append_suffix(&path, backup_suffix)).await?,
                contents
            );
        }
        Ok(())
    }

    #[tokio::test]
    async fn double_open_does_not_duplicate_migration_records() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let path = dir.path().join(DATABASE_FILENAME);
        let first = Database::open(&path).await?;
        first.0.close().await?;

        let second = Database::open(&path).await?;
        let migrations = second
            .connection()
            .query_all(Statement::from_string(
                DbBackend::Sqlite,
                "SELECT version FROM seaql_migrations".to_string(),
            ))
            .await?;
        assert_eq!(migrations.len(), 7);
        Ok(())
    }

    fn sqlite_options(path: &Path) -> SqliteConnectOptions {
        SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true)
    }
}
