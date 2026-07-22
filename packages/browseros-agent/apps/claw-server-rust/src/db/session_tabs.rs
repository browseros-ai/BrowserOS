//! Durable session-to-browser ownership windows. Asynchronous legacy-target and tab claim/release
//! mutations share one FIFO writer so their observed lifecycle order reaches SQLite unchanged.

use crate::{
    clock::now_epoch_ms,
    db::{
        Database,
        entities::{
            prelude::{SessionTabs, TabClaims},
            session_tabs, tab_claims,
        },
    },
    error::AppResult,
};
use sea_orm::{
    ActiveValue::{NotSet, Set},
    ColumnTrait, EntityTrait, QueryFilter, QueryOrder, TransactionTrait,
    sea_query::Expr,
};
use tokio::sync::{mpsc, oneshot};
use tracing::warn;

#[derive(Clone)]
pub struct SessionTabLedger {
    db: Database,
    claim_writes: mpsc::UnboundedSender<ClaimWrite>,
}

#[cfg(test)]
pub(crate) struct SessionTabSnapshot {
    pub id: i64,
    pub session_id: String,
    pub tab_id: i64,
    pub opened_target_id: Option<String>,
    pub claimed_at: i64,
    pub released_at: Option<i64>,
}

#[cfg(test)]
impl From<session_tabs::Model> for SessionTabSnapshot {
    fn from(row: session_tabs::Model) -> Self {
        Self {
            id: row.id,
            session_id: row.session_id,
            tab_id: row.tab_id,
            opened_target_id: row.opened_target_id,
            claimed_at: row.claimed_at,
            released_at: row.released_at,
        }
    }
}

#[derive(Debug)]
enum ClaimWrite {
    ClaimTarget {
        target_id: String,
        session_id: String,
        agent_id: String,
        claimed_at: i64,
    },
    ReleaseTargetForSession {
        target_id: String,
        session_id: String,
    },
    ReleaseSession {
        session_id: String,
        released_at: i64,
    },
    ReleaseTarget {
        target_id: String,
    },
    ClaimTab {
        tab_id: i64,
        opened_target_id: Option<String>,
        session_id: String,
        agent_id: String,
        claimed_at: i64,
    },
    InheritTab {
        opener_tab_id: i64,
        tab_id: i64,
        opened_target_id: String,
        claimed_at: i64,
    },
    ReleaseTabForSession {
        tab_id: i64,
        session_id: String,
        released_at: i64,
    },
    Flush(oneshot::Sender<()>),
}

impl SessionTabLedger {
    pub fn new(db: Database) -> Self {
        let (claim_writes, receiver) = mpsc::unbounded_channel();
        tokio::spawn(run_claim_writes(db.clone(), receiver));
        Self { db, claim_writes }
    }

    #[cfg(test)]
    pub(crate) fn connection(&self) -> &sea_orm::DatabaseConnection {
        self.db.connection()
    }

    #[cfg(test)]
    pub(crate) async fn all_legacy_claims_released_for_session(
        &self,
        session_id: &str,
    ) -> AppResult<bool> {
        Ok(TabClaims::find()
            .filter(tab_claims::Column::SessionId.eq(session_id))
            .all(self.db.connection())
            .await?
            .iter()
            .all(|claim| claim.released_at.is_some()))
    }

    #[cfg(test)]
    pub(crate) async fn first_session_tab(&self) -> AppResult<Option<SessionTabSnapshot>> {
        Ok(SessionTabs::find()
            .one(self.db.connection())
            .await?
            .map(SessionTabSnapshot::from))
    }

    #[cfg(test)]
    pub(crate) async fn session_tab_claimed_at(
        &self,
        claimed_at: i64,
    ) -> AppResult<Option<SessionTabSnapshot>> {
        Ok(SessionTabs::find()
            .filter(session_tabs::Column::ClaimedAt.eq(claimed_at))
            .one(self.db.connection())
            .await?
            .map(SessionTabSnapshot::from))
    }

