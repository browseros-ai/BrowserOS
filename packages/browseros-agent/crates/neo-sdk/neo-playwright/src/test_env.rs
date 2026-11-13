//! The host side of an op for unit tests: an [`OpEnv`] that records what the crate asked of
//! it, over a browser view whose CDP connection is never there.

use std::future;
use std::sync::{Arc, Mutex, PoisonError};

use browseros_cdp::{CdpError, CdpEvent, TargetedEvents};
use browseros_core::{
    BrowserSession, BrowserSessionHooks, BrowserView, CdpConnection, ProtocolSession, SessionId,
    TabId, TargetId,
};
use futures_util::future::BoxFuture;
use serde_json::{Map, Value};
use tokio::sync::{broadcast, mpsc};

use crate::{FileAccess, OpEnv, Refusal, RunState};

/// One request the crate made of its host, in order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum EnvCall {
    Recheck,
    MarkSent,
    /// The op said it is about to send script into a page's main world.
    MainWorld,
    Note(String),
    /// The close evidence handed to the host with a delivered event.
    ReportClose(TabId, TargetId, u64),
}

/// An `OpEnv` that records every call and refuses `recheck` when built with a refusal. Needs a tokio runtime, because the browser session it holds starts its
/// listener task.
pub(crate) struct TestEnv {
    view: BrowserView,
    run_state: Arc<RunState>,
    refusal: Option<Refusal>,
    file_access: Option<FileAccess>,
    /// How many `recheck`s pass before `refusal` applies; 0 refuses from the first.
    passing_rechecks: usize,
    calls: Mutex<Vec<EnvCall>>,
    /// Each tab an op asked to bring forward (`OpEnv::bring_forward`), in order. Apart from
    /// `calls`, which tests compare whole.
    forwarded: Mutex<Vec<TabId>>,
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
            file_access: None,
            passing_rechecks: 0,
            calls: Mutex::default(),
            forwarded: Mutex::default(),
        }
    }

    pub(crate) fn refusing(refusal: Refusal) -> Self {
        Self::new().with_refusal(refusal)
    }

    pub(crate) fn with_file_access(self, file_access: FileAccess) -> Self {
        Self {
            file_access: Some(file_access),
            ..self
        }
    }

    /// The same env, refusing every `recheck` with `refusal`.
    pub(crate) fn with_refusal(self, refusal: Refusal) -> Self {
        Self {
            refusal: Some(refusal),
            ..self
        }
    }

    /// The same env, letting `passing` rechecks through and refusing every later one with
    /// `refusal`: a host that took the tab while an op was between two sends.
    pub(crate) fn with_refusal_after(self, passing: usize, refusal: Refusal) -> Self {
        Self {
            refusal: Some(refusal),
            passing_rechecks: passing,
            ..self
        }
    }

    /// What the crate asked of the host so far.
    pub(crate) fn calls(&self) -> Vec<EnvCall> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Each tab an op asked the host to bring forward, in order.
    pub(crate) fn forwarded(&self) -> Vec<TabId> {
        self.forwarded
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
    fn file_access(&self) -> Option<&FileAccess> {
        self.file_access.as_ref()
    }

    fn view(&self) -> &BrowserView {
        &self.view
    }

    fn run_state(&self) -> &Arc<RunState> {
        &self.run_state
    }

    fn recheck(&self) -> Result<(), Refusal> {
        self.record(EnvCall::Recheck);
        let rechecks = self
            .calls()
            .iter()
            .filter(|call| matches!(call, EnvCall::Recheck))
            .count();
        if rechecks <= self.passing_rechecks {
            return Ok(());
        }
        self.refusal.clone().map_or(Ok(()), Err)
    }

    fn mark_sent(&self) {
        self.record(EnvCall::MarkSent);
    }

    fn main_world(&self) {
        self.record(EnvCall::MainWorld);
    }

    fn report_close(&self, tab: TabId, target: &TargetId, seq: u64) {
        self.record(EnvCall::ReportClose(tab, target.clone(), seq));
    }

    fn note(&self, line: String) {
        self.record(EnvCall::Note(line));
    }

    fn bring_forward(&self, tab: TabId) -> BoxFuture<'_, ()> {
        self.forwarded
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(tab);
        Box::pin(future::ready(()))
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

/// A CDP connection that answers every call with `{}` and records each `(method, params)` with
/// the session it went out on, for unit tests of what the crate sends.
#[derive(Debug, Default)]
pub(crate) struct Recording {
    sent: Mutex<Vec<Call>>,
    targeted: TargetedEvents,
}

/// One recorded call: its method, its params, and its session (`None`: the browser's own).
#[derive(Clone, Debug)]
struct Call {
    method: String,
    params: Value,
    session: Option<String>,
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
        self.calls()
            .into_iter()
            .map(|call| (call.method, call.params))
            .collect()
    }

    /// Every call so far, in order, as `(method, params, session)`.
    pub(crate) fn sent_on(&self) -> Vec<(String, Value, Option<String>)> {
        self.calls()
            .into_iter()
            .map(|call| (call.method, call.params, call.session))
            .collect()
    }

    fn calls(&self) -> Vec<Call> {
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
        session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<Value, CdpError>> {
        self.sent
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Call {
                method: method.to_owned(),
                params,
                session: session.map(|session| session.as_str().to_owned()),
            });
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
