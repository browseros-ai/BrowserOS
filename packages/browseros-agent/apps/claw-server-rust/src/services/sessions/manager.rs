use super::{PageOwnership, Session};
use crate::{
    analytics::{AnalyticsSink, NoopAnalyticsSink, events},
    db::{AuditLog, SessionTabLedger},
    error::{AppError, AppResult},
    identity::{ClientIdentity, ClientInfo, ConversationIdentity, generate_fun_name},
    ids::{ConvoId, SessionId},
};
use futures_util::future::BoxFuture;
use serde_json::json;
use std::{
    collections::{HashMap, HashSet},
    future::Future,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::{
    sync::{Mutex, Notify, RwLock},
    task::JoinHandle,
    time::{Instant, MissedTickBehavior, interval},
};
use tokio_util::sync::CancellationToken;
use tracing::{debug, warn};
use ulid::Ulid;

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum RetainedGroupAction {
    Collapse,
    Close,
}

pub type RetainedGroupHook = Arc<
    dyn Fn(Arc<PageOwnership>, ConvoId, RetainedGroupAction) -> BoxFuture<'static, bool>
        + Send
        + Sync,
>;

struct RetainedSession {
    ended_at: Instant,
}

/**
 * Tracks sessions removed from the live map whose asynchronous teardown is still running. A
 * removal registers while holding the sessions write lock, so shutdown cannot observe an empty
 * map before the teardown it must await is visible here.
 */
#[derive(Default)]
struct TeardownTracker {
    active: AtomicUsize,
    idle: Notify,
}

impl TeardownTracker {
    fn begin(&self) -> TeardownPermit<'_> {
        self.active.fetch_add(1, Ordering::AcqRel);
        TeardownPermit { tracker: self }
    }

    async fn wait_until_idle(&self) {
        loop {
            let idle = self.idle.notified();
            if self.active.load(Ordering::Acquire) == 0 {
                return;
            }
            idle.await;
        }
    }
}

struct TeardownPermit<'a> {
    tracker: &'a TeardownTracker,
}

impl Drop for TeardownPermit<'_> {
    fn drop(&mut self) {
        if self.tracker.active.fetch_sub(1, Ordering::AcqRel) == 1 {
            self.tracker.idle.notify_one();
        }
    }
}

/// Owns live MCP sessions and retained conversation state. Minting resolves identity and records
/// the audit start; teardown closes replay and audit state, then retains browser groups until a
/// later reap succeeds.
pub struct Sessions {
    sessions: RwLock<HashMap<SessionId, Arc<Session>>>,
    ownership: Arc<PageOwnership>,
    audit_log: Arc<AuditLog>,
    session_tabs: Arc<SessionTabLedger>,
    analytics: Arc<dyn AnalyticsSink>,
    teardowns: TeardownTracker,
    reserved_keys: Mutex<HashSet<ConvoId>>,
    retained: RwLock<HashMap<ConvoId, RetainedSession>>,
    reaping_keys: Mutex<HashSet<ConvoId>>,
    retained_group_hook: OnceLock<RetainedGroupHook>,
    idle_after: Duration,
    retention: Duration,
    sweep_interval: Duration,
}

impl Sessions {
    #[must_use]
    pub fn new(
        audit_log: Arc<AuditLog>,
        session_tabs: Arc<SessionTabLedger>,
        idle_after: Duration,
        retention: Duration,
        sweep_interval: Duration,
    ) -> Arc<Self> {
        Self::new_with_analytics(
            audit_log,
            session_tabs,
            idle_after,
            retention,
            sweep_interval,
            Arc::new(NoopAnalyticsSink),
        )
    }

    #[must_use]
    pub fn new_with_analytics(
        audit_log: Arc<AuditLog>,
        session_tabs: Arc<SessionTabLedger>,
        idle_after: Duration,
        retention: Duration,
        sweep_interval: Duration,
        analytics: Arc<dyn AnalyticsSink>,
    ) -> Arc<Self> {
        Arc::new(Self {
            sessions: RwLock::new(HashMap::new()),
            ownership: Arc::new(PageOwnership::new()),
            audit_log,
            session_tabs,
            analytics,
            teardowns: TeardownTracker::default(),
            reserved_keys: Mutex::new(HashSet::new()),
            retained: RwLock::new(HashMap::new()),
            reaping_keys: Mutex::new(HashSet::new()),
            retained_group_hook: OnceLock::new(),
            idle_after,
            retention,
            sweep_interval,
        })
    }

    #[must_use]
    pub fn ownership(&self) -> Arc<PageOwnership> {
        self.ownership.clone()
    }

