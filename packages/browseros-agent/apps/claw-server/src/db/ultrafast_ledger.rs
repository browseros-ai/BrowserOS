//! What ultrafast did in each task, kept in `ultrafast_sessions` after the task's audit rows are
//! deleted, so the Home meter and the savings totals never shrink when history ages out. Rows are
//! written from the audit history while it still exists: on every savings read and just before
//! the retention sweep deletes a task.

use crate::{
    db::{
        Database, audit_log::UltrafastSession, pursue_runs::PursueOutcome,
        ultrafast_runs::UltrafastRunStats,
    },
    error::AppResult,
};
use sea_orm::{ConnectionTrait, DatabaseBackend, Statement, Value};

/// One task's ultrafast numbers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UltrafastRecord {
    pub session_id: String,
    pub totals: UltrafastRunStats,
    pub fastest_run_ms: Option<i64>,
    /// neo's browser steps during pursue and every browser step in the task.
    pub pursue_step_share: Option<(i64, i64)>,
    pub first_run_at: i64,
    pub last_run_at: i64,
    pub site: Option<String>,
    /// How the task's pursue ended; `None` for a task without one, such as a saved skill.
    pub pursue_outcome: Option<PursueOutcome>,
}

impl From<&UltrafastSession> for UltrafastRecord {
    fn from(session: &UltrafastSession) -> Self {
        Self {
            session_id: session.session_id.clone(),
            totals: session.totals,
            fastest_run_ms: session
                .run_durations
                .iter()
                .copied()
                .filter(|duration| *duration > 0)
                .min(),
            pursue_step_share: session.pursue_step_share,
            first_run_at: session.first_run_at,
            last_run_at: session.last_run_at,
            site: session.site.clone(),
            pursue_outcome: session.pursue.as_ref().map(|run| run.outcome),
        }
    }
}

fn outcome_text(outcome: PursueOutcome) -> &'static str {
    match outcome {
        PursueOutcome::Completed => "completed",
        PursueOutcome::HandedBack => "handed_back",
    }
}

fn outcome_from(text: &str) -> Option<PursueOutcome> {
    match text {
        "completed" => Some(PursueOutcome::Completed),
        "handed_back" => Some(PursueOutcome::HandedBack),
        _ => None,
    }
}

#[derive(Clone)]
pub struct UltrafastLedger {
    db: Database,
}

impl UltrafastLedger {
    #[must_use]
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    /// Saves each task's numbers. A task only gains rows until it is deleted whole, so a newer
    /// reading never undercounts; the larger totals and the wider run span are kept anyway.
    pub async fn record(&self, records: &[UltrafastRecord]) -> AppResult<()> {
        let connection = self.db.connection();
        for record in records {
            let (neo_steps, browser_steps) = record
                .pursue_step_share
                .map_or((None, None), |(neo, all)| (Some(neo), Some(all)));
            connection
                .execute(Statement::from_sql_and_values(
                    DatabaseBackend::Sqlite,
                    "INSERT INTO ultrafast_sessions (session_id, duration_ms, pages_read, \
                     tokens_saved, fastest_run_ms, neo_steps, browser_steps, first_run_at, \
                     last_run_at, site, pursue_outcome) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
                     ON CONFLICT(session_id) DO UPDATE SET \
                     duration_ms = max(duration_ms, excluded.duration_ms), \
                     pages_read = max(pages_read, excluded.pages_read), \
                     tokens_saved = max(tokens_saved, excluded.tokens_saved), \
                     fastest_run_ms = coalesce(min(fastest_run_ms, excluded.fastest_run_ms), \
                       fastest_run_ms, excluded.fastest_run_ms), \
                     neo_steps = coalesce(excluded.neo_steps, neo_steps), \
                     browser_steps = coalesce(excluded.browser_steps, browser_steps), \
                     first_run_at = min(first_run_at, excluded.first_run_at), \
                     last_run_at = max(last_run_at, excluded.last_run_at), \
                     site = coalesce(excluded.site, site), \
                     pursue_outcome = coalesce(excluded.pursue_outcome, pursue_outcome)",
                    [
                        Value::from(record.session_id.clone()),
                        Value::from(record.totals.duration_ms.max(0)),
                        Value::from(record.totals.pages_read.max(0)),
                        Value::from(record.totals.tokens_saved.max(0)),
                        Value::from(record.fastest_run_ms),
                        Value::from(neo_steps),
                        Value::from(browser_steps),
                        Value::from(record.first_run_at),
                        Value::from(record.last_run_at),
                        Value::from(record.site.clone()),
                        Value::from(record.pursue_outcome.map(outcome_text)),
                    ],
                ))
                .await?;
        }
        Ok(())
    }

