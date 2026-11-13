//! A CDP connection for live tests that passes every call and event through to the browser's
//! own connection and keeps, for each `Input.*` call, the session it went out on: which
//! connection carried an entry's keys and text (ruling R24), and which keys a step pressed.
//!
//! It keeps the session, the method, the event's `type` and a named key (`Shift`, `Delete`),
//! never a character key or inserted text, so a recording holds no part of a value typed.

use std::mem;
use std::sync::{Arc, Mutex, PoisonError};

use browseros_cdp::{CdpError, CdpEvent};
use browseros_core::{CdpConnection, SessionId};
use futures_util::future::BoxFuture;
use serde_json::Value;
use tokio::sync::{broadcast, mpsc};

/// One `Input.*` call: the session it went out on (`None`: the browser's own), its method, the
/// event's `type` (`keyDown`, `keyUp`, ...) and its key when that key is a named one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct InputCall {
    pub(crate) session: Option<String>,
    pub(crate) method: String,
    pub(crate) kind: Option<String>,
    pub(crate) named_key: Option<String>,
}

/// The browser's connection, with every `Input.*` call kept until [`InputRecording::take`].
pub(crate) struct InputRecording {
    inner: Arc<dyn CdpConnection>,
    calls: Mutex<Vec<InputCall>>,
}

impl InputRecording {
    pub(crate) fn over(inner: Arc<dyn CdpConnection>) -> Arc<Self> {
        Arc::new(Self {
            inner,
            calls: Mutex::new(Vec::new()),
        })
    }

    /// The `Input.*` calls since the last take, in the order they went out.
    pub(crate) fn take(&self) -> Vec<InputCall> {
        mem::take(&mut *self.calls.lock().unwrap_or_else(PoisonError::into_inner))
    }

    fn keep(&self, method: &str, params: &Value, session: Option<&SessionId>) {
        if !method.starts_with("Input.") {
            return;
        }
        let text = |field: &str| params.get(field).and_then(Value::as_str).map(str::to_owned);
        let call = InputCall {
            session: session.map(|session| session.as_str().to_owned()),
            method: method.to_owned(),
            kind: text("type"),
            named_key: text("key").filter(|key| key.chars().count() > 1),
        };
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(call);
    }

    fn keep_raw(&self, method: &str, params_json: &str, session: Option<&SessionId>) {
        if method.starts_with("Input.") {
            let params = serde_json::from_str(params_json).unwrap_or(Value::Null);
            self.keep(method, &params, session);
        }
    }
}

impl CdpConnection for InputRecording {
    fn send<'a>(
        &'a self,
        method: &'a str,
        params: Value,
        session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<Value, CdpError>> {
        self.keep(method, &params, session);
        self.inner.send(method, params, session)
    }

    fn send_raw_json<'a>(
        &'a self,
        method: &'a str,
        params_json: &'a str,
        session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<String, CdpError>> {
        self.keep_raw(method, params_json, session);
        self.inner.send_raw_json(method, params_json, session)
    }

    fn send_at_epoch<'a>(
        &'a self,
        expected_epoch: u64,
        method: &'a str,
        params: Value,
        session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<Value, CdpError>> {
        self.keep(method, &params, session);
        self.inner
            .send_at_epoch(expected_epoch, method, params, session)
    }

    fn send_raw_json_at_epoch<'a>(
        &'a self,
        expected_epoch: u64,
        method: &'a str,
        params_json: &'a str,
        session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<String, CdpError>> {
        self.keep_raw(method, params_json, session);
        self.inner
            .send_raw_json_at_epoch(expected_epoch, method, params_json, session)
    }

    // Events, and their counts, are the browser connection's own: a wrapper that answered them
    // itself would break the readers that number what they took.
    fn events(&self) -> broadcast::Receiver<CdpEvent> {
        self.inner.events()
    }

    fn events_targeted(&self, method: &str) -> mpsc::UnboundedReceiver<CdpEvent> {
        self.inner.events_targeted(method)
    }

    fn events_broadcast(&self) -> u64 {
        self.inner.events_broadcast()
    }

    fn events_counted(&self) -> (broadcast::Receiver<CdpEvent>, u64) {
        self.inner.events_counted()
    }

    fn events_targeted_counted(&self, method: &str) -> (mpsc::UnboundedReceiver<CdpEvent>, u64) {
        self.inner.events_targeted_counted(method)
    }

    fn events_targeted_delivered(&self, method: &str) -> u64 {
        self.inner.events_targeted_delivered(method)
    }

    fn is_connected(&self) -> bool {
        self.inner.is_connected()
    }

    fn connection_epoch(&self) -> u64 {
        self.inner.connection_epoch()
    }
}
