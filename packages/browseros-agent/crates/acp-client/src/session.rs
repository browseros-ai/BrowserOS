//! A turn owns its event stream until the prompt response, including after host
//! cancellation. Cancel is enqueued before waking pending permission policies;
//! the prompt future is never dropped merely because the UI stopped reading.

use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use agent_client_protocol::{Agent, ConnectionTo};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::{
    AcpError, AgentConnection,
    connection::exited,
    process::lock,
    schema::{
        CancelNotification, CloseSessionRequest, ContentBlock, Meta, PromptRequest,
        SessionConfigOption, SessionId, SessionModeState, SessionUpdate,
        SetSessionConfigOptionRequest, SetSessionModeRequest, StopReason,
    },
};

/// One session lease with control metadata shared by every alias. Dispatch
/// refreshes the snapshot even between turns; transcript updates remain scoped
/// to the active turn and load replay never enters a later conversation.
pub struct Session {
    conn: AgentConnection,
    id: SessionId,
    snapshot: Arc<Mutex<SessionSnapshot>>,
}

/// Mutable control state lives independently of handles so notification dispatch
/// can update every alias without extending the public connection lifetime.
#[derive(Default)]
pub(crate) struct SessionSnapshot {
    pub(crate) modes: Option<SessionModeState>,
    pub(crate) config: Vec<SessionConfigOption>,
}

impl Session {
    pub(crate) fn new(
        conn: AgentConnection,
        id: SessionId,
        snapshot: Arc<Mutex<SessionSnapshot>>,
    ) -> Self {
        Self { conn, id, snapshot }
    }
    pub fn id(&self) -> &SessionId {
        &self.id
    }
    /// Clone the latest control state without retaining a dispatch lock.
    pub fn modes(&self) -> Option<SessionModeState> {
        lock(&self.snapshot).modes.clone()
    }
    /// Clone the latest full option list, including changes made between turns.
    pub fn config_options(&self) -> Vec<SessionConfigOption> {
        lock(&self.snapshot).config.clone()
    }

    pub async fn set_mode(&self, mode_id: &str) -> Result<(), AcpError> {
        self.conn
            .control(
                "session/set_mode",
                self.conn.inner.cx.send_request(SetSessionModeRequest::new(
                    self.id.clone(),
                    mode_id.to_owned(),
                )),
            )
            .await?;
        Ok(())
    }
    pub async fn set_config_option(
        &self,
        config_id: &str,
        value: &str,
    ) -> Result<Vec<SessionConfigOption>, AcpError> {
        let snapshot = self.snapshot.clone();
        self.conn
            .control_ordered(
                "session/set_config_option",
                self.conn
                    .inner
                    .cx
                    .prepare_request(SetSessionConfigOptionRequest::new(
                        self.id.clone(),
                        config_id.to_owned(),
                        value,
                    )),
                move |response| {
                    lock(&snapshot).config = response.config_options.clone();
                    response.config_options
                },
            )
            .await
    }

    pub fn prompt(&self, content: Vec<ContentBlock>) -> Result<Turn, AcpError> {
        self.conn.available()?;
        let (tx, events) = mpsc::unbounded_channel();
        let active = self.conn.inner.state.routes.start(&self.id, tx)?;
        // send_request publishes synchronously. Even an immediate cancel from
        // the caller is therefore ordered AFTER this prompt on the wire.
        let response = self
            .conn
            .inner
            .cx
            .send_request(PromptRequest::new(self.id.clone(), content));
        let canceller = TurnCanceller {
            session_id: self.id.clone(),
            cx: self.conn.inner.cx.clone(),
            state: active.clone(),
        };
        let state = self.conn.inner.state.clone();
        let auth = self.conn.auth_methods();
        let id = self.id.clone();
        tokio::spawn(async move {
            let response = state.response(&auth, response);
            tokio::pin!(response);
            // Child exit and stdout EOF are independent: a descendant can keep
            // stdout open forever. Allow buffered final output to drain, then
            // terminate with the supervisor's status and captured diagnostics.
            let result = tokio::select! {
                biased;
                () = state.stop.cancelled() => Err(state.stopped_error()),
                result = &mut response => result,
                report = state.process.closed() => {
                    tokio::select! {
                        biased;
                        () = state.stop.cancelled() => Err(state.stopped_error()),
                        result = tokio::time::timeout(Duration::from_secs(1), &mut response) => result.unwrap_or_else(|_| Err(exited(report))),
                    }
                }
            }.map(|response| TurnOutcome { stop_reason: response.stop_reason, meta: response.meta });
            state.routes.finish(&id, &active, TurnEvent::Done(result));
        });
        Ok(Turn {
            events,
            canceller,
            done: false,
        })
    }