    /// Installs browser-backed retained-group collapse and close operations.
    pub fn set_retained_group_hook(&self, hook: RetainedGroupHook) {
        let _ = self.retained_group_hook.set(hook);
    }

    pub async fn mint(
        self: &Arc<Self>,
        agent: ClientIdentity,
        client: ClientInfo,
    ) -> AppResult<Arc<Session>> {
        let id = SessionId::new(Ulid::new().to_string());
        self.mint_with_id(id, agent, client).await
    }

    pub async fn mint_with_id(
        self: &Arc<Self>,
        id: SessionId,
        agent: ClientIdentity,
        client: ClientInfo,
    ) -> AppResult<Arc<Session>> {
        let identity = {
            let mut reserved_keys = self.reserved_keys.lock().await;
            let generated_label = generate_fun_name(rand::random::<f64>, |label| {
                !reserved_keys.contains(&ConvoId::new(format!("{}-{label}", agent.slug())))
            })
            .map_err(|error| AppError::Internal(error.to_string()))?;
            let identity = ConversationIdentity::new(agent.slug(), generated_label);
            reserved_keys.insert(identity.convo_id().clone());
            identity
        };
        let session = Session::new(id.clone(), agent, identity, Instant::now());
        let audit_result = self
            .audit_log
            .record_session_start(
                id.as_str(),
                session.convo_id().as_str(),
                session.agent().slug(),
                session.agent().label(),
                client.name.as_str(),
                client.version.as_str(),
            )
            .await;
        self.finish_mint(id, session, client.name, audit_result)
            .await
    }

    async fn finish_mint(
        &self,
        id: SessionId,
        session: Arc<Session>,
        client_name: String,
        audit_result: AppResult<()>,
    ) -> AppResult<Arc<Session>> {
        if let Err(error) = audit_result {
            self.reserved_keys.lock().await.remove(session.convo_id());
            return Err(error);
        }
        self.sessions.write().await.insert(id, session.clone());
        self.analytics.capture(
            events::AGENT_SESSION_STARTED,
            json!({ "client_name": client_name }),
        );
        Ok(session)
    }

    pub async fn insert_for_testing(&self, session: Arc<Session>) {
        self.reserved_keys
            .lock()
            .await
            .insert(session.convo_id().clone());
        self.sessions
            .write()
            .await
            .insert(session.id().clone(), session);
    }

    pub async fn lookup(&self, id: &SessionId) -> Option<Arc<Session>> {
        self.sessions.read().await.get(id).cloned()
    }

    pub async fn contains(&self, id: &SessionId) -> bool {
        self.sessions.read().await.contains_key(id)
    }

    /// Returns the current live sessions in stable id order for read-side joins.
    pub async fn snapshot(&self) -> Vec<Arc<Session>> {
        let mut sessions: Vec<_> = self.sessions.read().await.values().cloned().collect();
        sessions.sort_by(|left, right| left.id().cmp(right.id()));
        sessions
    }

    pub async fn touch(&self, id: &SessionId) -> bool {
        let Some(session) = self.lookup(id).await else {
            return false;
        };
        session.touch(Instant::now()).await;
        true
    }

    pub async fn count(&self) -> usize {
        self.sessions.read().await.len()
    }

    pub async fn cancel_by_convo(&self, convo_id: &ConvoId) -> usize {
        let sessions: Vec<Arc<Session>> = self.sessions.read().await.values().cloned().collect();
        let mut cancelled = 0;
        for session in sessions {
            if session.convo_id() == convo_id {
                cancelled += session.cancel_active_dispatches().await;
            }
        }
        cancelled
    }

    /// `None` when the session is not live; `Some(n)` aborts its active
    /// dispatches and reports how many there were.
    pub async fn cancel_by_session(&self, session_id: &SessionId) -> Option<usize> {
        let session = self.lookup(session_id).await?;
        Some(session.cancel_active_dispatches().await)
    }

    pub async fn owner_of_page(&self, page_id: &browseros_core::PageId) -> Option<ConvoId> {
        self.ownership.owner_of_page(page_id).await
    }

    pub async fn remove(
        &self,
        id: &SessionId,
        kind: &str,
        reason: Option<&str>,
    ) -> AppResult<bool> {
        let session = {
            let mut sessions = self.sessions.write().await;
            sessions
                .remove(id)
                .map(|session| (session, self.teardowns.begin()))
        };
        if let Some((session, _teardown)) = session {
            self.teardown(session, kind, reason).await?;
            return Ok(true);
        }
        Ok(false)
    }

