use super::{PageOwnership, Session, TargetBindings};
use crate::{
    error::{HostError, HostResult},
    identity::{ClientIdentity, ClientInfo, ConversationIdentity, generate_fun_name},
    ids::{ConvoId, SessionId},
    noop::NoopAnalytics,
    seams::{HostAnalytics, HostEvent, SessionJournal, SessionStart},
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

/// Receives a session only after its end audit row is durable.
pub type SessionCompletionHook = Arc<dyn Fn(String) + Send + Sync>;

pub type SessionAuditFlushHook =
    Arc<dyn Fn(String) -> BoxFuture<'static, HostResult<()>> + Send + Sync>;

#[doc(hidden)]
pub struct RetainedSession {
    pub ended_at: Instant,
}

/// Why a session handle stopped resolving.
///
/// The distinction is the whole point of the tombstone: an agent that presents a handle
/// the user deliberately stopped deserves a different answer from one whose session was
/// reaped out from under it.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum RetirementCause {
    /// The user pressed Stop in the cockpit. A deliberate instruction, so it refuses.
    Cancelled,
    /// Idle sweep, transport close or shutdown. Nobody asked for it, so it redirects.
    Closed,
}

/// A handle that no longer resolves, kept just long enough to explain itself.
struct Retired {
    at: Instant,
    cause: RetirementCause,
    /// The successor minted for an agent that kept presenting this handle.
    ///
    /// Recorded once and then reused. The server already returns a fresh handle on every
    /// call and agents have been observed resending the dead one regardless, so a stubborn
    /// agent must land in one session rather than minting one per call.
    replacement: Option<SessionId>,
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
    /// Handles that stopped resolving, and what to tell an agent still presenting one.
    /// Bounded by the same retention window as `retained` and reaped by the same sweep.
    retired: RwLock<HashMap<SessionId, Retired>>,
    ownership: Arc<PageOwnership>,
    /// Durable lifecycle record (claw: audit DB and tab-claim ledger).
    journal: Arc<dyn SessionJournal>,
    analytics: Arc<dyn HostAnalytics>,
    teardowns: TeardownTracker,
    // Conversation keys remain reserved through live use and retained browser-group cleanup.
    // White-box access for claw-server's journal-backed lifecycle tests.
    #[doc(hidden)]
    pub reserved_keys: Mutex<HashSet<ConvoId>>,
    #[doc(hidden)]
    pub retained: RwLock<HashMap<ConvoId, RetainedSession>>,
    // One retained-group close per key may be in flight; failed attempts remain retryable.
    reaping_keys: Mutex<HashSet<ConvoId>>,
    /// Serializes retries per session through terminal persistence without coupling unrelated Stops.
    cancellation_transitions: Mutex<HashMap<SessionId, Arc<Mutex<()>>>>,
    retained_group_hook: OnceLock<RetainedGroupHook>,
    audit_flush_hook: OnceLock<SessionAuditFlushHook>,
    completion_hook: OnceLock<SessionCompletionHook>,
    target_bindings: OnceLock<Arc<TargetBindings>>,
    idle_after: Duration,
    retention: Duration,
    sweep_interval: Duration,
}

impl Sessions {
    #[must_use]
    pub fn new(
        journal: Arc<dyn SessionJournal>,
        idle_after: Duration,
        retention: Duration,
        sweep_interval: Duration,
    ) -> Arc<Self> {
        Self::new_with_analytics(
            journal,
            idle_after,
            retention,
            sweep_interval,
            Arc::new(NoopAnalytics),
        )
    }

    #[must_use]
    pub fn new_with_analytics(
        journal: Arc<dyn SessionJournal>,
        idle_after: Duration,
        retention: Duration,
        sweep_interval: Duration,
        analytics: Arc<dyn HostAnalytics>,
    ) -> Arc<Self> {
        Arc::new(Self {
            sessions: RwLock::new(HashMap::new()),
            retired: RwLock::new(HashMap::new()),
            ownership: Arc::new(PageOwnership::new()),
            journal,
            analytics,
            teardowns: TeardownTracker::default(),
            reserved_keys: Mutex::new(HashSet::new()),
            retained: RwLock::new(HashMap::new()),
            reaping_keys: Mutex::new(HashSet::new()),
            cancellation_transitions: Mutex::new(HashMap::new()),
            retained_group_hook: OnceLock::new(),
            audit_flush_hook: OnceLock::new(),
            completion_hook: OnceLock::new(),
            target_bindings: OnceLock::new(),
            idle_after,
            retention,
            sweep_interval,
        })
    }