    pub async fn close(&self) -> Result<(), AcpError> {
        self.conn.inner.state.routes.close(&self.id)?;
        if self.conn.features().close_session {
            self.conn
                .control(
                    "session/close",
                    self.conn
                        .inner
                        .cx
                        .send_request(CloseSessionRequest::new(self.id.clone())),
                )
                .await?;
        }
        Ok(())
    }
}

#[derive(Debug)]
#[allow(
    clippy::large_enum_variant,
    reason = "The agreed public interface carries schema updates by value without an extra allocation per chunk"
)]
pub enum TurnEvent {
    Update(SessionUpdate),
    /// Exactly one terminal event; no updates follow it on this stream.
    Done(Result<TurnOutcome, AcpError>),
}

#[derive(Debug, Clone)]
pub struct TurnOutcome {
    pub stop_reason: StopReason,
    pub meta: Option<Meta>,
}

/// Unbounded event delivery keeps SDK dispatch independent of UI backpressure.
/// Dropping the stream cancels the turn but its background task still drains.
pub struct Turn {
    events: mpsc::UnboundedReceiver<TurnEvent>,
    canceller: TurnCanceller,
    done: bool,
}
impl Turn {
    pub async fn next(&mut self) -> Option<TurnEvent> {
        if self.done {
            return None;
        }
        let event = self.events.recv().await;
        if matches!(event, Some(TurnEvent::Done(_)) | None) {
            self.done = true;
        }
        event
    }
    pub fn canceller(&self) -> TurnCanceller {
        self.canceller.clone()
    }
}
impl Drop for Turn {
    fn drop(&mut self) {
        self.canceller.cancel();
    }
}

#[derive(Default)]
struct Lifecycle {
    finished: bool,
    cancel_sent: bool,
}

/// Shared by the route, prompt task, cancellers, and permission responders. Its
/// lifecycle lock orders cancel/approval/finish without holding the dispatch loop.
pub(crate) struct TurnState {
    pub(crate) tx: mpsc::UnboundedSender<TurnEvent>,
    pub(crate) permission_stop: CancellationToken,
    lifecycle: Mutex<Lifecycle>,
}
impl TurnState {
    pub(crate) fn new(tx: mpsc::UnboundedSender<TurnEvent>) -> Self {
        Self {
            tx,
            permission_stop: CancellationToken::new(),
            lifecycle: Mutex::default(),
        }
    }
    /// Serialize response publication with cancel, so a policy resolving on
    /// another executor cannot send an approval AFTER session/cancel.
    pub(crate) fn permission_response(
        &self,
        request: &crate::schema::RequestPermissionRequest,
        decision: crate::PermissionDecision,
        responder: agent_client_protocol::Responder<crate::schema::RequestPermissionResponse>,
    ) -> Result<(), crate::schema::Error> {
        let lifecycle = lock(&self.lifecycle);
        let decision = if lifecycle.cancel_sent || lifecycle.finished {
            crate::PermissionDecision::Cancel
        } else {
            decision
        };
        responder.respond(crate::schema::RequestPermissionResponse::new(
            crate::permissions::outcome(request, decision),
        ))
    }

    pub(crate) fn finish(&self) {
        lock(&self.lifecycle).finished = true;
        self.permission_stop.cancel();
    }
}

/// Idempotent even across clones. The lifecycle lock prevents a stale canceller
/// from sending a session-wide cancel after its turn has finished and a new one
/// has started; cancellation leaves the prompt receiver alive to drain updates.
#[derive(Clone)]
pub struct TurnCanceller {
    session_id: SessionId,
    cx: ConnectionTo<Agent>,
    state: Arc<TurnState>,
}
impl TurnCanceller {
    pub fn cancel(&self) {
        let mut lifecycle = lock(&self.state.lifecycle);
        if lifecycle.finished || lifecycle.cancel_sent {
            return;
        }
        lifecycle.cancel_sent = true;
        if let Err(error) = self
            .cx
            .send_notification(CancelNotification::new(self.session_id.clone()))
        {
            tracing::debug!(%error, "Unable to send ACP cancellation on closed connection");
        }
        self.state.permission_stop.cancel();
    }
}