    pub async fn sweep_idle(&self) -> AppResult<usize> {
        let now = Instant::now();
        let sessions: Vec<(SessionId, Arc<Session>)> = self
            .sessions
            .read()
            .await
            .iter()
            .map(|(id, session)| (id.clone(), session.clone()))
            .collect();
        let mut expired = Vec::new();
        for (id, session) in sessions {
            if session.idle_for(now).await >= self.idle_after {
                expired.push(id);
            }
        }
        let mut removed = 0;
        for id in expired {
            if self.remove(&id, "closed", Some("idle timeout")).await? {
                removed += 1;
            }
        }
        self.reap_retained(now).await;
        Ok(removed)
    }

    pub async fn shutdown(&self) -> AppResult<usize> {
        let sessions = {
            let mut guard = self.sessions.write().await;
            std::mem::take(&mut *guard)
        };
        let result = teardown_all(sessions.into_values(), |session| {
            self.teardown(session, "closed", Some("server shutdown"))
        })
        .await;
        self.teardowns.wait_until_idle().await;
        result
    }

    pub fn spawn_idle_sweeper(self: Arc<Self>, cancel: CancellationToken) -> JoinHandle<()> {
        tokio::spawn(async move {
            let mut ticker = interval(self.sweep_interval);
            ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
            loop {
                tokio::select! {
                    () = cancel.cancelled() => return,
                    _ = ticker.tick() => {
                        match self.sweep_idle().await {
                            Ok(count) if count > 0 => debug!(count, "swept idle sessions"),
                            Ok(_) => {}
                            Err(err) => warn!(error = %err, "session idle sweep failed"),
                        }
                    }
                }
            }
        })
    }

    async fn teardown(
        &self,
        session: Arc<Session>,
        kind: &str,
        reason: Option<&str>,
    ) -> AppResult<()> {
        session.cancel_active_dispatches().await;
        session.cancel();
        self.session_tabs
            .enqueue_release_claims_for_session(session.id().as_str().to_string());
        let audit_result = self
            .audit_log
            .record_session_end(session.id().as_str(), kind, reason)
            .await;
        self.finish_teardown(session, kind, audit_result).await
    }

    async fn finish_teardown(
        &self,
        session: Arc<Session>,
        kind: &str,
        audit_result: AppResult<()>,
    ) -> AppResult<()> {
        let key = session.convo_id().clone();
        self.retained.write().await.insert(
            key.clone(),
            RetainedSession {
                ended_at: Instant::now(),
            },
        );
        if let Some(hook) = self.retained_group_hook.get() {
            hook(self.ownership.clone(), key, RetainedGroupAction::Collapse).await;
        }
        self.analytics
            .capture(events::AGENT_SESSION_ENDED, json!({ "kind": kind }));
        audit_result?;
        Ok(())
    }

    async fn reap_retained(&self, now: Instant) -> usize {
        let retained: Vec<(ConvoId, Instant)> = self
            .retained
            .read()
            .await
            .iter()
            .map(|(key, retained)| (key.clone(), retained.ended_at))
            .collect();
        let mut expired = Vec::new();
        let mut active = Vec::new();
        {
            let mut reaping = self.reaping_keys.lock().await;
            for (key, ended_at) in retained {
                if now.duration_since(ended_at) >= self.retention {
                    if reaping.insert(key.clone()) {
                        expired.push(key);
                    }
                } else if !reaping.contains(&key) {
                    active.push(key);
                }
            }
        }
        if let Some(hook) = self.retained_group_hook.get() {
            for key in active {
                hook(self.ownership.clone(), key, RetainedGroupAction::Collapse).await;
            }
        }

        let mut reaped = 0;
        for key in expired {
            let closed = match self.retained_group_hook.get() {
                Some(hook) => {
                    hook(
                        self.ownership.clone(),
                        key.clone(),
                        RetainedGroupAction::Close,
                    )
                    .await
                }
                None => self.ownership.tab_group_ref(&key).await.is_none(),
            };
            if closed && self.retained.write().await.remove(&key).is_some() {
                self.ownership.forget(&key).await;
                self.reserved_keys.lock().await.remove(&key);
                reaped += 1;
            }
            self.reaping_keys.lock().await.remove(&key);
        }
        reaped
    }
}

async fn teardown_all<T, I, F, Fut>(items: I, mut teardown: F) -> AppResult<usize>
where
    I: IntoIterator<Item = T>,
    F: FnMut(T) -> Fut,
    Fut: Future<Output = AppResult<()>>,
{
    let mut count = 0;
    let mut first_error = None;
    for item in items {
        count += 1;
        if let Err(error) = teardown(item).await
            && first_error.is_none()
        {
            first_error = Some(error);
        }
    }
    match first_error {
        Some(error) => Err(error),
        None => Ok(count),
    }
}