    #[must_use]
    pub fn ownership(&self) -> Arc<PageOwnership> {
        self.ownership.clone()
    }

    /// The journal this registry records lifecycle through; local tools write to it too.
    #[must_use]
    pub fn journal(&self) -> &Arc<dyn SessionJournal> {
        &self.journal
    }

    #[must_use]
    pub fn analytics(&self) -> &Arc<dyn HostAnalytics> {
        &self.analytics
    }

    /// Installs browser-backed retained-group collapse and close operations.
    pub fn set_retained_group_hook(&self, hook: RetainedGroupHook) {
        let _ = self.retained_group_hook.set(hook);
    }

    /// Installs the non-blocking notification that follows durable session completion.
    pub fn set_completion_hook(&self, hook: SessionCompletionHook) {
        let _ = self.completion_hook.set(hook);
    }

    pub fn set_audit_flush_hook(&self, hook: SessionAuditFlushHook) {
        let _ = self.audit_flush_hook.set(hook);
    }

    /// Connects lifecycle retirement to durable task intent without retaining BrowserViews.
    pub fn set_target_bindings(&self, targets: Arc<TargetBindings>) {
        let _ = self.target_bindings.set(targets);
    }

    pub async fn mint(
        self: &Arc<Self>,
        agent: ClientIdentity,
        client: ClientInfo,
    ) -> HostResult<Arc<Session>> {
        let id = SessionId::new(Ulid::new().to_string());
        self.mint_with_id(id, agent, client, None).await
    }

    /// Publishes ownership and client presentation together. `declared_product`
    /// comes from the unmodified agentName, before a saved profile rewrites its slug.
    pub async fn mint_with_id(
        self: &Arc<Self>,
        id: SessionId,
        agent: ClientIdentity,
        client: ClientInfo,
        declared_product: Option<&'static str>,
    ) -> HostResult<Arc<Session>> {
        let identity = {
            let mut reserved_keys = self.reserved_keys.lock().await;
            let generated_label = generate_fun_name(rand::random::<f64>, |label| {
                !reserved_keys.contains(&ConvoId::new(format!("{}-{label}", agent.slug())))
            })
            .map_err(|error| HostError::Internal(error.to_string()))?;
            let identity = ConversationIdentity::new(agent.slug(), generated_label);
            reserved_keys.insert(identity.convo_id().clone());
            identity
        };
        let session = Session::new_with_declared_product(
            id.clone(),
            agent,
            identity,
            client.name.clone(),
            declared_product,
            Instant::now(),
        );
        let audit_result = self
            .journal
            .record_session_start(SessionStart {
                session_id: id.as_str(),
                convo_id: session.convo_id().as_str(),
                slug: session.agent().slug(),
                agent_label: session.agent().label(),
                client_name: client.name.as_str(),
                client_version: client.version.as_str(),
            })
            .await;
        self.finish_mint(id, session, audit_result).await
    }