    /// Every task ultrafast did work in, newest run first.
    pub async fn all(&self) -> AppResult<Vec<UltrafastRecord>> {
        let rows = self
            .db
            .connection()
            .query_all(Statement::from_string(
                DatabaseBackend::Sqlite,
                "SELECT session_id, duration_ms, pages_read, tokens_saved, fastest_run_ms, \
                 neo_steps, browser_steps, first_run_at, last_run_at, site, pursue_outcome \
                 FROM ultrafast_sessions \
                 ORDER BY last_run_at DESC, session_id DESC"
                    .to_string(),
            ))
            .await?;
        rows.into_iter()
            .map(|row| {
                let neo: Option<i64> = row.try_get("", "neo_steps")?;
                let all: Option<i64> = row.try_get("", "browser_steps")?;
                Ok(UltrafastRecord {
                    session_id: row.try_get("", "session_id")?,
                    totals: UltrafastRunStats {
                        duration_ms: row.try_get("", "duration_ms")?,
                        pages_read: row.try_get("", "pages_read")?,
                        tokens_saved: row.try_get("", "tokens_saved")?,
                    },
                    fastest_run_ms: row.try_get("", "fastest_run_ms")?,
                    pursue_step_share: neo.zip(all),
                    first_run_at: row.try_get("", "first_run_at")?,
                    last_run_at: row.try_get("", "last_run_at")?,
                    site: row.try_get("", "site")?,
                    pursue_outcome: row
                        .try_get::<Option<String>>("", "pursue_outcome")?
                        .as_deref()
                        .and_then(outcome_from),
                })
            })
            .collect::<Result<Vec<_>, sea_orm::DbErr>>()
            .map_err(Into::into)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(session_id: &str, tokens: i64, fastest: Option<i64>, last: i64) -> UltrafastRecord {
        UltrafastRecord {
            session_id: session_id.into(),
            totals: UltrafastRunStats {
                duration_ms: 6_000,
                pages_read: 4,
                tokens_saved: tokens,
            },
            fastest_run_ms: fastest,
            pursue_step_share: Some((3, 4)),
            first_run_at: last - 1_000,
            last_run_at: last,
            site: Some("google.com".into()),
            pursue_outcome: Some(PursueOutcome::HandedBack),
        }
    }

    #[tokio::test]
    async fn records_are_kept_newest_first_and_never_shrink() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let db = Database::open(dir.path().join("ledger.sqlite")).await?;
        let ledger = UltrafastLedger::new(db);
        ledger
            .record(&[
                record("a", 10_000, Some(6_000), 5_000),
                record("b", 2_000, None, 9_000),
            ])
            .await?;
        // A later reading with fewer tokens (history partly gone) and a faster run keeps the most.
        ledger
            .record(&[record("a", 4_000, Some(3_000), 5_000)])
            .await?;

        let rows = ledger.all().await?;
        assert_eq!(
            rows.iter()
                .map(|row| row.session_id.as_str())
                .collect::<Vec<_>>(),
            ["b", "a"]
        );
        assert_eq!(rows[1].totals.tokens_saved, 10_000);
        assert_eq!(rows[1].fastest_run_ms, Some(3_000));
        assert_eq!(rows[1].pursue_step_share, Some((3, 4)));
        assert_eq!(rows[0].fastest_run_ms, None);
        assert_eq!(rows[1].site.as_deref(), Some("google.com"));
        assert_eq!(rows[1].pursue_outcome, Some(PursueOutcome::HandedBack));
        Ok(())
    }
}
