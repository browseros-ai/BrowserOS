use crate::{db::Database, error::AppResult};
use sea_orm::{ConnectionTrait, DatabaseBackend, Statement, Value};

/// How many run failures one install may forward in a UTC day.
pub const DAILY_RUN_ERROR_CAP: i64 = 10;

/// Outcome of asking for room in today's budget.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BudgetDecision {
    /// Room was reserved. `used` is this claim's position, 1 through the cap.
    Allowed { used: i64 },
    /// Already at the cap. The suppressed counter was incremented instead.
    Suppressed { suppressed: i64 },
}

/// Formats an epoch in milliseconds as a UTC `YYYY-MM-DD` key.
///
/// A calendar day rather than a rolling window: a counter is enough, where a rolling
/// window would need to retain a timestamp per report.
#[must_use]
pub fn utc_day(epoch_ms: i64) -> String {
    let seconds = epoch_ms.div_euclid(1_000);
    let date = time::OffsetDateTime::from_unix_timestamp(seconds)
        .unwrap_or(time::OffsetDateTime::UNIX_EPOCH)
        .date();
    format!(
        "{:04}-{:02}-{:02}",
        date.year(),
        u8::from(date.month()),
        date.day()
    )
}

/// Per-install daily budget for outbound run failure reports.
#[derive(Clone)]
pub struct RunErrorBudgetRepository {
    db: Database,
}

impl RunErrorBudgetRepository {
    #[must_use]
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    /// Reserves one report against today's budget.
    ///
    /// The reservation is a single conditional upsert rather than a read then a write, so
    /// two dispatches failing at the same moment cannot both see nine and both send. The
    /// `WHERE sent < cap` rides on the conflict branch, so the update simply does not fire
    /// once the cap is reached.
    pub async fn claim(&self, epoch_ms: i64, cap: i64) -> AppResult<BudgetDecision> {
        let day = utc_day(epoch_ms);
        let connection = self.db.connection();

        let claimed = connection
            .execute(Statement::from_sql_and_values(
                DatabaseBackend::Sqlite,
                // The SELECT ... WHERE guards the first claim of a day, which has no
                // conflict to hang a condition on. Without it a zero or exhausted cap
                // would still let one report through each day.
                "INSERT INTO run_error_budget (day, sent, suppressed) \
                 SELECT ?, 1, 0 WHERE ? > 0 \
                 ON CONFLICT(day) DO UPDATE SET sent = sent + 1 WHERE sent < ?",
                [Value::from(day.clone()), Value::from(cap), Value::from(cap)],
            ))
            .await?
            .rows_affected();

        if claimed > 0 {
            // Yesterday's row has no further use once a new day has opened one.
            connection
                .execute(Statement::from_sql_and_values(
                    DatabaseBackend::Sqlite,
                    "DELETE FROM run_error_budget WHERE day <> ?",
                    [Value::from(day.clone())],
                ))
                .await?;
            return Ok(BudgetDecision::Allowed {
                used: self.counts(&day).await?.0,
            });
        }

        // Upsert rather than update: with a zero cap the day has no row to update.
        connection
            .execute(Statement::from_sql_and_values(
                DatabaseBackend::Sqlite,
                "INSERT INTO run_error_budget (day, sent, suppressed) VALUES (?, 0, 1) \
                 ON CONFLICT(day) DO UPDATE SET suppressed = suppressed + 1",
                [Value::from(day.clone())],
            ))
            .await?;
        Ok(BudgetDecision::Suppressed {
            suppressed: self.counts(&day).await?.1,
        })
    }

