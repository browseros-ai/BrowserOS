use crate::{
    identity::{ClientIdentity, ConversationIdentity},
    ids::{ConvoId, DispatchId, SessionId},
};
use std::{collections::BTreeMap, sync::Arc, time::Duration};
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
    dispatches: Mutex<DispatchState>,
    dispatches_drained: Notify,
    cancel: CancellationToken,
    last_activity: Mutex<Instant>,
}

struct DispatchState {
    accepting: bool,
    active: BTreeMap<DispatchId, CancellationToken>,
}

impl Session {
    #[must_use]
    pub fn new(
        id: SessionId,
        agent: ClientIdentity,
        identity: ConversationIdentity,
        now: Instant,
    ) -> Arc<Self> {
        Arc::new(Self {
            id,
            agent,
            identity,
            dispatches: Mutex::new(DispatchState {
                accepting: true,
                active: BTreeMap::new(),
            }),
            dispatches_drained: Notify::new(),
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

    /// Returns true when completion linearized before Stop; false when Stop owns the result.
    pub async fn finish_dispatch(&self, dispatch_id: &DispatchId) -> bool {
        let (completed_before_stop, drained) = {
            let mut state = self.dispatches.lock().await;
            let was_active = state.active.remove(dispatch_id).is_some();
            (was_active && state.accepting, state.active.is_empty())
        };
        if drained {
            self.dispatches_drained.notify_waiters();
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
}