    #[cfg(test)]
    pub(crate) async fn session_tab_by_id(&self, id: i64) -> AppResult<Option<SessionTabSnapshot>> {
        Ok(SessionTabs::find_by_id(id)
            .one(self.db.connection())
            .await?
            .map(SessionTabSnapshot::from))
    }

    /// Closes every open claim when CDP reports that its target was destroyed.
    pub async fn release_claims_for_target(&self, target_id: &str) -> AppResult<u64> {
        release_claims_for_target(self.db.connection(), target_id).await
    }

    /// Opens a claim window when a session begins driving a target.
    pub async fn claim_target_for_session(
        &self,
        target_id: &str,
        session_id: &str,
        agent_id: &str,
        claimed_at: i64,
    ) -> AppResult<i64> {
        claim_target_for_session(
            self.db.connection(),
            target_id,
            session_id,
            agent_id,
            claimed_at,
        )
        .await
    }

    /// Closes this session's open claim after it closes the target.
    pub async fn release_target_for_session(
        &self,
        target_id: &str,
        session_id: &str,
    ) -> AppResult<u64> {
        release_target_for_session(self.db.connection(), target_id, session_id).await
    }

    /// Closes every open claim when an MCP session ends.
    pub async fn release_claims_for_session(&self, session_id: &str) -> AppResult<u64> {
        release_claims_for_session(self.db.connection(), session_id, now_epoch_ms()).await
    }

    pub fn enqueue_claim_target_for_session(
        &self,
        target_id: String,
        session_id: String,
        agent_id: String,
        claimed_at: i64,
    ) {
        self.enqueue_claim_write(ClaimWrite::ClaimTarget {
            target_id,
            session_id,
            agent_id,
            claimed_at,
        });
    }

    pub fn enqueue_release_target_for_session(&self, target_id: String, session_id: String) {
        self.enqueue_claim_write(ClaimWrite::ReleaseTargetForSession {
            target_id,
            session_id,
        });
    }

    pub fn enqueue_release_claims_for_session(&self, session_id: String) {
        self.enqueue_claim_write(ClaimWrite::ReleaseSession {
            session_id,
            released_at: now_epoch_ms(),
        });
    }

    pub fn enqueue_release_claims_for_target(&self, target_id: String) {
        self.enqueue_claim_write(ClaimWrite::ReleaseTarget { target_id });
    }

    pub fn enqueue_claim_tab_for_session(
        &self,
        tab_id: i64,
        opened_target_id: Option<String>,
        session_id: String,
        agent_id: String,
        claimed_at: i64,
    ) {
        self.enqueue_claim_write(ClaimWrite::ClaimTab {
            tab_id,
            opened_target_id,
            session_id,
            agent_id,
            claimed_at,
        });
    }

    pub fn enqueue_inherit_tab_ownership(
        &self,
        opener_tab_id: i64,
        tab_id: i64,
        opened_target_id: String,
        claimed_at: i64,
    ) {
        self.enqueue_claim_write(ClaimWrite::InheritTab {
            opener_tab_id,
            tab_id,
            opened_target_id,
            claimed_at,
        });
    }

    pub fn enqueue_release_tab_for_session(&self, tab_id: i64, session_id: String) {
        self.enqueue_claim_write(ClaimWrite::ReleaseTabForSession {
            tab_id,
            session_id,
            released_at: now_epoch_ms(),
        });
    }

    /// A point-in-time FIFO barrier that waits for every mutation ordered ahead of its flush
    /// message. It neither closes the writer nor waits for later sends.
    pub async fn drain_writes(&self) {
        let (done, receiver) = oneshot::channel();
        if self.claim_writes.send(ClaimWrite::Flush(done)).is_ok() {
            let _ = receiver.await;
        }
    }

    fn enqueue_claim_write(&self, write: ClaimWrite) {
        if let Err(error) = self.claim_writes.send(write) {
            warn!(write = ?error.0, "claim write queue closed");
        }
    }