    /// `(sent, suppressed)` for a day, zero when the day has no row.
    pub async fn counts(&self, day: &str) -> AppResult<(i64, i64)> {
        let row = self
            .db
            .connection()
            .query_one(Statement::from_sql_and_values(
                DatabaseBackend::Sqlite,
                "SELECT sent, suppressed FROM run_error_budget WHERE day = ?",
                [Value::from(day.to_string())],
            ))
            .await?;
        match row {
            Some(row) => Ok((row.try_get("", "sent")?, row.try_get("", "suppressed")?)),
            None => Ok((0, 0)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    const DAY_ONE: i64 = 1_789_560_000_000; // 2026-09-16T12:00:00Z, deliberately midday
    //   so a one-hour step cannot silently cross into the next day
    const DAY_TWO: i64 = DAY_ONE + 86_400_000;

    async fn repository(dir: &std::path::Path) -> anyhow::Result<RunErrorBudgetRepository> {
        let db = Database::open(dir.join("browserclaw.sqlite")).await?;
        Ok(RunErrorBudgetRepository::new(db))
    }

    #[tokio::test]
    async fn the_day_key_is_utc_and_stable_within_a_day() {
        let morning = utc_day(DAY_ONE);
        let later = utc_day(DAY_ONE + 3_600_000);
        assert_eq!(morning, later);
        assert_eq!(morning.len(), "YYYY-MM-DD".len());
        assert_ne!(morning, utc_day(DAY_TWO));
    }

    #[tokio::test]
    async fn claims_are_allowed_up_to_the_cap_then_suppressed() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let repo = repository(dir.path()).await?;

        for expected in 1..=DAILY_RUN_ERROR_CAP {
            assert_eq!(
                repo.claim(DAY_ONE, DAILY_RUN_ERROR_CAP).await?,
                BudgetDecision::Allowed { used: expected }
            );
        }
        assert_eq!(
            repo.claim(DAY_ONE, DAILY_RUN_ERROR_CAP).await?,
            BudgetDecision::Suppressed { suppressed: 1 }
        );
        assert_eq!(
            repo.claim(DAY_ONE, DAILY_RUN_ERROR_CAP).await?,
            BudgetDecision::Suppressed { suppressed: 2 }
        );

        let (sent, suppressed) = repo.counts(&utc_day(DAY_ONE)).await?;
        assert_eq!((sent, suppressed), (DAILY_RUN_ERROR_CAP, 2));
        Ok(())
    }

    #[tokio::test]
    async fn a_new_day_resets_the_budget_and_prunes_the_old_row() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let repo = repository(dir.path()).await?;
        for _ in 0..DAILY_RUN_ERROR_CAP {
            repo.claim(DAY_ONE, DAILY_RUN_ERROR_CAP).await?;
        }
        assert!(matches!(
            repo.claim(DAY_ONE, DAILY_RUN_ERROR_CAP).await?,
            BudgetDecision::Suppressed { .. }
        ));

        assert_eq!(
            repo.claim(DAY_TWO, DAILY_RUN_ERROR_CAP).await?,
            BudgetDecision::Allowed { used: 1 }
        );
        assert_eq!(repo.counts(&utc_day(DAY_ONE)).await?, (0, 0));
        Ok(())
    }

    #[tokio::test]
    async fn the_budget_survives_a_restart() -> anyhow::Result<()> {
        let dir = tempdir()?;
        {
            let repo = repository(dir.path()).await?;
            for _ in 0..DAILY_RUN_ERROR_CAP {
                repo.claim(DAY_ONE, DAILY_RUN_ERROR_CAP).await?;
            }
        }
        let reopened = repository(dir.path()).await?;
        assert!(
            matches!(
                reopened.claim(DAY_ONE, DAILY_RUN_ERROR_CAP).await?,
                BudgetDecision::Suppressed { .. }
            ),
            "a restart must not hand back a fresh budget"
        );
        Ok(())
    }

    /// The reason the claim is one conditional upsert rather than a read then a write.
    /// Twenty dispatches failing at once must not all see room.
    #[tokio::test]
    async fn concurrent_claims_never_exceed_the_cap() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let repo = repository(dir.path()).await?;

        let mut handles = Vec::new();
        for _ in 0..20 {
            let repo = repo.clone();
            handles.push(tokio::spawn(async move {
                repo.claim(DAY_ONE, DAILY_RUN_ERROR_CAP).await
            }));
        }
        let mut allowed = 0;
        for handle in handles {
            if matches!(handle.await??, BudgetDecision::Allowed { .. }) {
                allowed += 1;
            }
        }
        assert_eq!(allowed, DAILY_RUN_ERROR_CAP, "cap leaked under concurrency");

        let (sent, suppressed) = repo.counts(&utc_day(DAY_ONE)).await?;
        assert_eq!(sent, DAILY_RUN_ERROR_CAP);
        assert_eq!(suppressed, 20 - DAILY_RUN_ERROR_CAP);
        Ok(())
    }

    #[tokio::test]
    async fn a_zero_cap_reports_nothing() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let repo = repository(dir.path()).await?;
        assert_eq!(
            repo.claim(DAY_ONE, 0).await?,
            BudgetDecision::Suppressed { suppressed: 1 }
        );
        Ok(())
    }
}
