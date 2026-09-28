//! At most one feedback invitation per installation, ever.
//!
//! The frequency rule lives in the schema rather than in code: `install_id` is the primary
//! key of `feedback_invite`, so a second invitation cannot be recorded and therefore cannot
//! be offered. Two cockpit tabs opening at once, a retried request and a restart mid-flight
//! all converge on one row without anyone reasoning about a race.
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

/// What the reader did with the invitation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InviteOutcome {
    /// The card reached the screen. This is what spends the installation's invitation.
    Shown,
    /// The booking link was opened.
    Clicked,
    /// Declined, by either control.
    Dismissed,
}

impl InviteOutcome {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Shown => "shown",
            Self::Clicked => "clicked",
            Self::Dismissed => "dismissed",
        }
    }

    /// How strong a claim this outcome makes about what the reader did.
    ///
    /// A recorded outcome may only ever be raised, never lowered, which is what keeps the
    /// funnel honest when the same installation reports more than once.
    ///
    /// `Clicked` outranks `Dismissed` deliberately. Someone who books a call and then
    /// closes the card has not declined, they have accepted and tidied up, so letting the
    /// dismissal land last would record the opposite of what happened. `Shown` ranks
    /// lowest, so an impression arriving late from a second tab cannot erase either.
    fn rank(self) -> i64 {
        match self {
            Self::Shown => 0,
            Self::Dismissed => 1,
            Self::Clicked => 2,
        }
    }

    /// Whether this is the reader acting, rather than the card merely appearing.
    fn is_response(self) -> bool {
        !matches!(self, Self::Shown)
    }
}

impl FeedbackInviteRepository {
    #[must_use]
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    /// Records `outcome` for this installation, spending its single invitation if it has
    /// not been spent already.
    ///
    /// A recorded outcome is only ever raised, never lowered, because arrival order says
    /// nothing about what the reader did. A second tab reporting `shown` after a click, a
    /// dismissal that follows a booking, and a retried `clicked` that lands after a later
    /// dismissal all arrive out of order and must not undo the stronger claim. See
    /// [`InviteOutcome::rank`].
    pub async fn record(
        &self,
        install_id: &str,
        outcome: InviteOutcome,
        now_ms: i64,
    ) -> AppResult<()> {
        let connection = self.db.connection();
        let dismissed_at = (outcome == InviteOutcome::Dismissed).then_some(now_ms);
        connection
            .execute(Statement::from_sql_and_values(
                DatabaseBackend::Sqlite,
                "INSERT INTO feedback_invite \
                 (install_id, shown_at_ms, outcome, settled_at_ms, dismissed_at_ms) \
                 VALUES (?, ?, ?, ?, ?) ON CONFLICT(install_id) DO NOTHING",
                [
                    Value::from(install_id.to_owned()),
                    Value::from(now_ms),
                    Value::from(outcome.as_str().to_owned()),
                    Value::from(outcome.is_response().then_some(now_ms)),
                    Value::from(dismissed_at),
                ],
            ))
            .await?;

        // One statement, so the funnel outcome and the dismissal gate cannot end up
        // disagreeing. Written as two, a crash in between could leave `outcome` saying
        // dismissed while the gate stayed null, and the card would come back after a
        // restart with no migration left to repair it.
        //
        // Each column carries its own guard. The outcome only ever rises, by rank. The gate
        // is only ever set, never moved or cleared, so the first dismissal is the one that
        // stands and a later click cannot reopen it.
        connection
            .execute(Statement::from_sql_and_values(
                DatabaseBackend::Sqlite,
                "UPDATE feedback_invite SET \
                   outcome = CASE WHEN ? > (CASE outcome \
                     WHEN 'clicked' THEN 2 WHEN 'dismissed' THEN 1 ELSE 0 END) \
                     THEN ? ELSE outcome END, \
                   settled_at_ms = CASE WHEN ? > (CASE outcome \
                     WHEN 'clicked' THEN 2 WHEN 'dismissed' THEN 1 ELSE 0 END) \
                     THEN ? ELSE settled_at_ms END, \
                   dismissed_at_ms = COALESCE(dismissed_at_ms, ?) \
                 WHERE install_id = ?",
                [
                    Value::from(outcome.rank()),
                    Value::from(outcome.as_str().to_owned()),
                    Value::from(outcome.rank()),
                    Value::from(outcome.is_response().then_some(now_ms)),
                    Value::from(dismissed_at),
                    Value::from(install_id.to_owned()),
                ],
            ))
            .await?;
        Ok(())
    }