    /// Closes claims left open across an unclean server shutdown.
    pub async fn release_all_open(&self) -> AppResult<u64> {
        let target_result = TabClaims::update_many()
            .col_expr(tab_claims::Column::ReleasedAt, Expr::value(now_epoch_ms()))
            .filter(tab_claims::Column::ReleasedAt.is_null())
            .exec(self.db.connection())
            .await?;
        let tab_result = SessionTabs::update_many()
            .col_expr(
                session_tabs::Column::ReleasedAt,
                Expr::value(now_epoch_ms()),
            )
            .filter(session_tabs::Column::ReleasedAt.is_null())
            .exec(self.db.connection())
            .await?;
        Ok(target_result.rows_affected + tab_result.rows_affected)
    }

    /// Returns the durable browser-tab ownership windows that are still open.
    pub async fn list_open_session_tabs(
        &self,
        session_ids: &[String],
    ) -> AppResult<Vec<session_tabs::Model>> {
        if session_ids.is_empty() {
            return Ok(Vec::new());
        }
        Ok(SessionTabs::find()
            .filter(session_tabs::Column::SessionId.is_in(session_ids.iter().cloned()))
            .filter(session_tabs::Column::ReleasedAt.is_null())
            .order_by_asc(session_tabs::Column::SessionId)
            .order_by_asc(session_tabs::Column::TabId)
            .all(self.db.connection())
            .await?)
    }

    /// Returns current durable ownership for one session and Chrome tab.
    pub async fn open_session_tab(
        &self,
        session_id: &str,
        tab_id: i64,
    ) -> AppResult<Option<session_tabs::Model>> {
        Ok(SessionTabs::find()
            .filter(session_tabs::Column::SessionId.eq(session_id))
            .filter(session_tabs::Column::TabId.eq(tab_id))
            .filter(session_tabs::Column::ReleasedAt.is_null())
            .one(self.db.connection())
            .await?)
    }
}

async fn run_claim_writes(db: Database, mut receiver: mpsc::UnboundedReceiver<ClaimWrite>) {
    while let Some(write) = receiver.recv().await {
        let write = match write {
            ClaimWrite::Flush(done) => {
                let _ = done.send(());
                continue;
            }
            write => write,
        };
        let result = match &write {
            ClaimWrite::ClaimTarget {
                target_id,
                session_id,
                agent_id,
                claimed_at,
            } => claim_target_for_session(
                db.connection(),
                target_id,
                session_id,
                agent_id,
                *claimed_at,
            )
            .await
            .map(|_| ()),
            ClaimWrite::ReleaseTargetForSession {
                target_id,
                session_id,
            } => release_target_for_session(db.connection(), target_id, session_id)
                .await
                .map(|_| ()),
            ClaimWrite::ReleaseSession {
                session_id,
                released_at,
            } => release_claims_for_session(db.connection(), session_id, *released_at)
                .await
                .map(|_| ()),
            ClaimWrite::ReleaseTarget { target_id } => {
                release_claims_for_target(db.connection(), target_id)
                    .await
                    .map(|_| ())
            }
            ClaimWrite::ClaimTab {
                tab_id,
                opened_target_id,
                session_id,
                agent_id,
                claimed_at,
            } => claim_tab_for_session(
                db.connection(),
                *tab_id,
                opened_target_id.as_deref(),
                session_id,
                agent_id,
                *claimed_at,
            )
            .await
            .map(|_| ()),
            ClaimWrite::InheritTab {
                opener_tab_id,
                tab_id,
                opened_target_id,
                claimed_at,
            } => inherit_tab_ownership(
                db.connection(),
                *opener_tab_id,
                *tab_id,
                opened_target_id,
                *claimed_at,
            )
            .await
            .map(|_| ()),
            ClaimWrite::ReleaseTabForSession {
                tab_id,
                session_id,
                released_at,
            } => release_tab_for_session(db.connection(), *tab_id, session_id, *released_at)
                .await
                .map(|_| ()),
            ClaimWrite::Flush(_) => unreachable!(),
        };
        if let Err(error) = result {
            warn!(write = ?write, error = %error, "claim write failed");
        }
    }
}

