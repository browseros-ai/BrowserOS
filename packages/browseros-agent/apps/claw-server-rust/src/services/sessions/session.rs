use super::usage::{SessionUsageSnapshot, UsageTracker};
use crate::{
    identity::{ClientIdentity, ConversationIdentity},
    ids::{ConvoId, DispatchId, SessionId},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::{
    sync::{Mutex, Notify},
    time::Instant,
};
use tokio_util::sync::CancellationToken;

/// Runtime state for one MCP transport session.
/// Its `SessionId` owns transport and audit lifetime; its `ConvoId` separately
/// keys tab ownership.
pub struct Session {
    id: SessionId,
    agent: ClientIdentity,
    identity: ConversationIdentity,
    usage: UsageTracker,
    dispatches: Mutex<DispatchState>,
    dispatches_drained: Notify,
    operator_stop_requested: AtomicBool,
    cancel: CancellationToken,
    last_activity: Mutex<Instant>,
}

struct DispatchState {
    accepting: bool,
    active: BTreeMap<DispatchId, CancellationToken>,
    pending_operator_audits: BTreeSet<DispatchId>,
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
            dispatches: Mutex::new(DispatchState {
                accepting: true,
                active: BTreeMap::new(),
                pending_operator_audits: BTreeSet::new(),
            }),
            dispatches_drained: Notify::new(),
            operator_stop_requested: AtomicBool::new(false),
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

    pub async fn try_register_dispatch(
        &self,
        dispatch_id: DispatchId,
        token: CancellationToken,
    ) -> bool {
        let mut state = self.dispatches.lock().await;
        if !state.accepting {
            token.cancel();
            return false;
        }
        state.active.insert(dispatch_id, token);
        true
    }

    pub fn request_operator_stop(&self) {
        self.operator_stop_requested.store(true, Ordering::Release);
    }

    #[must_use]
    pub fn operator_stop_requested(&self) -> bool {
        self.operator_stop_requested.load(Ordering::Acquire)
    }

    /// Keeps Stop-owned calls active until their effects finish and audit reconciliation is queued.
    pub async fn begin_finish_dispatch(&self, dispatch_id: &DispatchId) -> bool {
        let drained = {
            let mut state = self.dispatches.lock().await;
            if !state.accepting {
                return false;
            }
            state.active.remove(dispatch_id);
            state.active.is_empty()
        };
        if drained {
            self.dispatches_drained.notify_waiters();
        }
        true
    }

    pub async fn finish_interrupted_dispatch(&self, dispatch_id: &DispatchId) {
        let drained = {
            let mut state = self.dispatches.lock().await;
            state.active.remove(dispatch_id);
            if self.operator_stop_requested() {
                state.pending_operator_audits.insert(dispatch_id.clone());
            }
            state.active.is_empty()
        };
        if drained {
            self.dispatches_drained.notify_waiters();
        }
    }

    /// Returns true when completion linearized before teardown; false when teardown owns it.
    pub async fn finish_dispatch(&self, dispatch_id: &DispatchId) -> bool {
        let completed_before_stop = self.begin_finish_dispatch(dispatch_id).await;
        if !completed_before_stop {
            self.finish_interrupted_dispatch(dispatch_id).await;
        }
        completed_before_stop
    }

    pub async fn active_dispatch_count(&self) -> usize {
        self.dispatches.lock().await.active.len()
    }

    pub async fn stop_dispatches(&self) -> usize {
        let tokens = {
            let mut state = self.dispatches.lock().await;
            state.accepting = false;
            state.active.values().cloned().collect::<Vec<_>>()
        };
        for token in &tokens {
            token.cancel();
        }
        self.cancel.cancel();
        tokens.len()
    }

    pub async fn wait_for_dispatches(&self) {
        loop {
            let drained = self.dispatches_drained.notified();
            if self.dispatches.lock().await.active.is_empty() {
                return;
            }
            drained.await;
        }
    }

    pub async fn pending_operator_cancellation_audits(&self) -> Vec<DispatchId> {
        self.dispatches
            .lock()
            .await
            .pending_operator_audits
            .iter()
            .cloned()
            .collect()
    }

    pub async fn mark_operator_cancellation_audit_reconciled(&self, dispatch_id: &DispatchId) {
        self.dispatches
            .lock()
            .await
            .pending_operator_audits
            .remove(dispatch_id);
    }

    #[must_use]
    pub fn child_token(&self) -> CancellationToken {
        self.cancel.child_token()
    }
}

#[cfg(test)]
mod tests {
    use super::Session;
    use crate::{
        identity::{ClientIdentity, ConversationIdentity},
        ids::{DispatchId, SessionId},
    };
    use tokio::time::Instant;
    use tokio_util::sync::CancellationToken;

    fn test_session(id: &str) -> std::sync::Arc<Session> {
        Session::new(
            SessionId::new(id),
            ClientIdentity::Ephemeral {
                slug: "codex".to_string(),
                label: "Codex".to_string(),
            },
            ConversationIdentity::new("codex", "steady-otter".to_string()),
            "Codex".to_string(),
            Instant::now(),
        )
    }

    #[tokio::test]
    async fn stop_cancels_registered_dispatch_and_rejects_late_registration() {
        let session = test_session("session-stop");
        let active = CancellationToken::new();
        assert!(
            session
                .try_register_dispatch(DispatchId::new(), active.clone())
                .await
        );

        assert_eq!(session.stop_dispatches().await, 1);
        assert!(active.is_cancelled());
        assert!(session.child_token().is_cancelled());

        let late = CancellationToken::new();
        assert!(
            !session
                .try_register_dispatch(DispatchId::new(), late.clone())
                .await
        );
        assert!(late.is_cancelled());
    }

    #[tokio::test]
    async fn stopping_idle_session_closes_future_dispatch_admission() {
        let session = test_session("session-idle");
        assert_eq!(session.stop_dispatches().await, 0);
        let late = CancellationToken::new();
        assert!(!session.try_register_dispatch(DispatchId::new(), late).await);
    }

    #[tokio::test]
    async fn finish_linearizes_against_stop() {
        let completed = test_session("session-completed");
        let completed_id = DispatchId::new();
        assert!(
            completed
                .try_register_dispatch(completed_id.clone(), CancellationToken::new())
                .await
        );
        assert!(completed.finish_dispatch(&completed_id).await);
        assert_eq!(completed.stop_dispatches().await, 0);

        let interrupted = test_session("session-interrupted");
        let interrupted_id = DispatchId::new();
        assert!(
            interrupted
                .try_register_dispatch(interrupted_id.clone(), CancellationToken::new())
                .await
        );
        assert_eq!(interrupted.stop_dispatches().await, 1);
        assert!(!interrupted.finish_dispatch(&interrupted_id).await);
    }

    #[tokio::test]
    async fn operator_stop_keeps_late_audits_pending_until_reconciled() {
        let session = test_session("session-pending-audit");
        let dispatch_id = DispatchId::new();
        assert!(
            session
                .try_register_dispatch(dispatch_id.clone(), CancellationToken::new())
                .await
        );
        session.request_operator_stop();
        assert_eq!(session.stop_dispatches().await, 1);

        assert!(!session.begin_finish_dispatch(&dispatch_id).await);
        session.finish_interrupted_dispatch(&dispatch_id).await;
        assert_eq!(
            session.pending_operator_cancellation_audits().await,
            vec![dispatch_id.clone()]
        );
        session
            .mark_operator_cancellation_audit_reconciled(&dispatch_id)
            .await;
        assert!(
            session
                .pending_operator_cancellation_audits()
                .await
                .is_empty()
        );
    }
}
