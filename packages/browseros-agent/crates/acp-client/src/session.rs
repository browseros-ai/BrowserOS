//! A turn owns its event stream until the prompt response, including after host
//! cancellation. Cancel is enqueued before waking pending permission policies;
//! the prompt future is never dropped merely because the UI stopped reading.

use std::sync::{Arc, Mutex};

use agent_client_protocol::{Agent, ConnectionTo};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::{
    AcpError, AgentConnection,
    process::lock,
    schema::{
        CancelNotification, CloseSessionRequest, ContentBlock, Meta, PromptRequest,
        SessionConfigOption, SessionId, SessionModeState, SessionUpdate,
        SetSessionConfigOptionRequest, SetSessionModeRequest, StopReason,
    },
};

/// Session metadata is the snapshot from new/load/resume. Subsequent agent
/// changes arrive as turn updates; desired configuration is owned by the caller.
pub struct Session {
    conn: AgentConnection,
    id: SessionId,
    modes: Option<SessionModeState>,
    config: Vec<SessionConfigOption>,
}

impl Session {
    pub(crate) fn new(
        conn: AgentConnection,
        id: SessionId,
        modes: Option<SessionModeState>,
        config: Vec<SessionConfigOption>,
    ) -> Self {
        Self {
            conn,
            id,
            modes,
            config,
        }
    }
    pub fn id(&self) -> &SessionId {
        &self.id
    }
    pub fn modes(&self) -> Option<&SessionModeState> {
        self.modes.as_ref()
    }
    pub fn config_options(&self) -> &[SessionConfigOption] {
        &self.config
    }

    pub async fn set_mode(&self, mode_id: &str) -> Result<(), AcpError> {
        self.conn
            .control(
                "session/set_mode",
                self.conn
                    .inner
                    .cx
                    .send_request(SetSessionModeRequest::new(
                        self.id.clone(),
                        mode_id.to_owned(),
                    ))
                    .block_task(),
            )
            .await?;
        Ok(())
    }
    pub async fn set_config_option(&self, config_id: &str, value: &str) -> Result<(), AcpError> {
        self.conn
            .control(
                "session/set_config_option",
                self.conn
                    .inner
                    .cx
                    .send_request(SetSessionConfigOptionRequest::new(
                        self.id.clone(),
                        config_id.to_owned(),
                        value,
                    ))
                    .block_task(),
            )
            .await?;
        Ok(())
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
            .send_request(PromptRequest::new(self.id.clone(), content))
            .block_task();
        let canceller = TurnCanceller {
            session_id: self.id.clone(),
            cx: self.conn.inner.cx.clone(),
            state: active.clone(),
        };
        let conn = self.conn.clone();
        let id = self.id.clone();
        tokio::spawn(async move {
            // waitpid may finish while stdout still contains the final updates
            // and response. The SDK must drain that stream before exit becomes
            // a turn error; racing the child watcher here would lose valid output.
            let result = tokio::select! {
                biased;
                () = conn.inner.state.stop.cancelled() => Err(AcpError::Closed),
                response = response => match response {
                    Ok(response) => Ok(TurnOutcome { stop_reason: response.stop_reason, meta: response.meta }),
                    Err(error) => Err(conn.map_error(error).await),
                },
            };
            conn.inner
                .state
                .routes
                .finish(&id, &active, TurnEvent::Done(result));
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
                        .send_request(CloseSessionRequest::new(self.id.clone()))
                        .block_task(),
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