async fn claim_target_for_session(
    db: &sea_orm::DatabaseConnection,
    target_id: &str,
    session_id: &str,
    agent_id: &str,
    claimed_at: i64,
) -> AppResult<i64> {
    let result = TabClaims::insert(tab_claims::ActiveModel {
        id: NotSet,
        target_id: Set(target_id.to_string()),
        session_id: Set(session_id.to_string()),
        agent_id: Set(agent_id.to_string()),
        claimed_at: Set(claimed_at),
        released_at: Set(None),
    })
    .exec(db)
    .await?;
    Ok(result.last_insert_id)
}

async fn release_target_for_session(
    db: &sea_orm::DatabaseConnection,
    target_id: &str,
    session_id: &str,
) -> AppResult<u64> {
    let result = TabClaims::update_many()
        .col_expr(tab_claims::Column::ReleasedAt, Expr::value(now_epoch_ms()))
        .filter(tab_claims::Column::TargetId.eq(target_id))
        .filter(tab_claims::Column::SessionId.eq(session_id))
        .filter(tab_claims::Column::ReleasedAt.is_null())
        .exec(db)
        .await?;
    Ok(result.rows_affected)
}

async fn release_claims_for_session(
    db: &sea_orm::DatabaseConnection,
    session_id: &str,
    released_at: i64,
) -> AppResult<u64> {
    let target_result = TabClaims::update_many()
        .col_expr(tab_claims::Column::ReleasedAt, Expr::value(released_at))
        .filter(tab_claims::Column::SessionId.eq(session_id))
        .filter(tab_claims::Column::ReleasedAt.is_null())
        .exec(db)
        .await?;
    let tab_result = SessionTabs::update_many()
        .col_expr(session_tabs::Column::ReleasedAt, Expr::value(released_at))
        .filter(session_tabs::Column::SessionId.eq(session_id))
        .filter(session_tabs::Column::ReleasedAt.is_null())
        .exec(db)
        .await?;
    Ok(target_result.rows_affected + tab_result.rows_affected)
}

async fn claim_tab_for_session(
    db: &sea_orm::DatabaseConnection,
    tab_id: i64,
    opened_target_id: Option<&str>,
    session_id: &str,
    agent_id: &str,
    claimed_at: i64,
) -> AppResult<i64> {
    let txn = db.begin().await?;
    let existing = SessionTabs::find()
        .filter(session_tabs::Column::TabId.eq(tab_id))
        .filter(session_tabs::Column::ReleasedAt.is_null())
        .one(&txn)
        .await?;
    if let Some(existing) = existing {
        if existing.session_id == session_id && existing.agent_id == agent_id {
            txn.commit().await?;
            return Ok(existing.id);
        }
        SessionTabs::update_many()
            .col_expr(session_tabs::Column::ReleasedAt, Expr::value(claimed_at))
            .filter(session_tabs::Column::Id.eq(existing.id))
            .exec(&txn)
            .await?;
    }
    let result = SessionTabs::insert(session_tabs::ActiveModel {
        id: NotSet,
        session_id: Set(session_id.to_string()),
        agent_id: Set(agent_id.to_string()),
        tab_id: Set(tab_id),
        opened_target_id: Set(opened_target_id.map(str::to_string)),
        claimed_at: Set(claimed_at),
        released_at: Set(None),
    })
    .exec(&txn)
    .await?;
    txn.commit().await?;
    Ok(result.last_insert_id)
}

async fn inherit_tab_ownership(
    db: &sea_orm::DatabaseConnection,
    opener_tab_id: i64,
    tab_id: i64,
    opened_target_id: &str,
    claimed_at: i64,
) -> AppResult<Option<i64>> {
    let Some(owner) = SessionTabs::find()
        .filter(session_tabs::Column::TabId.eq(opener_tab_id))
        .filter(session_tabs::Column::ReleasedAt.is_null())
        .one(db)
        .await?
    else {
        return Ok(None);
    };
    claim_tab_for_session(
        db,
        tab_id,
        Some(opened_target_id),
        &owner.session_id,
        &owner.agent_id,
        claimed_at,
    )
    .await
    .map(Some)
}

