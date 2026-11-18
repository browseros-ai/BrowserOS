use crate::{db::Database, error::AppResult};
use sea_orm::{ConnectionTrait, DatabaseBackend, Statement, Value};

/// Calls one tool made from one client on one UTC day.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolCallCount {
    pub tool_name: String,
    pub client_name: String,
    pub calls: i64,
    pub error_calls: i64,
}

/// Tool call counts per UTC day, held until the day is reported.
#[derive(Clone)]
pub struct ToolCallsDailyRepository {
    db: Database,
}

impl ToolCallsDailyRepository {
    #[must_use]
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    /// Adds one call to the day's count for this tool and client.
    pub async fn record(
        &self,
        day: &str,
        tool_name: &str,
        client_name: &str,
        failed: bool,
    ) -> AppResult<()> {
        let error_calls = i64::from(failed);
        self.db
            .connection()
            .execute(Statement::from_sql_and_values(
                DatabaseBackend::Sqlite,
                "INSERT INTO tool_calls_daily (day, tool_name, client_name, calls, error_calls) \
                 VALUES (?, ?, ?, 1, ?) \
                 ON CONFLICT(day, tool_name, client_name) DO UPDATE SET \
                 calls = calls + 1, error_calls = error_calls + excluded.error_calls",
                [
                    Value::from(day.to_string()),
                    Value::from(tool_name.to_string()),
                    Value::from(client_name.to_string()),
                    Value::from(error_calls),
                ],
            ))
            .await?;
        Ok(())
    }

    /// Days before `day` that still have counts, oldest first.
    pub async fn days_before(&self, day: &str) -> AppResult<Vec<String>> {
        let rows = self
            .db
            .connection()
            .query_all(Statement::from_sql_and_values(
                DatabaseBackend::Sqlite,
                "SELECT DISTINCT day FROM tool_calls_daily WHERE day < ? ORDER BY day",
                [Value::from(day.to_string())],
            ))
            .await?;
        rows.iter().map(|row| Ok(row.try_get("", "day")?)).collect()
    }

    pub async fn counts(&self, day: &str) -> AppResult<Vec<ToolCallCount>> {
        let rows = self
            .db
            .connection()
            .query_all(Statement::from_sql_and_values(
                DatabaseBackend::Sqlite,
                "SELECT tool_name, client_name, calls, error_calls FROM tool_calls_daily \
                 WHERE day = ? ORDER BY tool_name, client_name",
                [Value::from(day.to_string())],
            ))
            .await?;
        rows.iter()
            .map(|row| {
                Ok(ToolCallCount {
                    tool_name: row.try_get("", "tool_name")?,
                    client_name: row.try_get("", "client_name")?,
                    calls: row.try_get("", "calls")?,
                    error_calls: row.try_get("", "error_calls")?,
                })
            })
            .collect()
    }

    pub async fn delete_day(&self, day: &str) -> AppResult<()> {
        self.db
            .connection()
            .execute(Statement::from_sql_and_values(
                DatabaseBackend::Sqlite,
                "DELETE FROM tool_calls_daily WHERE day = ?",
                [Value::from(day.to_string())],
            ))
            .await?;
        Ok(())
    }

    pub async fn delete_all(&self) -> AppResult<()> {
        self.db
            .connection()
            .execute_unprepared("DELETE FROM tool_calls_daily")
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    async fn repository(dir: &std::path::Path) -> anyhow::Result<ToolCallsDailyRepository> {
        let db = Database::open(dir.join("browserclaw.sqlite")).await?;
        Ok(ToolCallsDailyRepository::new(db))
    }

    #[tokio::test]
    async fn calls_add_up_per_day_tool_and_client_and_survive_a_restart() -> anyhow::Result<()> {
        let dir = tempdir()?;
        {
            let repo = repository(dir.path()).await?;
            repo.record("2026-11-10", "navigate", "Codex", false)
                .await?;
            repo.record("2026-11-10", "navigate", "Codex", true).await?;
            repo.record("2026-11-10", "tabs", "Claude Code", false)
                .await?;
            repo.record("2026-11-11", "tabs", "Codex", false).await?;
        }
        let repo = repository(dir.path()).await?;
        assert_eq!(
            repo.counts("2026-11-10").await?,
            vec![
                ToolCallCount {
                    tool_name: "navigate".to_string(),
                    client_name: "Codex".to_string(),
                    calls: 2,
                    error_calls: 1,
                },
                ToolCallCount {
                    tool_name: "tabs".to_string(),
                    client_name: "Claude Code".to_string(),
                    calls: 1,
                    error_calls: 0,
                },
            ]
        );
        assert_eq!(repo.days_before("2026-11-11").await?, vec!["2026-11-10"]);
        assert_eq!(
            repo.days_before("2026-11-12").await?,
            vec!["2026-11-10", "2026-11-11"]
        );

        repo.delete_day("2026-11-10").await?;
        assert!(repo.counts("2026-11-10").await?.is_empty());
        assert_eq!(repo.days_before("2026-11-12").await?, vec!["2026-11-11"]);
        repo.delete_all().await?;
        assert!(repo.days_before("2026-11-12").await?.is_empty());
        Ok(())
    }
}
