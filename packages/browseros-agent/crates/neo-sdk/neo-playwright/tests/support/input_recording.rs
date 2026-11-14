//! A CDP connection for live tests that passes every call and event through to the browser's
//! own connection and keeps, for each `Input.*` call, the session it went out on: which
//! connection carried an entry's keys and text (ruling R24), and which keys a step pressed.
//!
//! It keeps the session, the method, the event's `type` and a named key (`Shift`, `Delete`),
//! never a character key or inserted text, so a recording holds no part of a value typed.
//!
//! While a test names a text ([`InputRecording::watch_for`]), it also keeps the session of every
//! command, of any method, whose params carry that text: where a host's value went, and where it
//! never went (ruling R48). Only the session is kept, never the params.
//!
//! It also fails one method a test names, refused or left hanging, from its first call or from
//! its Nth, and counts every method's calls, so a row can show what a check answers when a read
//! of the browser fails, the first read or only the second (ruling R63).

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::{future, mem};

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

/// How a command a test names fails ([`InputRecording::fail`]).
#[derive(Clone, Copy, Debug)]
pub(crate) enum Failing {
    /// Answered at once with a protocol error, as Chromium refuses a command.
    Refuse,
    /// Never answered, as a renderer that does not get to it.
    Hang,
}

/// The browser's connection, with every `Input.*` call kept until [`InputRecording::take`].
pub(crate) struct InputRecording {
    inner: Arc<dyn CdpConnection>,
    calls: Mutex<Vec<InputCall>>,
    /// The text a test watches for, and the session of each command that carried it since.
    watched: Mutex<Option<(String, Vec<Option<String>>)>>,
    /// The method a test makes fail, and how.
    failing: Mutex<Option<Fault>>,
    /// How many times each method was sent, since the last [`InputRecording::reset_calls`].
    counts: Mutex<HashMap<String, usize>>,
}

/// A method a test makes fail, how, and from which of its calls since the fault was set.
struct Fault {
    method: String,
    how: Failing,
    /// The first call to fail, counted from 1.
    from: usize,
    /// The calls of `method` since the fault was set.
    seen: usize,
}

impl InputRecording {
    pub(crate) fn over(inner: Arc<dyn CdpConnection>) -> Arc<Self> {
        Arc::new(Self {
            inner,
            calls: Mutex::new(Vec::new()),
            watched: Mutex::new(None),
            failing: Mutex::new(None),
            counts: Mutex::new(HashMap::new()),
        })
    }

    /// From now on, every `method` command fails `how`, until [`Self::stop_failing`].
    pub(crate) fn fail(&self, method: &str, how: Failing) {
        self.fail_from(method, how, 1);
    }

    /// From now on, `method` goes through for its first `from - 1` calls and then fails `how` at
    /// every call, until [`Self::stop_failing`].
    pub(crate) fn fail_from(&self, method: &str, how: Failing, from: usize) {
        *self.failing.lock().unwrap_or_else(PoisonError::into_inner) = Some(Fault {
            method: method.to_owned(),
            how,
            from,
            seen: 0,
        });
    }

    /// How many times `method` was sent since the last [`Self::reset_calls`].
    pub(crate) fn calls(&self, method: &str) -> usize {
        self.counts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(method)
            .copied()
            .unwrap_or(0)
    }

    /// Counts every method's calls from zero again.
    pub(crate) fn reset_calls(&self) {
        self.counts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clear();
    }

    /// Commands go through again.
    pub(crate) fn stop_failing(&self) {
        *self.failing.lock().unwrap_or_else(PoisonError::into_inner) = None;
    }

    /// How `method` must fail, if a test made it.
    fn note(&self, method: &str) -> Option<Failing> {
        *self
            .counts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .entry(method.to_owned())
            .or_default() += 1;
        let mut failing = self.failing.lock().unwrap_or_else(PoisonError::into_inner);
        let fault = failing.as_mut().filter(|fault| fault.method == method)?;
        fault.seen += 1;
        (fault.seen >= fault.from).then_some(fault.how)
    }

    /// From now on, keeps the session of every command whose params carry `text`.
    pub(crate) fn watch_for(&self, text: &str) {
        *self.watched.lock().unwrap_or_else(PoisonError::into_inner) =
            Some((text.to_owned(), Vec::new()));
    }

    /// The session of each command that carried the watched text since the last call (`None`:
    /// the browser's own), in the order they went out.
    pub(crate) fn take_carriers(&self) -> Vec<Option<String>> {
        self.watched
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_mut()
            .map(|(_, carriers)| mem::take(carriers))
            .unwrap_or_default()
    }

    fn spot(&self, params: impl FnOnce() -> String, session: Option<&SessionId>) {
        let mut watched = self.watched.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some((text, carriers)) = watched.as_mut()
            && params().contains(text.as_str())
        {
            carriers.push(session.map(|session| session.as_str().to_owned()));
        }
    }

    /// The `Input.*` calls since the last take, in the order they went out.
    pub(crate) fn take(&self) -> Vec<InputCall> {
        mem::take(&mut *self.calls.lock().unwrap_or_else(PoisonError::into_inner))
    }

    fn keep(&self, method: &str, params: &Value, session: Option<&SessionId>) {
        self.spot(|| params.to_string(), session);
        self.keep_input(method, params, session);
    }

    fn keep_input(&self, method: &str, params: &Value, session: Option<&SessionId>) {
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
        self.spot(|| params_json.to_owned(), session);
        if method.starts_with("Input.") {
            let params = serde_json::from_str(params_json).unwrap_or(Value::Null);
            self.keep_input(method, &params, session);
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
        if let Some(how) = self.note(method) {
            return failed(how);
        }
        self.inner.send(method, params, session)
    }

    fn send_raw_json<'a>(
        &'a self,
        method: &'a str,
        params_json: &'a str,
        session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<String, CdpError>> {
        self.keep_raw(method, params_json, session);
        if let Some(how) = self.note(method) {
            return failed(how);
        }
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
        if let Some(how) = self.note(method) {
            return failed(how);
        }
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
        if let Some(how) = self.note(method) {
            return failed(how);
        }
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

/// A command's answer when a test makes it fail `how`.
fn failed<'a, T: Send + 'a>(how: Failing) -> BoxFuture<'a, Result<T, CdpError>> {
    match how {
        Failing::Refuse => Box::pin(future::ready(Err(CdpError::Protocol {
            code: -32000,
            message: "refused by the test".to_owned(),
        }))),
        Failing::Hang => Box::pin(future::pending()),
    }
}
