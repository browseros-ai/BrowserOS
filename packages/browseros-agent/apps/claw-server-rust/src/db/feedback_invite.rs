//! At most one feedback invitation per installation, ever.
//!
//! The frequency rule lives in the schema rather than in code: `install_id` is the primary
//! key of `feedback_invite`, so a second invitation cannot be recorded and therefore cannot
//! be emitted. Concurrent agents finishing a task at the same moment, a retried dispatch and
//! a restart mid-flight all converge on one row without anyone reasoning about a race.
//!
//! This is deliberately weaker machinery than [`crate::db::run_error_budget`], which caps a
//! rolling daily count and earns its conditional claim. Here the rule is simpler, so the
//! mechanism is too.

use crate::{db::Database, error::AppResult};
use sea_orm::{ConnectionTrait, DatabaseBackend, Statement, Value};

#[derive(Clone)]
pub struct FeedbackInviteRepository {
    db: Database,
}

/// Whether this call is the one that gets to emit the invitation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InviteClaim {
    /// Won: no invitation had been recorded for this installation, and now one has.
    Granted,
    /// Lost: this installation was already invited, by an earlier call or a parallel one.
    AlreadyInvited,
}

impl FeedbackInviteRepository {
    #[must_use]
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    /// Records an invitation for `install_id` if it has never had one.
    ///
    /// Call this **before** appending the invitation to a result, never after. An agent
    /// that silently drops the line still consumes the invitation, which is the safe
    /// direction: the alternative risks inviting the same person repeatedly because the
    /// server cannot see what the agent chose to render.
    pub async fn claim(
        &self,
        install_id: &str,
        session_id: &str,
        agent: Option<&str>,
        now_ms: i64,
    ) -> AppResult<InviteClaim> {
        let claimed = self
            .db
            .connection()
            .execute(Statement::from_sql_and_values(
                DatabaseBackend::Sqlite,
                "INSERT INTO feedback_invite (install_id, emitted_at_ms, session_id, agent) \
                 VALUES (?, ?, ?, ?) ON CONFLICT(install_id) DO NOTHING",
                [
                    Value::from(install_id.to_owned()),
                    Value::from(now_ms),
                    Value::from(session_id.to_owned()),
                    Value::from(agent.map(str::to_owned)),
                ],
            ))
            .await?
            .rows_affected();

        Ok(if claimed > 0 {
            InviteClaim::Granted
        } else {
            InviteClaim::AlreadyInvited
        })
    }

    /// Whether this installation has already been invited, without claiming.
    pub async fn already_invited(&self, install_id: &str) -> AppResult<bool> {
        use crate::db::entities::prelude::FeedbackInvite;
        use sea_orm::EntityTrait;

        Ok(FeedbackInvite::find_by_id(install_id.to_owned())
            .one(self.db.connection())
            .await?
            .is_some())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{DATABASE_FILENAME, Database};
    use tempfile::tempdir;

    async fn repository(dir: &tempfile::TempDir) -> anyhow::Result<FeedbackInviteRepository> {
        Ok(FeedbackInviteRepository::new(
            Database::open(dir.path().join(DATABASE_FILENAME)).await?,
        ))
    }

    #[tokio::test]
    async fn the_first_claim_wins_and_every_later_one_loses() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let repo = repository(&dir).await?;

        assert_eq!(
            repo.claim("install-a", "session-1", Some("claude-code"), 1_000)
                .await?,
            InviteClaim::Granted
        );
        // A different session, a different agent, much later: still the same installation.
        assert_eq!(
            repo.claim("install-a", "session-2", Some("codex"), 9_999_999)
                .await?,
            InviteClaim::AlreadyInvited
        );
        assert!(repo.already_invited("install-a").await?);
        Ok(())
    }

    #[tokio::test]
    async fn installations_do_not_share_a_budget() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let repo = repository(&dir).await?;

        assert_eq!(
            repo.claim("install-a", "s", None, 1).await?,
            InviteClaim::Granted
        );
        assert_eq!(
            repo.claim("install-b", "s", None, 1).await?,
            InviteClaim::Granted
        );
        assert!(!repo.already_invited("install-c").await?);
        Ok(())
    }

    /// The claim is what makes the rule true across restarts: the row outlives the process
    /// that wrote it, so a server that comes back up cannot re-invite.
    #[tokio::test]
    async fn a_claim_survives_reopening_the_database() -> anyhow::Result<()> {
        let dir = tempdir()?;
        {
            let repo = repository(&dir).await?;
            assert_eq!(
                repo.claim("install-a", "s", None, 1).await?,
                InviteClaim::Granted
            );
        }
        let reopened = repository(&dir).await?;
        assert_eq!(
            reopened.claim("install-a", "s", None, 2).await?,
            InviteClaim::AlreadyInvited
        );
        Ok(())
    }

    #[tokio::test]
    async fn concurrent_claims_grant_exactly_one() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let repo = repository(&dir).await?;

        let racers = (0..8).map(|index| {
            let repo = repo.clone();
            tokio::spawn(async move {
                repo.claim("install-a", &format!("session-{index}"), None, 1)
                    .await
            })
        });
        let mut granted = 0;
        for racer in racers.collect::<Vec<_>>() {
            if racer.await?? == InviteClaim::Granted {
                granted += 1;
            }
        }
        assert_eq!(granted, 1, "eight parallel claims granted {granted}");
        Ok(())
    }
}
