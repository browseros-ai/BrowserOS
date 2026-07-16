use crate::{
    domain::{AgentKey, AgentPageOwnership, AgentRef, ClientInfo, Session, SessionId},
    error::AppResult,
    services::{audit::AuditService, replay::ReplayService},
};
use futures_util::future::BoxFuture;
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, OnceLock},
    time::Duration,
};
use tokio::{
    sync::RwLock,
    task::JoinHandle,
    time::{Instant, MissedTickBehavior, interval},
};
use tracing::{debug, warn};
use ulid::Ulid;

pub type LastSessionTeardownHook =
    Arc<dyn Fn(Arc<AgentPageOwnership>, AgentKey) -> BoxFuture<'static, ()> + Send + Sync>;

pub struct SessionRegistry {
    sessions: RwLock<HashMap<SessionId, Arc<Session>>>,
    ownership: Arc<AgentPageOwnership>,
    audit: Arc<AuditService>,
    replay: Arc<ReplayService>,
    fallback_sessions: RwLock<HashSet<SessionId>>,
    last_session_teardown_hook: OnceLock<LastSessionTeardownHook>,
    idle_after: Duration,
    sweep_interval: Duration,
}

impl SessionRegistry {
    #[must_use]
    pub fn new(
        audit: Arc<AuditService>,
        replay: Arc<ReplayService>,
        idle_after: Duration,
        sweep_interval: Duration,
    ) -> Arc<Self> {
        Arc::new(Self {
            sessions: RwLock::new(HashMap::new()),
            ownership: Arc::new(AgentPageOwnership::new()),
            audit,
            replay,
            fallback_sessions: RwLock::new(HashSet::new()),
            last_session_teardown_hook: OnceLock::new(),
            idle_after,
            sweep_interval,
        })
    }

    #[must_use]
    pub fn ownership(&self) -> Arc<AgentPageOwnership> {
        self.ownership.clone()
    }

    /// Installs the host cleanup invoked when an ownership key loses its final session.
    pub fn set_last_session_teardown_hook(&self, hook: LastSessionTeardownHook) {
        let _ = self.last_session_teardown_hook.set(hook);
    }

    pub async fn mint(
        self: &Arc<Self>,
        agent: AgentRef,
        client: ClientInfo,
    ) -> AppResult<Arc<Session>> {
        let id = SessionId::new(Ulid::new().to_string());
        self.mint_with_id(id, agent, client).await
    }

    pub async fn mint_with_id(
        self: &Arc<Self>,
        id: SessionId,
        agent: AgentRef,
        client: ClientInfo,
    ) -> AppResult<Arc<Session>> {
        let session = Session::new(id.clone(), agent, Instant::now());
        self.audit
            .record_session_start(
                id.as_str(),
                session.agent().agent_id().as_str(),
                session.agent().slug(),
                session.agent().label(),
                client.name.as_str(),
                client.version.as_str(),
            )
            .await?;
        if super::agent_ref::slugify_client_name(&client.name).is_none() {
            self.fallback_sessions.write().await.insert(id.clone());
        }
        self.sessions.write().await.insert(id, session.clone());
        Ok(session)
    }