async fn release_tab_for_session(
    db: &sea_orm::DatabaseConnection,
    tab_id: i64,
    session_id: &str,
    released_at: i64,
) -> AppResult<u64> {
    let result = SessionTabs::update_many()
        .col_expr(session_tabs::Column::ReleasedAt, Expr::value(released_at))
        .filter(session_tabs::Column::TabId.eq(tab_id))
        .filter(session_tabs::Column::SessionId.eq(session_id))
        .filter(session_tabs::Column::ReleasedAt.is_null())
        .exec(db)
        .await?;
    Ok(result.rows_affected)
}

async fn release_claims_for_target(
    db: &sea_orm::DatabaseConnection,
    target_id: &str,
) -> AppResult<u64> {
    let result = TabClaims::update_many()
        .col_expr(tab_claims::Column::ReleasedAt, Expr::value(now_epoch_ms()))
        .filter(tab_claims::Column::TargetId.eq(target_id))
        .filter(tab_claims::Column::ReleasedAt.is_null())
        .exec(db)
        .await?;
    Ok(result.rows_affected)
}

#[cfg(test)]
mod tests {
    use super::{ClaimWrite, SessionTabLedger};
    use crate::db::{
        DATABASE_FILENAME, Database,
        entities::prelude::{SessionTabs, TabClaims},
    };
    use sea_orm::EntityTrait;
    use tempfile::tempdir;

    #[tokio::test]
    async fn queued_claim_mutations_preserve_lifecycle_order() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let ledger =
            SessionTabLedger::new(Database::open(dir.path().join(DATABASE_FILENAME)).await?);
        ledger.enqueue_claim_target_for_session(
            "target-a".to_string(),
            "session-a".to_string(),
            "agent-a".to_string(),
            100,
        );
        ledger.enqueue_release_claims_for_session("session-a".to_string());
        ledger.drain_writes().await;

        let claim = TabClaims::find()
            .one(ledger.connection())
            .await?
            .unwrap_or_else(|| panic!("queued claim missing"));
        assert!(claim.released_at.is_some());
        Ok(())
    }

    #[tokio::test]
    async fn tab_claim_transfer_closes_the_prior_owner_at_the_boundary() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let ledger =
            SessionTabLedger::new(Database::open(dir.path().join(DATABASE_FILENAME)).await?);
        ledger.enqueue_claim_tab_for_session(
            11,
            Some("target-a".to_string()),
            "session-a".to_string(),
            "agent-a".to_string(),
            100,
        );
        ledger.enqueue_claim_tab_for_session(
            11,
            Some("target-b".to_string()),
            "session-b".to_string(),
            "agent-b".to_string(),
            200,
        );
        ledger.drain_writes().await;

        let claims = SessionTabs::find().all(ledger.connection()).await?;
        assert_eq!(claims.len(), 2);
        assert_eq!(claims[0].session_id, "session-a");
        assert_eq!(claims[0].released_at, Some(200));
        assert_eq!(claims[1].session_id, "session-b");
        assert_eq!(claims[1].released_at, None);
        Ok(())
    }

    #[tokio::test]
    async fn queued_tab_release_preserves_the_observed_boundary() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let ledger =
            SessionTabLedger::new(Database::open(dir.path().join(DATABASE_FILENAME)).await?);
        ledger.enqueue_claim_tab_for_session(
            11,
            Some("target-a".to_string()),
            "session-a".to_string(),
            "agent-a".to_string(),
            100,
        );
        ledger.enqueue_claim_write(ClaimWrite::ReleaseTabForSession {
            tab_id: 11,
            session_id: "session-a".to_string(),
            released_at: 150,
        });
        ledger.drain_writes().await;

        let claim = SessionTabs::find()
            .one(ledger.connection())
            .await?
            .unwrap_or_else(|| panic!("queued tab claim missing"));
        assert_eq!(claim.released_at, Some(150));
        Ok(())
    }
}
