pub mod entities;
mod migration;

use crate::{db::open_and_migrate, error::AppResult};
use migration::AuditMigrator;
use sea_orm::DatabaseConnection;
use std::path::Path;

#[derive(Clone)]
pub struct AuditDb(DatabaseConnection);

impl AuditDb {
    /// Opens and migrates the audit database.
    pub async fn open(path: impl AsRef<Path>) -> AppResult<Self> {
        open_and_migrate::<AuditMigrator>(path.as_ref())
            .await
            .map(Self)
    }

    pub(crate) fn connection(&self) -> &DatabaseConnection {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::AuditDb;
    use crate::capture::audit::{AuditService, ListDispatchesQuery};
    use sea_orm::{
        ConnectionTrait, DbBackend, Statement,
        sqlx::{
            self, Connection, Row,
            sqlite::{SqliteConnectOptions, SqliteConnection},
        },
    };
    use std::{collections::HashSet, path::Path};
    use tempfile::tempdir;

    const TS_0000: &str =
        include_str!("../../../../claw-server/drizzle/0000_add_tool_dispatches.sql");
    const TS_0001: &str =
        include_str!("../../../../claw-server/drizzle/0001_add_agent_session_events.sql");
    const TS_0002: &str =
        include_str!("../../../../claw-server/drizzle/0002_default_created_at_in_js.sql");

    #[tokio::test]
    async fn fresh_file_has_the_complete_baseline_schema() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let db = AuditDb::open(dir.path().join("audit.sqlite")).await?;
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
        ] {
            assert!(names.contains(index), "missing index {index}");
        }

        let migrations = db
            .connection()
            .query_all(Statement::from_string(
                DbBackend::Sqlite,
                "SELECT version FROM seaql_migrations".to_string(),
            ))
            .await?;
        assert_eq!(migrations.len(), 4);
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
        Ok(())
    }

    #[tokio::test]
    async fn ts_snapshot_upgrades_in_place_and_preserves_dispatches() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let path = dir.path().join("audit.sqlite");
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

        let audit = AuditService::open(&path).await?;
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
        let path = dir.path().join("audit.sqlite");
        let backup = path.with_extension("sqlite.bak");
        tokio::fs::write(&path, b"not a sqlite database").await?;

        let audit = AuditService::open(&path).await?;
        assert_eq!(tokio::fs::read(&backup).await?, b"not a sqlite database");
        audit
            .record_session_start("session-id", "agent-id", "agent", "Agent", "test", "1")
            .await?;
        assert!(audit.get_task("session-id").await?.is_some());
        Ok(())
    }

    #[tokio::test]
    async fn double_open_keeps_one_baseline_record() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let path = dir.path().join("audit.sqlite");
        let first = AuditDb::open(&path).await?;
        first.0.close().await?;

        let second = AuditDb::open(&path).await?;
        let migrations = second
            .connection()
            .query_all(Statement::from_string(
                DbBackend::Sqlite,
                "SELECT version FROM seaql_migrations".to_string(),
            ))
            .await?;
        assert_eq!(migrations.len(), 4);
        Ok(())
    }

    fn sqlite_options(path: &Path) -> SqliteConnectOptions {
        SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true)
    }
}