    pub async fn insert_for_testing(&self, session: Arc<Session>) {
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

    pub async fn cancel_by_agent(&self, agent_id: &str) -> usize {
        let sessions: Vec<Arc<Session>> = self.sessions.read().await.values().cloned().collect();
        let mut cancelled = 0;
        for session in sessions {
            if session.agent().agent_id().as_str() == agent_id {
                cancelled += session.cancel_active_dispatches().await;
            }
        }
        cancelled
    }

    pub async fn owner_of_page(&self, page_id: &browseros_core::PageId) -> Option<AgentKey> {
        self.ownership.owner_of_page(page_id).await
    }

    pub async fn remove(
        &self,
        id: &SessionId,
        kind: &str,
        reason: Option<&str>,
    ) -> AppResult<bool> {
        let session = self.sessions.write().await.remove(id);
        if let Some(session) = session {
            let fallback = self.fallback_sessions.write().await.remove(id);
            self.teardown(session, kind, reason, true, fallback).await?;
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
        Ok(removed)
    }

    pub async fn shutdown(&self) -> AppResult<usize> {
        let sessions = {
            let mut guard = self.sessions.write().await;
            std::mem::take(&mut *guard)
        };
        let fallback_sessions = {
            let mut guard = self.fallback_sessions.write().await;
            std::mem::take(&mut *guard)
        };
        let mut remaining_by_key = HashMap::<AgentKey, usize>::new();
        for session in sessions.values() {
            *remaining_by_key
                .entry(session.agent().ownership_key())
                .or_default() += 1;
        }
        let mut count = 0;
        for session in sessions.into_values() {
            let key = session.agent().ownership_key();
            let remaining = remaining_by_key.entry(key).or_default();
            *remaining = remaining.saturating_sub(1);
            let last_for_key = *remaining == 0;
            let fallback = fallback_sessions.contains(session.id());
            self.teardown(
                session,
                "closed",
                Some("server shutdown"),
                last_for_key,
                fallback,
            )
            .await?;
            count += 1;
        }
        Ok(count)
    }

    pub fn spawn_idle_sweeper(self: Arc<Self>) -> JoinHandle<()> {
        tokio::spawn(async move {
            let mut ticker = interval(self.sweep_interval);
            ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
            loop {
                ticker.tick().await;
                match self.sweep_idle().await {
                    Ok(count) if count > 0 => debug!(count, "swept idle sessions"),
                    Ok(_) => {}
                    Err(err) => warn!(error = %err, "session idle sweep failed"),
                }
            }
        })
    }

    async fn teardown(
        &self,
        session: Arc<Session>,
        kind: &str,
        reason: Option<&str>,
        finalize_key: bool,
        fallback: bool,
    ) -> AppResult<()> {
        session.cancel_active_dispatches().await;
        session.cancel();
        let replay_result = self.replay.close_session(session.id().as_str()).await;
        let audit_result = self
            .audit
            .record_session_end(session.id().as_str(), kind, reason)
            .await;
        let key = session.agent().ownership_key();
        for page_id in self.ownership.owned_pages(&key).await {
            session.forget_first_capture(&page_id).await;
        }
        if finalize_key {
            self.finalize_inactive_key(&key, fallback).await;
        }
        replay_result?;
        audit_result?;
        Ok(())
    }

    async fn finalize_inactive_key(&self, key: &AgentKey, fallback: bool) {
        let sessions = self.sessions.read().await;
        if sessions
            .values()
            .any(|candidate| candidate.agent().ownership_key() == *key)
        {
            return;
        }
        if let Some(hook) = self.last_session_teardown_hook.get() {
            hook(self.ownership.clone(), key.clone()).await;
        }
        if fallback {
            self.ownership.forget(key).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::SessionRegistry;
    use crate::{
        domain::{AgentRef, ClientInfo, Session, SessionId},
        services::{audit::AuditService, replay::ReplayService},
    };
    use std::{
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
        time::Duration,
    };
    use tempfile::tempdir;
    use tokio::time::Instant;

    #[tokio::test(start_paused = true)]
    async fn sweep_removes_idle_sessions_and_writes_end_row() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let audit = Arc::new(AuditService::open(dir.path().join("audit.sqlite")).await?);
        let replay = Arc::new(ReplayService::new(
            dir.path().join("replays"),
            50,
            Duration::from_secs(30),
        ));
        let registry = SessionRegistry::new(
            audit.clone(),
            replay,
            Duration::from_secs(5),
            Duration::from_secs(1),
        );
        let session = Session::new(
            SessionId::new("s1"),
            AgentRef::Ephemeral {
                agent_id: crate::domain::AgentId::new("a1"),
                slug: "a1".to_string(),
                label: "A1".to_string(),
            },
            Instant::now(),
        );
        registry.insert_for_testing(session).await;
        tokio::time::advance(Duration::from_secs(6)).await;
        assert_eq!(registry.sweep_idle().await?, 1);
        assert_eq!(registry.count().await, 0);
        let detail = audit.get_task("s1").await?;
        assert!(detail.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn mint_registers_live_session() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let audit = Arc::new(AuditService::open(dir.path().join("audit.sqlite")).await?);
        let replay = Arc::new(ReplayService::new(
            dir.path().join("replays"),
            50,
            Duration::from_secs(30),
        ));
        let registry = SessionRegistry::new(
            audit,
            replay,
            Duration::from_secs(60),
            Duration::from_secs(1),
        );
        let session = registry
            .mint(
                AgentRef::Ephemeral {
                    agent_id: crate::domain::AgentId::new("agent-1"),
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
    async fn ownership_survives_reconnect_and_preserves_tab_group() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let audit = Arc::new(AuditService::open(dir.path().join("audit.sqlite")).await?);
        let replay = Arc::new(ReplayService::new(
            dir.path().join("replays"),
            50,
            Duration::from_secs(30),
        ));
        let registry = SessionRegistry::new(
            audit,
            replay,
            Duration::from_secs(60),
            Duration::from_secs(1),
        );
        let session1 = Session::new(
            SessionId::new("s1"),
            AgentRef::Ephemeral {
                agent_id: crate::domain::AgentId::new("codex-a"),
                slug: "codex".to_string(),
                label: "Codex".to_string(),
            },
            Instant::now(),
        );
        let key1 = session1.agent().ownership_key();
        registry.insert_for_testing(session1.clone()).await;
        registry
            .ownership()
            .claim_page(key1.clone(), browseros_core::PageId(1))
            .await;
        registry
            .ownership()
            .claim_page(key1.clone(), browseros_core::PageId(2))
            .await;
        registry
            .ownership()
            .set_tab_group(
                key1.clone(),
                Some("group-1".to_string()),
                Some(crate::domain::TabGroupColor::Purple),
            )
            .await;

        assert!(
            registry
                .remove(session1.id(), "closed", Some("reconnect"))
                .await?
        );

        let session2 = Session::new(
            SessionId::new("s2"),
            AgentRef::Ephemeral {
                agent_id: crate::domain::AgentId::new("codex-b"),
                slug: "codex".to_string(),
                label: "Codex".to_string(),
            },
            Instant::now(),
        );
        let key2 = session2.agent().ownership_key();
        registry.insert_for_testing(session2).await;

        assert_eq!(key1, key2);
        assert_eq!(
            registry
                .ownership()
                .owned_pages(&key2)
                .await
                .into_iter()
                .collect::<Vec<_>>(),
            vec![browseros_core::PageId(1), browseros_core::PageId(2)]
        );
        assert_eq!(
            registry.ownership().tab_group_ref(&key2).await.as_deref(),
            Some("group-1")
        );
        assert_eq!(
            registry.ownership().tab_group_color(&key2).await,
            Some(crate::domain::TabGroupColor::Purple)
        );
        Ok(())
    }

    #[tokio::test]
    async fn fallback_identity_is_forgotten_after_its_last_session() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let audit = Arc::new(AuditService::open(dir.path().join("audit.sqlite")).await?);
        let replay = Arc::new(ReplayService::new(
            dir.path().join("replays"),
            50,
            Duration::from_secs(30),
        ));
        let registry = SessionRegistry::new(
            audit,
            replay,
            Duration::from_secs(60),
            Duration::from_secs(1),
        );
        let session_id = SessionId::new("fallback-session");
        let session = registry
            .mint_with_id(
                session_id.clone(),
                AgentRef::Ephemeral {
                    agent_id: crate::domain::AgentId::new("unknown-a"),
                    slug: "unknown-a".to_string(),
                    label: "unknown-a".to_string(),
                },
                ClientInfo {
                    name: "...".to_string(),
                    version: "1".to_string(),
                    title: None,
                },
            )
            .await?;
        let key = session.agent().ownership_key();
        registry
            .ownership()
            .claim_page(key.clone(), browseros_core::PageId(4))
            .await;
        session
            .mark_first_capture_done(browseros_core::PageId(4))
            .await;
        registry
            .ownership()
            .set_tab_group_ref(key.clone(), Some("group-4".to_string()))
            .await;

        assert!(registry.remove(&session_id, "closed", None).await?);

        assert_eq!(
            registry
                .ownership()
                .owner_of_page(&browseros_core::PageId(4))
                .await,
            None
        );
        assert_eq!(registry.ownership().tab_group_ref(&key).await, None);
        assert!(!session.has_first_capture(&browseros_core::PageId(4)).await);
        Ok(())
    }

    #[tokio::test]
    async fn last_session_teardown_hook_fires_once_per_ownership_key() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let audit = Arc::new(AuditService::open(dir.path().join("audit.sqlite")).await?);
        let replay = Arc::new(ReplayService::new(
            dir.path().join("replays"),
            50,
            Duration::from_secs(30),
        ));
        let registry = SessionRegistry::new(
            audit,
            replay,
            Duration::from_secs(60),
            Duration::from_secs(1),
        );
        let calls = Arc::new(AtomicUsize::new(0));
        let hook_calls = calls.clone();
        registry.set_last_session_teardown_hook(Arc::new(move |ownership, key| {
            let hook_calls = hook_calls.clone();
            Box::pin(async move {
                let _ = (ownership, key);
                hook_calls.fetch_add(1, Ordering::SeqCst);
            })
        }));
        for id in ["s1", "s2"] {
            registry
                .insert_for_testing(Session::new(
                    SessionId::new(id),
                    AgentRef::Ephemeral {
                        agent_id: crate::domain::AgentId::new(format!("codex-{id}")),
                        slug: "codex".to_string(),
                        label: "Codex".to_string(),
                    },
                    Instant::now(),
                ))
                .await;
        }

        registry
            .remove(&SessionId::new("s1"), "closed", None)
            .await?;
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        registry
            .remove(&SessionId::new("s2"), "closed", None)
            .await?;
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        Ok(())
    }

    #[tokio::test]
    async fn inactive_key_finalization_rechecks_for_a_reconnected_session() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let audit = Arc::new(AuditService::open(dir.path().join("audit.sqlite")).await?);
        let replay = Arc::new(ReplayService::new(
            dir.path().join("replays"),
            50,
            Duration::from_secs(30),
        ));
        let registry = SessionRegistry::new(
            audit,
            replay,
            Duration::from_secs(60),
            Duration::from_secs(1),
        );
        let calls = Arc::new(AtomicUsize::new(0));
        let hook_calls = calls.clone();
        registry.set_last_session_teardown_hook(Arc::new(move |ownership, key| {
            let hook_calls = hook_calls.clone();
            Box::pin(async move {
                let _ = (ownership, key);
                hook_calls.fetch_add(1, Ordering::SeqCst);
            })
        }));
        let session1 = Session::new(
            SessionId::new("s1"),
            AgentRef::Ephemeral {
                agent_id: crate::domain::AgentId::new("codex-a"),
                slug: "codex".to_string(),
                label: "Codex".to_string(),
            },
            Instant::now(),
        );
        let key = session1.agent().ownership_key();
        registry.insert_for_testing(session1.clone()).await;
        registry
            .ownership()
            .set_tab_group_ref(key.clone(), Some("group-1".to_string()))
            .await;

        let removed = registry.sessions.write().await.remove(session1.id());
        assert!(removed.is_some());
        registry
            .mint_with_id(
                SessionId::new("s2"),
                AgentRef::Ephemeral {
                    agent_id: crate::domain::AgentId::new("codex-b"),
                    slug: "codex".to_string(),
                    label: "Codex".to_string(),
                },
                ClientInfo {
                    name: "Codex".to_string(),
                    version: "1".to_string(),
                    title: None,
                },
            )
            .await?;

        registry.finalize_inactive_key(&key, true).await;

        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert_eq!(
            registry.ownership().tab_group_ref(&key).await.as_deref(),
            Some("group-1")
        );
        Ok(())
    }
}