    /// Whether this installation has already been offered an invitation.
    ///
    /// Only says the card has been on screen before. It does not gate anything: the card
    /// keeps appearing until the reader dismisses it, so use [`Self::has_dismissed`] for
    /// that question.
    pub async fn already_invited(&self, install_id: &str) -> AppResult<bool> {
        Ok(self.outcome_of(install_id).await?.is_some())
    }

    /// Whether the reader has asked to stop seeing the card. Final once true.
    pub async fn has_dismissed(&self, install_id: &str) -> AppResult<bool> {
        use crate::db::entities::prelude::FeedbackInvite;
        use sea_orm::EntityTrait;

        Ok(FeedbackInvite::find_by_id(install_id.to_owned())
            .one(self.db.connection())
            .await?
            .is_some_and(|row| row.dismissed_at_ms.is_some()))
    }

    /// The recorded outcome for this installation, if it has one.
    pub async fn outcome_of(&self, install_id: &str) -> AppResult<Option<String>> {
        use crate::db::entities::prelude::FeedbackInvite;
        use sea_orm::EntityTrait;

        Ok(FeedbackInvite::find_by_id(install_id.to_owned())
            .one(self.db.connection())
            .await?
            .map(|row| row.outcome))
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
    async fn an_impression_is_recorded_without_ending_the_offer() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let repo = repository(&dir).await?;

        assert!(!repo.already_invited("install-a").await?);
        repo.record("install-a", InviteOutcome::Shown, 1_000)
            .await?;
        assert!(repo.already_invited("install-a").await?);
        assert_eq!(
            repo.outcome_of("install-a").await?.as_deref(),
            Some("shown")
        );
        assert!(
            !repo.has_dismissed("install-a").await?,
            "an impression is not the reader asking it to stop"
        );
        Ok(())
    }

    /// Only a dismissal shuts the gate, and nothing reopens it. A click afterwards raises
    /// the funnel outcome because it is the stronger claim about what the reader did, and
    /// must still leave the gate shut.
    #[tokio::test]
    async fn only_a_dismissal_shuts_the_gate_and_nothing_reopens_it() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let repo = repository(&dir).await?;

        repo.record("install-a", InviteOutcome::Shown, 1_000)
            .await?;
        repo.record("install-a", InviteOutcome::Clicked, 2_000)
            .await?;
        assert!(!repo.has_dismissed("install-a").await?);

        repo.record("install-a", InviteOutcome::Dismissed, 3_000)
            .await?;
        assert!(repo.has_dismissed("install-a").await?);