#[cfg(test)]
mod tests {
    use super::{RetainedGroupAction, Session, Sessions, teardown_all};
    use crate::{
        analytics::{AnalyticsSink, events},
        db::{AuditLog, DATABASE_FILENAME, Database, SessionTabLedger},
        identity::{ClientIdentity, ClientInfo, ConversationIdentity, generate_fun_name},
        ids::{ConvoId, SessionId},
    };
    use serde_json::{Value, json};
    use std::{
        sync::{
            Arc, Mutex as StdMutex,
            atomic::{AtomicBool, AtomicUsize, Ordering},
        },
        time::Duration,
    };
    use tempfile::tempdir;
    use tokio::time::Instant;

    #[derive(Default)]
    struct RecordingAnalytics {
        events: StdMutex<Vec<(events::EventDefinition, Value)>>,
    }

    impl AnalyticsSink for RecordingAnalytics {
        fn capture(&self, event: events::EventDefinition, properties: Value) {
            self.events
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push((event, properties));
        }
    }

    impl RecordingAnalytics {
        fn snapshot(&self) -> Vec<(events::EventDefinition, Value)> {
            self.events
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clone()
        }
    }

    async fn repositories(
        dir: &tempfile::TempDir,
    ) -> anyhow::Result<(Arc<AuditLog>, Arc<SessionTabLedger>)> {
        let database = Database::open(dir.path().join(DATABASE_FILENAME)).await?;
        Ok((
            Arc::new(AuditLog::new(database.clone())),
            Arc::new(SessionTabLedger::new(database)),
        ))
    }

    #[tokio::test]
    async fn sweep_removes_idle_sessions_and_writes_end_row() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let (audit_log, session_tabs) = repositories(&dir).await?;
        let registry = Sessions::new(
            audit_log.clone(),
            session_tabs.clone(),
            Duration::from_secs(5),
            Duration::from_secs(60),
            Duration::from_secs(1),
        );
        let session = Session::new(
            SessionId::new("s1"),
            ClientIdentity::Ephemeral {
                slug: "a1".to_string(),
                label: "A1".to_string(),
            },
            ConversationIdentity::new("a1", "agile-alpaca".to_string()),
            Instant::now(),
        );
        registry.insert_for_testing(session).await;
        // Resume before the sweep writes to SQLite; its worker runs on wall time.
        tokio::time::pause();
        tokio::time::advance(Duration::from_secs(6)).await;
        tokio::time::resume();
        assert_eq!(registry.sweep_idle().await?, 1);
        assert_eq!(registry.count().await, 0);
        let detail = audit_log.get_task("s1").await?;
        assert!(detail.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn session_teardown_releases_every_open_target_claim() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let (audit_log, session_tabs) = repositories(&dir).await?;
        let registry = Sessions::new(
            audit_log.clone(),
            session_tabs.clone(),
            Duration::from_secs(60),
            Duration::from_secs(60),
            Duration::from_secs(1),
        );
        let session = Session::new(
            SessionId::new("claim-session"),
            ClientIdentity::Ephemeral {
                slug: "agent".to_string(),
                label: "Agent".to_string(),
            },
            ConversationIdentity::new("agent", "agile-alpaca".to_string()),
            Instant::now(),
        );
        registry.insert_for_testing(session.clone()).await;
        session_tabs
            .claim_target_for_session("target-a", session.id().as_str(), "agent", 1)
            .await?;
        session_tabs
            .claim_target_for_session("target-b", session.id().as_str(), "agent", 2)
            .await?;

        assert!(registry.remove(session.id(), "closed", None).await?);

        for _ in 0..100 {
            if session_tabs
                .all_legacy_claims_released_for_session(session.id().as_str())
                .await?
            {
                return Ok(());
            }
            tokio::task::yield_now().await;
        }
        anyhow::bail!("session teardown left claims open")
    }

