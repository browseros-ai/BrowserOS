//! The host side of an op for unit tests: an [`OpEnv`] that records what the crate asked of
//! it, over a browser view whose CDP connection is never there.

use std::sync::{Arc, Mutex, PoisonError};

use browseros_cdp::{CdpError, CdpEvent, TargetedEvents};
use browseros_core::{
    BrowserSession, BrowserSessionHooks, BrowserView, CdpConnection, ProtocolSession, SessionId,
};
use futures_util::future::BoxFuture;
use serde_json::{Map, Value};
use tokio::sync::{broadcast, mpsc};

use crate::{OpEnv, Refusal, RunState};

/// One request the crate made of its host, in order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum EnvCall {
    Recheck,
    MarkSent,
    MarkSensitiveTarget,
    Note(String),
}

/// An `OpEnv` that records every call and refuses `recheck` when built with a refusal. Needs a
/// tokio runtime, because the browser session it holds starts its listener task.
pub(crate) struct TestEnv {
    view: BrowserView,
    run_state: Arc<RunState>,
    refusal: Option<Refusal>,
    calls: Mutex<Vec<EnvCall>>,
}

impl TestEnv {
    pub(crate) fn new() -> Self {
        Self::over(offline_view())
    }

    /// An env whose ops admit their pages through `view`, such as an unbound view of a live
    /// browser.
    pub(crate) fn over(view: BrowserView) -> Self {
        Self {
            view,
            run_state: Arc::default(),
            refusal: None,
            calls: Mutex::default(),
        }
    }

    pub(crate) fn refusing(refusal: Refusal) -> Self {
        Self {
            refusal: Some(refusal),
            ..Self::new()
        }
    }

    /// What the crate asked of the host so far.
    pub(crate) fn calls(&self) -> Vec<EnvCall> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn record(&self, call: EnvCall) {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(call);
    }
}

impl OpEnv for TestEnv {
    fn view(&self) -> &BrowserView {
        &self.view
    }

    fn run_state(&self) -> &Arc<RunState> {
        &self.run_state
    }

    fn recheck(&self) -> Result<(), Refusal> {
        self.record(EnvCall::Recheck);
        self.refusal.clone().map_or(Ok(()), Err)
    }

    fn mark_sent(&self) {
        self.record(EnvCall::MarkSent);
    }

    fn mark_sensitive_target(&self) {
        self.record(EnvCall::MarkSensitiveTarget);
    }

    fn note(&self, line: String) {
        self.record(EnvCall::Note(line));
    }
}

/// A CDP connection that refuses every call and never delivers an event.
#[derive(Default)]
struct Offline {
    targeted: TargetedEvents,
}

impl CdpConnection for Offline {
    fn send<'a>(
        &'a self,
        _method: &'a str,
        _params: Value,
        _session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<Value, CdpError>> {
        Box::pin(async { Err(CdpError::NotConnected) })
    }

    fn send_raw_json<'a>(
        &'a self,
        _method: &'a str,
        _params_json: &'a str,
        _session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<String, CdpError>> {
        Box::pin(async { Err(CdpError::NotConnected) })
    }

    fn events(&self) -> broadcast::Receiver<CdpEvent> {
        // The sender drops at once, so every listener sees the stream closed and ends.
        broadcast::channel(1).1
    }

    fn events_targeted(&self, method: &str) -> mpsc::UnboundedReceiver<CdpEvent> {
        // Unlike the broadcast, open for as long as the connection lives, as the trait requires.
        self.targeted.subscribe(method)
    }

    fn is_connected(&self) -> bool {
        false
    }

    fn connection_epoch(&self) -> u64 {
        0
    }
}

/// A browser session over [`Offline`].
pub(crate) fn offline_session() -> Arc<BrowserSession> {
    BrowserSession::new(Arc::new(Offline::default()), BrowserSessionHooks::default())
}

fn offline_view() -> BrowserView {
    BrowserView::unbound(offline_session(), None)
}

/// A CDP connection that answers every call with `{}` and records each `(method, params)`, for
/// unit tests of what the crate sends.
#[derive(Debug, Default)]
pub(crate) struct Recording {
    sent: Mutex<Vec<(String, Value)>>,
    targeted: TargetedEvents,
}

impl Recording {
    /// A session on this connection.
    pub(crate) fn session(self: &Arc<Self>, id: &str) -> ProtocolSession {
        ProtocolSession::for_session(
            Arc::clone(self) as Arc<dyn CdpConnection>,
            SessionId::from(id),
        )
    }

    /// Every call so far, in order.
    pub(crate) fn sent(&self) -> Vec<(String, Value)> {
        self.sent
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl CdpConnection for Recording {
    fn send<'a>(
        &'a self,
        method: &'a str,
        params: Value,
        _session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<Value, CdpError>> {
        self.sent
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((method.to_owned(), params));
        Box::pin(async { Ok(Value::Object(Map::new())) })
    }

    fn send_raw_json<'a>(
        &'a self,
        _method: &'a str,
        _params_json: &'a str,
        _session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<String, CdpError>> {
        Box::pin(async { Ok("{}".to_owned()) })
    }

    fn events(&self) -> broadcast::Receiver<CdpEvent> {
        broadcast::channel(1).1
    }

    fn events_targeted(&self, method: &str) -> mpsc::UnboundedReceiver<CdpEvent> {
        self.targeted.subscribe(method)
    }

    fn is_connected(&self) -> bool {
        true
    }

    fn connection_epoch(&self) -> u64 {
        1
    }
}