        repo.record("install-a", InviteOutcome::Clicked, 4_000)
            .await?;
        assert!(
            repo.has_dismissed("install-a").await?,
            "a later click must not undo the dismissal"
        );
        assert_eq!(
            repo.outcome_of("install-a").await?.as_deref(),
            Some("clicked"),
            "while the funnel still keeps the stronger claim"
        );
        Ok(())
    }

    /// The gate keeps the first dismissal's time, not a later one's.
    #[tokio::test]
    async fn a_second_dismissal_does_not_move_the_gate() -> anyhow::Result<()> {
        use crate::db::entities::prelude::FeedbackInvite;
        use sea_orm::EntityTrait;

        let dir = tempdir()?;
        let repo = repository(&dir).await?;
        repo.record("install-a", InviteOutcome::Dismissed, 1_000)
            .await?;
        repo.record("install-a", InviteOutcome::Dismissed, 9_000)
            .await?;

        let Some(row) = FeedbackInvite::find_by_id("install-a".to_owned())
            .one(repo.db.connection())
            .await?
        else {
            panic!("the dismissal row is missing");
        };
        assert_eq!(row.dismissed_at_ms, Some(1_000));
        Ok(())
    }

    #[tokio::test]
    async fn installations_do_not_share_an_invitation() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let repo = repository(&dir).await?;

        repo.record("install-a", InviteOutcome::Shown, 1).await?;
        assert!(repo.already_invited("install-a").await?);
        assert!(!repo.already_invited("install-b").await?);
        Ok(())
    }

    #[tokio::test]
    async fn a_response_replaces_the_impression_it_followed() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let repo = repository(&dir).await?;

        repo.record("install-a", InviteOutcome::Shown, 1_000)
            .await?;
        repo.record("install-a", InviteOutcome::Clicked, 2_000)
            .await?;
        assert_eq!(
            repo.outcome_of("install-a").await?.as_deref(),
            Some("clicked")
        );
        Ok(())
    }

    /// Booking and then closing the card is not declining. The reader accepted and tidied
    /// up, so the record has to keep the click; this needs no race at all, it is what a
    /// happy user does.
    #[tokio::test]
    async fn tidying_the_card_away_after_booking_keeps_the_click() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let repo = repository(&dir).await?;

        repo.record("install-a", InviteOutcome::Shown, 1_000)
            .await?;
        repo.record("install-a", InviteOutcome::Clicked, 2_000)
            .await?;
        repo.record("install-a", InviteOutcome::Dismissed, 3_000)
            .await?;
        assert_eq!(
            repo.outcome_of("install-a").await?.as_deref(),
            Some("clicked")
        );
        Ok(())
    }

    /// A retried click can land after a dismissal the reader made while it was in flight.
    /// Arrival order says nothing about what they did, so the stronger claim still stands.
    #[tokio::test]
    async fn a_late_click_retry_outranks_a_dismissal_it_arrives_after() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let repo = repository(&dir).await?;

        repo.record("install-a", InviteOutcome::Shown, 1_000)
            .await?;
        repo.record("install-a", InviteOutcome::Dismissed, 2_000)
            .await?;
        repo.record("install-a", InviteOutcome::Clicked, 1_500)
            .await?;
        assert_eq!(
            repo.outcome_of("install-a").await?.as_deref(),
            Some("clicked")
        );
        Ok(())
    }

    /// Raising still works: a dismissal is a stronger claim than a bare impression.
    #[tokio::test]
    async fn a_dismissal_still_replaces_an_impression() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let repo = repository(&dir).await?;

        repo.record("install-a", InviteOutcome::Shown, 1_000)
            .await?;
        repo.record("install-a", InviteOutcome::Dismissed, 2_000)
            .await?;
        assert_eq!(
            repo.outcome_of("install-a").await?.as_deref(),
            Some("dismissed")
        );
        Ok(())
    }

    /// Two cockpit tabs, the second one rendering after the reader clicked in the first.
    /// The late impression must not erase what the reader actually did.
    #[tokio::test]
    async fn a_late_impression_does_not_overwrite_a_response() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let repo = repository(&dir).await?;

        repo.record("install-a", InviteOutcome::Shown, 1_000)
            .await?;
        repo.record("install-a", InviteOutcome::Clicked, 2_000)
            .await?;
        repo.record("install-a", InviteOutcome::Shown, 3_000)
            .await?;
        assert_eq!(
            repo.outcome_of("install-a").await?.as_deref(),
            Some("clicked")
        );
        Ok(())
    }

    /// A response with no impression before it is still recorded, so a client that skipped
    /// the impression still has its answer honoured.
    #[tokio::test]
    async fn a_response_without_an_impression_is_still_recorded() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let repo = repository(&dir).await?;

        repo.record("install-a", InviteOutcome::Dismissed, 1_000)
            .await?;
        assert!(repo.already_invited("install-a").await?);
        assert_eq!(
            repo.outcome_of("install-a").await?.as_deref(),
            Some("dismissed")
        );
        Ok(())
    }

    /// The row is what makes the rule true across restarts: it outlives the process that
    /// wrote it, so a server that comes back up cannot re-invite.
    #[tokio::test]
    async fn an_invitation_survives_reopening_the_database() -> anyhow::Result<()> {
        let dir = tempdir()?;
        {
            let repo = repository(&dir).await?;
            repo.record("install-a", InviteOutcome::Dismissed, 1)
                .await?;
        }
        let reopened = repository(&dir).await?;
        assert!(reopened.already_invited("install-a").await?);
        Ok(())
    }

    #[tokio::test]
    async fn concurrent_impressions_write_one_row() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let repo = repository(&dir).await?;

        let racers = (0..8).map(|index| {
            let repo = repo.clone();
            tokio::spawn(async move {
                repo.record("install-a", InviteOutcome::Shown, 1_000 + index)
                    .await
            })
        });
        for racer in racers.collect::<Vec<_>>() {
            racer.await??;
        }

        let rows: i64 = {
            use sea_orm::EntityTrait;
            i64::try_from(
                crate::db::entities::prelude::FeedbackInvite::find()
                    .all(repo.db.connection())
                    .await?
                    .len(),
            )?
        };
        assert_eq!(rows, 1, "eight parallel impressions wrote {rows} rows");
        // The first writer's timestamp is the one that stands: the invitation was spent
        // then, not by whichever request happened to finish last.
        assert_eq!(
            repo.outcome_of("install-a").await?.as_deref(),
            Some("shown")
        );
        Ok(())
    }
}
