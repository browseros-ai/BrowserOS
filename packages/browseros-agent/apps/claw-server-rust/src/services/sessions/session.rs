use super::usage::{SessionUsageSnapshot, UsageTracker};
use crate::{
    identity::{ClientIdentity, ConversationIdentity},
    ids::{ConvoId, DispatchId, SessionId},
};
use std::{collections::BTreeMap, sync::Arc, time::Duration};
use tokio::{sync::Mutex, time::Instant};
use tokio_util::sync::CancellationToken;

/// Runtime state for one MCP transport session.
/// Its `SessionId` owns transport and audit lifetime; its `ConvoId` separately
/// keys tab ownership.
pub struct Session {
    id: SessionId,
    agent: ClientIdentity,
    identity: ConversationIdentity,
    usage: UsageTracker,
    active_dispatches: Mutex<BTreeMap<DispatchId, CancellationToken>>,
    cancel: CancellationToken,
    last_activity: Mutex<Instant>,
}

impl Session {
    #[must_use]
    pub fn new(
        id: SessionId,
        agent: ClientIdentity,
        identity: ConversationIdentity,
        client_name: String,
        now: Instant,
    ) -> Arc<Self> {
        Arc::new(Self {
            id,
            agent,
            identity,
            usage: UsageTracker::new(client_name),
            active_dispatches: Mutex::new(BTreeMap::new()),
            cancel: CancellationToken::new(),
            last_activity: Mutex::new(now),
        })
    }

    #[must_use]
    pub fn id(&self) -> &SessionId {
        &self.id
    }

    #[must_use]
    pub fn agent(&self) -> &ClientIdentity {
        &self.agent
    }

    #[must_use]
    pub fn convo_id(&self) -> &ConvoId {
        self.identity.convo_id()
    }

    #[must_use]
    pub fn generated_label(&self) -> &str {
        self.identity.generated_label()
    }

    #[must_use]
    pub fn client_name(&self) -> &str {
        self.usage.client_name()
    }

    pub fn mark_used(&self) {
        self.usage.mark_used();
    }

    #[must_use]
    pub fn is_used(&self) -> bool {
        self.usage.is_used()
    }

    pub async fn record_tool_usage(
        &self,
        tool_name: &str,
        elapsed: Duration,
        concurrent_used_sessions: usize,
    ) {
        self.usage
            .record(tool_name, elapsed, concurrent_used_sessions)
            .await;
    }

    pub(crate) async fn usage_snapshot(&self) -> SessionUsageSnapshot {
        self.usage.snapshot().await
    }

    pub async fn label(&self) -> String {
        self.identity.label().await
    }

    pub async fn rename(&self, new_label: String) -> String {
        self.identity.rename(new_label).await
    }

    pub async fn take_rename_nudge(&self) -> Option<String> {
        self.identity.take_rename_nudge().await
    }

    pub async fn touch(&self, now: Instant) {
        *self.last_activity.lock().await = now;
    }

    pub async fn idle_for(&self, now: Instant) -> Duration {
        now.saturating_duration_since(*self.last_activity.lock().await)
    }

    pub fn cancel(&self) {
        self.cancel.cancel();
    }

    pub async fn register_dispatch(&self, dispatch_id: DispatchId, token: CancellationToken) {
        self.active_dispatches
            .lock()
            .await
            .insert(dispatch_id, token);
    }

    pub async fn unregister_dispatch(&self, dispatch_id: &DispatchId) {
        self.active_dispatches.lock().await.remove(dispatch_id);
    }

    pub async fn cancel_active_dispatches(&self) -> usize {
        let tokens = self
            .active_dispatches
            .lock()
            .await
            .values()
            .cloned()
            .collect::<Vec<_>>();
        for token in &tokens {
            token.cancel();
        }
        tokens.len()
    }

    #[must_use]
    pub fn child_token(&self) -> CancellationToken {
        self.cancel.child_token()
    }
}