    #[tokio::test]
    async fn mint_registers_live_session() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let (audit_log, session_tabs) = repositories(&dir).await?;
        let registry = Sessions::new(
            audit_log,
            session_tabs,
            Duration::from_secs(60),
            Duration::from_secs(60),
            Duration::from_secs(1),
        );
        let session = registry
            .mint(
                ClientIdentity::Ephemeral {
                    slug: "agent".to_string(),
                    label: "Agent".to_string(),
                },
                ClientInfo {
                    name: "Agent".to_string(),
                    version: "1".to_string(),
                    title: None,
                },
            )
            .await?;
        assert!(registry.lookup(session.id()).await.is_some());
        Ok(())
    }

    #[tokio::test]
    async fn session_lifecycle_emits_once_at_successful_state_transitions() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let (audit_log, session_tabs) = repositories(&dir).await?;
        let analytics = Arc::new(RecordingAnalytics::default());
        let registry = Sessions::new_with_analytics(
            audit_log,
            session_tabs,
            Duration::from_secs(60),
            Duration::from_secs(60),
            Duration::from_secs(1),
            analytics.clone(),
        );
        let session = registry
            .mint(
                ClientIdentity::Ephemeral {
                    slug: "agent".to_string(),
                    label: "Agent".to_string(),
                },
                ClientInfo {
                    name: "Claude Code".to_string(),
                    version: "1".to_string(),
                    title: None,
                },
            )
            .await?;
        assert!(registry.remove(session.id(), "closed", None).await?);
        assert!(!registry.remove(session.id(), "closed", None).await?);

        assert_eq!(
            analytics.snapshot(),
            vec![
                (
                    events::AGENT_SESSION_STARTED,
                    json!({ "client_name": "Claude Code" }),
                ),
                (events::AGENT_SESSION_ENDED, json!({ "kind": "closed" }),),
            ]
        );
        Ok(())
    }

    #[tokio::test]
    async fn failed_start_audit_emits_nothing_and_failed_end_audit_still_emits()
    -> anyhow::Result<()> {
        let dir = tempdir()?;
        let (audit_log, session_tabs) = repositories(&dir).await?;
        let analytics = Arc::new(RecordingAnalytics::default());
        let registry = Sessions::new_with_analytics(
            audit_log,
            session_tabs,
            Duration::from_secs(60),
            Duration::from_secs(60),
            Duration::from_secs(1),
            analytics.clone(),
        );
        let failed_start = Session::new(
            SessionId::new("failed-start"),
            ClientIdentity::Ephemeral {
                slug: "agent".to_string(),
                label: "Agent".to_string(),
            },
            ConversationIdentity::new("agent", "failed-start-label".to_string()),
            Instant::now(),
        );
        registry
            .reserved_keys
            .lock()
            .await
            .insert(failed_start.convo_id().clone());
        assert!(
            registry
                .finish_mint(
                    failed_start.id().clone(),
                    failed_start.clone(),
                    "Codex".to_string(),
                    Err(crate::error::AppError::Internal("audit failed".to_string())),
                )
                .await
                .is_err()
        );
        assert!(registry.lookup(failed_start.id()).await.is_none());
        assert!(analytics.snapshot().is_empty());

        let failed_end = Session::new(
            SessionId::new("failed-end"),
            ClientIdentity::Ephemeral {
                slug: "agent".to_string(),
                label: "Agent".to_string(),
            },
            ConversationIdentity::new("agent", "failed-end-label".to_string()),
            Instant::now(),
        );
        assert!(
            registry
                .finish_teardown(
                    failed_end,
                    "errored",
                    Err(crate::error::AppError::Internal("audit failed".to_string())),
                )
                .await
                .is_err()
        );
        assert_eq!(
            analytics.snapshot(),
            vec![(events::AGENT_SESSION_ENDED, json!({ "kind": "errored" }),)]
        );
        Ok(())
    }

    #[tokio::test]
    async fn idle_sweep_and_shutdown_emit_for_every_removed_session() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let (audit_log, session_tabs) = repositories(&dir).await?;
        let analytics = Arc::new(RecordingAnalytics::default());
        let registry = Sessions::new_with_analytics(
            audit_log,
            session_tabs,
            Duration::ZERO,
            Duration::from_secs(60),
            Duration::from_secs(1),
            analytics.clone(),
        );
        for id in ["idle-1", "idle-2"] {
            registry
                .insert_for_testing(Session::new(
                    SessionId::new(id),
                    ClientIdentity::Ephemeral {
                        slug: "agent".to_string(),
                        label: "Agent".to_string(),
                    },
                    ConversationIdentity::new("agent", format!("{id}-label")),
                    Instant::now(),
                ))
                .await;
        }
        assert_eq!(registry.sweep_idle().await?, 2);

        registry
            .insert_for_testing(Session::new(
                SessionId::new("shutdown"),
                ClientIdentity::Ephemeral {
                    slug: "agent".to_string(),
                    label: "Agent".to_string(),
                },
                ConversationIdentity::new("agent", "shutdown-label".to_string()),
                Instant::now(),
            ))
            .await;
        assert_eq!(registry.shutdown().await?, 1);
        assert_eq!(
            analytics
                .snapshot()
                .into_iter()
                .filter(|(event, _)| *event == events::AGENT_SESSION_ENDED)
                .count(),
            3
        );
        Ok(())
    }

    #[tokio::test]
    async fn shutdown_teardown_continues_after_the_first_error() {
        let attempts = Arc::new(AtomicUsize::new(0));
        let result = teardown_all([0, 1, 2], {
            let attempts = attempts.clone();
            move |index| {
                attempts.fetch_add(1, Ordering::SeqCst);
                async move {
                    if index == 0 {
                        Err(crate::error::AppError::Internal(
                            "first teardown failed".to_string(),
                        ))
                    } else {
                        Ok(())
                    }
                }
            }
        })
        .await;

        assert!(result.is_err());
        assert_eq!(attempts.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn shutdown_waits_for_a_session_removed_by_transport_close() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let (audit_log, session_tabs) = repositories(&dir).await?;
        let analytics = Arc::new(RecordingAnalytics::default());
        let registry = Sessions::new_with_analytics(
            audit_log,
            session_tabs,
            Duration::from_secs(60),
            Duration::from_secs(60),
            Duration::from_secs(1),
            analytics.clone(),
        );
        let teardown_entered = Arc::new(tokio::sync::Semaphore::new(0));
        let teardown_release = Arc::new(tokio::sync::Semaphore::new(0));
        let hook_entered = teardown_entered.clone();
        let hook_release = teardown_release.clone();
        registry.set_retained_group_hook(Arc::new(move |_, _, action| {
            let hook_entered = hook_entered.clone();
            let hook_release = hook_release.clone();
            Box::pin(async move {
                if matches!(action, RetainedGroupAction::Collapse) {
                    hook_entered.add_permits(1);
                    let Ok(permit) = hook_release.acquire().await else {
                        return false;
                    };
                    permit.forget();
                }
                true
            })
        }));
        let session = Session::new(
            SessionId::new("transport-close"),
            ClientIdentity::Ephemeral {
                slug: "agent".to_string(),
                label: "Agent".to_string(),
            },
            ConversationIdentity::new("agent", "transport-close-label".to_string()),
            Instant::now(),
        );
        registry.insert_for_testing(session.clone()).await;

        let remove_registry = registry.clone();
        let session_id = session.id().clone();
        let remove_task = tokio::spawn(async move {
            remove_registry
                .remove(&session_id, "closed", Some("transport closed"))
                .await
        });
        let entered = teardown_entered
            .acquire()
            .await
            .map_err(|_| anyhow::anyhow!("teardown semaphore closed"))?;
        entered.forget();

        let shutdown_registry = registry.clone();
        let mut shutdown_task = tokio::spawn(async move { shutdown_registry.shutdown().await });
        assert!(
            tokio::time::timeout(Duration::from_millis(50), &mut shutdown_task)
                .await
                .is_err()
        );

        teardown_release.add_permits(1);
        assert!(remove_task.await??);
        assert_eq!(shutdown_task.await??, 0);
        assert_eq!(
            analytics.snapshot(),
            vec![(events::AGENT_SESSION_ENDED, json!({ "kind": "closed" }))]
        );
        Ok(())
    }

    #[tokio::test]
    async fn same_client_sessions_get_distinct_identity_and_ownership() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let (audit_log, session_tabs) = repositories(&dir).await?;
        let registry = Sessions::new(
            audit_log,
            session_tabs,
            Duration::from_secs(60),
            Duration::from_secs(60),
            Duration::from_secs(1),
        );
        let client = ClientInfo {
            name: "Codex".to_string(),
            version: "1".to_string(),
            title: None,
        };
        let agent = ClientIdentity::Ephemeral {
            slug: "codex".to_string(),
            label: "Codex".to_string(),
        };
        let session1 = registry.mint(agent.clone(), client.clone()).await?;
        let key1 = session1.convo_id().clone();
        registry
            .ownership()
            .claim_page(key1.clone(), browseros_core::PageId(1))
            .await;
        let session2 = registry.mint(agent, client).await?;
        let key2 = session2.convo_id().clone();
        registry
            .ownership()
            .claim_page(key2.clone(), browseros_core::PageId(2))
            .await;

        assert_ne!(key1, key2);
        assert_eq!(
            registry
                .ownership()
                .owned_pages(&key1)
                .await
                .into_iter()
                .collect::<Vec<_>>(),
            vec![browseros_core::PageId(1)]
        );
        assert!(Arc::ptr_eq(
            &registry
                .lookup(session1.id())
                .await
                .ok_or_else(|| anyhow::anyhow!("session missing"))?,
            &session1
        ));
        assert_eq!(
            registry.ownership().owned_pages(&key2).await,
            std::collections::BTreeSet::from([browseros_core::PageId(2)])
        );
        Ok(())
    }

    #[tokio::test]
    async fn retained_session_collapses_then_closes_and_forgets_after_expiry() -> anyhow::Result<()>
    {
        let dir = tempdir()?;
        let (audit_log, session_tabs) = repositories(&dir).await?;
        let registry = Sessions::new(
            audit_log,
            session_tabs,
            Duration::from_secs(60),
            Duration::from_secs(10),
            Duration::from_secs(1),
        );
        let actions = Arc::new(std::sync::Mutex::new(Vec::new()));
        let hook_actions = actions.clone();
        registry.set_retained_group_hook(Arc::new(move |_, _, action| {
            let hook_actions = hook_actions.clone();
            Box::pin(async move {
                hook_actions
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .push(action);
                true
            })
        }));
        let session_id = SessionId::new("s1");
        let session = Session::new(
            session_id.clone(),
            ClientIdentity::Ephemeral {
                slug: "codex".to_string(),
                label: "Codex".to_string(),
            },
            ConversationIdentity::new("codex", "agile-alpaca".to_string()),
            Instant::now(),
        );
        let key = session.convo_id().clone();
        registry.insert_for_testing(session.clone()).await;
        registry
            .ownership()
            .claim_page(key.clone(), browseros_core::PageId(4))
            .await;
        registry
            .ownership()
            .set_tab_group_ref(key.clone(), Some("group-4".to_string()))
            .await;

        assert!(registry.remove(&session_id, "closed", None).await?);
        let retained_at = registry
            .retained
            .read()
            .await
            .get(&key)
            .map(|retained| retained.ended_at)
            .ok_or_else(|| anyhow::anyhow!("retained session missing"))?;
        assert_eq!(registry.retained.read().await.len(), 1);
        assert_eq!(
            actions
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .as_slice(),
            &[RetainedGroupAction::Collapse]
        );
        assert_eq!(
            registry
                .ownership()
                .owner_of_page(&browseros_core::PageId(4))
                .await,
            Some(key.clone())
        );

        assert_eq!(
            registry
                .reap_retained(retained_at + Duration::from_secs(9))
                .await,
            0
        );
        assert_eq!(registry.retained.read().await.len(), 1);
        assert_eq!(
            registry
                .reap_retained(retained_at + Duration::from_secs(10))
                .await,
            1
        );
        assert_eq!(registry.retained.read().await.len(), 0);
        assert_eq!(registry.ownership().tab_group_ref(&key).await, None);
        assert!(!registry.reserved_keys.lock().await.contains(&key));
        assert_eq!(
            actions
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .as_slice(),
            &[
                RetainedGroupAction::Collapse,
                RetainedGroupAction::Collapse,
                RetainedGroupAction::Close
            ]
        );
        Ok(())
    }

    #[tokio::test]
    async fn failed_or_disconnected_close_retries_without_forgetting_state() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let (audit_log, session_tabs) = repositories(&dir).await?;
        let registry = Sessions::new(
            audit_log,
            session_tabs,
            Duration::from_secs(60),
            Duration::from_secs(10),
            Duration::from_secs(1),
        );
        let close_allowed = Arc::new(AtomicBool::new(false));
        let close_attempts = Arc::new(AtomicUsize::new(0));
        let hook_allowed = close_allowed.clone();
        let hook_attempts = close_attempts.clone();
        registry.set_retained_group_hook(Arc::new(move |_, _, action| {
            let hook_allowed = hook_allowed.clone();
            let hook_attempts = hook_attempts.clone();
            Box::pin(async move {
                if matches!(action, RetainedGroupAction::Close) {
                    hook_attempts.fetch_add(1, Ordering::SeqCst);
                    return hook_allowed.load(Ordering::SeqCst);
                }
                false
            })
        }));
        let session = Session::new(
            SessionId::new("s1"),
            ClientIdentity::Ephemeral {
                slug: "codex".to_string(),
                label: "Codex".to_string(),
            },
            ConversationIdentity::new("codex", "agile-alpaca".to_string()),
            Instant::now(),
        );
        let key = session.convo_id().clone();
        registry.insert_for_testing(session.clone()).await;
        registry
            .ownership()
            .claim_page(key.clone(), browseros_core::PageId(7))
            .await;
        registry
            .ownership()
            .set_tab_group_ref(key.clone(), Some("group-7".to_string()))
            .await;
        registry.remove(session.id(), "closed", None).await?;
        let expired_at = Instant::now() + Duration::from_secs(10);

        assert_eq!(registry.reap_retained(expired_at).await, 0);
        assert_eq!(close_attempts.load(Ordering::SeqCst), 1);
        assert_eq!(registry.retained.read().await.len(), 1);
        assert_eq!(
            registry
                .ownership()
                .owner_of_page(&browseros_core::PageId(7))
                .await,
            Some(key.clone())
        );

        close_allowed.store(true, Ordering::SeqCst);
        assert_eq!(registry.reap_retained(expired_at).await, 1);
        assert_eq!(close_attempts.load(Ordering::SeqCst), 2);
        assert_eq!(registry.retained.read().await.len(), 0);
        assert_eq!(
            registry
                .ownership()
                .owner_of_page(&browseros_core::PageId(7))
                .await,
            None
        );
        Ok(())
    }

    #[tokio::test]
    async fn generated_key_stays_reserved_until_retained_cleanup() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let (audit_log, session_tabs) = repositories(&dir).await?;
        let registry = Sessions::new(
            audit_log,
            session_tabs,
            Duration::from_secs(60),
            Duration::from_secs(10),
            Duration::from_secs(1),
        );
        let session = Session::new(
            SessionId::new("s1"),
            ClientIdentity::Ephemeral {
                slug: "codex".to_string(),
                label: "Codex".to_string(),
            },
            ConversationIdentity::new("codex", "agile-alpaca".to_string()),
            Instant::now(),
        );
        registry.insert_for_testing(session.clone()).await;
        registry.remove(session.id(), "closed", None).await?;
        let expired_at = Instant::now() + Duration::from_secs(10);

        let candidate_while_retained = {
            let reserved = registry.reserved_keys.lock().await;
            generate_fun_name(
                || 0.0,
                |label| !reserved.contains(&ConvoId::new(format!("codex-{label}"))),
            )?
        };
        assert_eq!(candidate_while_retained, "agile-alpaca-2");

        assert_eq!(registry.reap_retained(expired_at).await, 1);
        let candidate_after_cleanup = {
            let reserved = registry.reserved_keys.lock().await;
            generate_fun_name(
                || 0.0,
                |label| !reserved.contains(&ConvoId::new(format!("codex-{label}"))),
            )?
        };
        assert_eq!(candidate_after_cleanup, "agile-alpaca");
        Ok(())
    }

    #[tokio::test]
    async fn overlapping_retained_sweeps_issue_one_close() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let (audit_log, session_tabs) = repositories(&dir).await?;
        let registry = Sessions::new(
            audit_log,
            session_tabs,
            Duration::from_secs(60),
            Duration::from_secs(10),
            Duration::from_secs(1),
        );
        let close_attempts = Arc::new(AtomicUsize::new(0));
        let close_entered = Arc::new(tokio::sync::Notify::new());
        let close_release = Arc::new(tokio::sync::Notify::new());
        let hook_attempts = close_attempts.clone();
        let hook_entered = close_entered.clone();
        let hook_release = close_release.clone();
        registry.set_retained_group_hook(Arc::new(move |_, _, action| {
            let hook_attempts = hook_attempts.clone();
            let hook_entered = hook_entered.clone();
            let hook_release = hook_release.clone();
            Box::pin(async move {
                if matches!(action, RetainedGroupAction::Close) {
                    hook_attempts.fetch_add(1, Ordering::SeqCst);
                    hook_entered.notify_one();
                    hook_release.notified().await;
                }
                true
            })
        }));
        let session = Session::new(
            SessionId::new("s1"),
            ClientIdentity::Ephemeral {
                slug: "codex".to_string(),
                label: "Codex".to_string(),
            },
            ConversationIdentity::new("codex", "agile-alpaca".to_string()),
            Instant::now(),
        );
        registry.insert_for_testing(session.clone()).await;
        registry.remove(session.id(), "closed", None).await?;
        let now = Instant::now() + Duration::from_secs(10);
        let entered = close_entered.notified();
        let first_registry = registry.clone();
        let first = tokio::spawn(async move { first_registry.reap_retained(now).await });
        entered.await;

        assert_eq!(registry.reap_retained(now).await, 0);
        assert_eq!(close_attempts.load(Ordering::SeqCst), 1);
        close_release.notify_one();
        assert_eq!(first.await?, 1);
        assert_eq!(registry.reap_retained(now).await, 0);
        assert_eq!(close_attempts.load(Ordering::SeqCst), 1);
        Ok(())
    }
}