    // Exposed for claw-server's journal-backed lifecycle tests; not host interface.
    #[doc(hidden)]
    pub async fn finish_mint(
        &self,
        id: SessionId,
        session: Arc<Session>,
        audit_result: HostResult<()>,
    ) -> HostResult<Arc<Session>> {
        if let Err(error) = audit_result {
            self.reserved_keys.lock().await.remove(session.convo_id());
            return Err(error);
        }
        self.sessions.write().await.insert(id, session.clone());
        self.analytics.capture(
            HostEvent::SessionStarted,
            json!({ "client_name": session.client_name() }),
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

    /// Why a handle stopped resolving, for a caller deciding what to tell the agent.
    ///
    /// `None` means the server genuinely has no memory of it: never minted, or retired
    /// long enough ago to have been reaped.
    pub async fn retirement_of(&self, id: &SessionId) -> Option<RetirementCause> {
        self.retired.read().await.get(id).map(|entry| entry.cause)
    }

    /// The successor already bound to a retired handle, if one is.
    ///
    /// Lets a caller take the settled answer without minting a session it would only have
    /// to discard, which is the common case: the race happens once, the resends follow.
    pub async fn replacement_of(&self, retired: &SessionId) -> Option<SessionId> {
        self.retired.read().await.get(retired)?.replacement.clone()
    }

    /// Binds one substitute to a handle that does not resolve live, and returns the
    /// substitute that won.
    ///
    /// Records the handle if it was not recorded already, because the case that matters
    /// most is a handle this server has no memory of at all. Both maps live in memory, so
    /// a restart makes every handle its agents are holding unrecognised at once. Without a
    /// binding those mint per call, and an agent faithfully resending one handle collects a
    /// session and a tab for every call it makes. Observed in the wild: one handle, seven
    /// calls, seven sessions.
    ///
    /// Compare-and-set under a single write lock, so concurrent calls presenting the same
    /// handle converge on one session instead of racing to two. A caller whose `candidate`
    /// loses is responsible for discarding it.
    ///
    /// The bound session is always server-minted; a presented handle is only ever a map
    /// key, so a caller still cannot choose or seed a session id.
    pub async fn adopt_replacement(&self, handle: &SessionId, candidate: SessionId) -> SessionId {
        let mut entries = self.retired.write().await;
        let entry = entries.entry(handle.clone()).or_insert_with(|| Retired {
            at: Instant::now(),
            cause: RetirementCause::Closed,
            replacement: None,
        });
        entry.replacement.get_or_insert(candidate).clone()
    }

    // Exposed for claw-server's journal-backed lifecycle tests; not host interface.
    #[doc(hidden)]
    pub async fn retire(&self, id: &SessionId, cause: RetirementCause) {
        self.retired.write().await.insert(
            id.clone(),
            Retired {
                at: Instant::now(),
                cause,
                replacement: None,
            },
        );
    }

    // Exposed for claw-server's journal-backed lifecycle tests; not host interface.
    #[doc(hidden)]
    pub async fn reap_retired(&self, now: Instant) -> usize {
        let mut entries = self.retired.write().await;
        let before = entries.len();
        entries.retain(|_, entry| now.duration_since(entry.at) < self.retention);
        before - entries.len()
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

    pub async fn used_count(&self) -> usize {
        self.sessions
            .read()
            .await
            .values()
            .filter(|session| session.is_used())
            .count()
    }

    /// Removes the live entry before teardown so transport requests cannot resolve it again.
    pub async fn cancel_by_session(&self, session_id: &SessionId) -> HostResult<Option<usize>> {
        let transition = self.cancellation_transition(session_id).await;
        let result = {
            let _transition = transition.lock().await;
            self.cancel_by_session_serialized(session_id).await
        };
        self.release_cancellation_transition(session_id, &transition)
            .await;
        result
    }

    async fn cancel_by_session_serialized(
        &self,
        session_id: &SessionId,
    ) -> HostResult<Option<usize>> {
        // The tombstone is written in the same critical section as the removal, not after
        // teardown. Teardown waits on in-flight dispatches and can take seconds, and that
        // gap is exactly the window in which an agent's next call would mint a session.
        // The moment a handle stops resolving is the moment it must start explaining itself.
        let session = {
            let mut sessions = self.sessions.write().await;
            if sessions.contains_key(session_id)
                && let Some(targets) = self.target_bindings.get()
            {
                // Persist Stop before removing the live handle. A restart in teardown
                // must still refuse every alias instead of adopting an unbound successor.
                targets.forget(session_id).await?;
            }
            let removed = sessions
                .remove(session_id)
                .map(|session| (session, self.teardowns.begin()));
            if removed.is_some() {
                self.retire(session_id, RetirementCause::Cancelled).await;
            }
            removed
        };
        let Some((session, _teardown)) = session else {
            return Ok(None);
        };
        session.request_operator_stop();
        match self
            .teardown(
                session.clone(),
                "cancelled",
                Some("operator requested stop"),
            )
            .await
        {
            Ok(cancelled_dispatches) => Ok(Some(cancelled_dispatches)),
            Err(error) => {
                self.restore_pending_operator_stop(session_id, session)
                    .await;
                Err(error)
            }
        }
    }

    async fn cancellation_transition(&self, session_id: &SessionId) -> Arc<Mutex<()>> {
        self.cancellation_transitions
            .lock()
            .await
            .entry(session_id.clone())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    }

    async fn release_cancellation_transition(
        &self,
        session_id: &SessionId,
        transition: &Arc<Mutex<()>>,
    ) {
        let mut transitions = self.cancellation_transitions.lock().await;
        if Arc::strong_count(transition) == 2
            && transitions
                .get(session_id)
                .is_some_and(|current| Arc::ptr_eq(current, transition))
        {
            transitions.remove(session_id);
        }
    }

    pub async fn owner_of_page(&self, page_id: &browseros_core::PageId) -> Option<ConvoId> {
        self.ownership.owner_of_page(page_id).await
    }

    pub async fn remove(
        &self,
        id: &SessionId,
        kind: &str,
        reason: Option<&str>,
    ) -> HostResult<bool> {
        let session = {
            let mut sessions = self.sessions.write().await;
            let removed = sessions
                .remove(id)
                .map(|session| (session, self.teardowns.begin()));
            if removed.is_some() {
                self.retire(id, RetirementCause::Closed).await;
            }
            removed
        };
        if let Some((session, _teardown)) = session {
            return match self.teardown(session.clone(), kind, reason).await {
                Ok(_) => Ok(true),
                Err(error) => {
                    self.restore_pending_operator_stop(id, session).await;
                    Err(error)
                }
            };
        }
        Ok(false)
    }

    // Exposed for claw-server's journal-backed lifecycle tests; not host interface.
    #[doc(hidden)]
    pub async fn restore_pending_operator_stop(&self, id: &SessionId, session: Arc<Session>) {
        if session.operator_stop_requested() {
            self.sessions
                .write()
                .await
                .entry(id.clone())
                .or_insert(session);
            // The handle resolves again, so the tombstone would be answering for a live
            // session.
            self.retired.write().await.remove(id);
        }
    }

    pub async fn sweep_idle(&self) -> HostResult<usize> {
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
        self.reap_retired(now).await;
        if let Some(targets) = self.target_bindings.get() {
            targets.sweep(crate::clock::now_epoch_ms()).await?;
        }
        Ok(removed)
    }

    pub async fn shutdown(&self) -> HostResult<usize> {
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
    ) -> HostResult<usize> {
        let operator_stop_requested = session.operator_stop_requested();
        let (kind, reason) = if operator_stop_requested {
            ("cancelled", Some("operator requested stop"))
        } else {
            (kind, reason)
        };
        let cancelled_dispatches = session.stop_dispatches().await;
        session.wait_for_dispatches().await;
        if let Some(hook) = self.audit_flush_hook.get() {
            hook(session.id().as_str().to_string()).await?;
        }
        if operator_stop_requested {
            for dispatch_id in session.pending_operator_cancellation_audits().await {
                self.journal
                    .mark_dispatch_operator_cancelled(&dispatch_id)
                    .await?;
                session
                    .mark_operator_cancellation_audit_reconciled(&dispatch_id)
                    .await;
            }
        }
        self.journal
            .release_session_tabs(session.id().as_str().to_string());
        let audit_result = self
            .journal
            .record_session_end(session.id().as_str(), kind, reason)
            .await;
        self.finish_teardown(session, kind, audit_result).await?;
        Ok(cancelled_dispatches)
    }

    // Exposed for claw-server's journal-backed lifecycle tests; not host interface.
    #[doc(hidden)]
    pub async fn finish_teardown(
        &self,
        session: Arc<Session>,
        kind: &str,
        audit_result: HostResult<()>,
    ) -> HostResult<()> {
        if audit_result.is_ok()
            && let Some(hook) = self.completion_hook.get()
        {
            hook(session.id().as_str().to_owned());
        }
        let usage = session.usage_snapshot().await;
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
        for tool in usage.tools.iter().filter(|tool| tool.dispatch_count > 0) {
            self.analytics.capture(
                HostEvent::SessionToolUsage,
                json!({
                    "client_name": usage.client_name.as_str(),
                    "tool_name": tool.tool_name.as_str(),
                    "dispatch_count": tool.dispatch_count,
                    "total_duration_ms": tool.total_duration_ms,
                    "max_duration_ms": tool.max_duration_ms,
                }),
            );
        }
        self.analytics.capture(
            HostEvent::SessionEnded,
            json!({
                "kind": kind,
                "client_name": usage.client_name.as_str(),
                "dispatch_count": usage.dispatch_count,
                "distinct_tool_count": usage.distinct_tool_count,
                "max_concurrent_used_sessions": usage.max_concurrent_used_sessions,
            }),
        );
        audit_result?;
        Ok(())
    }

    // Exposed for claw-server's journal-backed lifecycle tests; not host interface.
    #[doc(hidden)]
    pub async fn reap_retained(&self, now: Instant) -> usize {
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

// Exposed for claw-server's journal-backed lifecycle tests; not host interface.
#[doc(hidden)]
pub async fn teardown_all<T, I, F, Fut, R>(items: I, mut teardown: F) -> HostResult<usize>
where
    I: IntoIterator<Item = T>,
    F: FnMut(T) -> Fut,
    Fut: Future<Output = HostResult<R>>,
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
