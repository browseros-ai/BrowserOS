//! Test adapters for the capture ports: a scripted CDP connection standing in for the browser,
//! and a session table set by the test. Used by the hub's own tests and the `/v1` route tests.

use super::ports::{CaptureTransport, SessionTabs, SessionView};
use crate::error::AppResult;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use browseros_cdp::{CdpError, CdpEvent, SessionId};
use browseros_core::CdpConnection;
use futures_util::future::BoxFuture;
use serde_json::{Value, json};
use std::{
    collections::{HashMap, VecDeque},
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::sync::{Notify, broadcast, mpsc};

/// One command the hub sent.
#[derive(Clone, Debug)]
pub struct SentCommand {
    pub method: String,
    pub params: Value,
    pub session: Option<String>,
}

/// A browser that records every command, attaches sessions `capture-1`, `capture-2`, ..., and
/// emits the frames, recording bytes and detaches the test asks for.
pub struct ScriptedCdp {
    sent: Mutex<Vec<SentCommand>>,
    sent_changed: Notify,
    sessions: AtomicUsize,
    frames: Mutex<Option<mpsc::UnboundedSender<CdpEvent>>>,
    events: broadcast::Sender<CdpEvent>,
    recordings: Mutex<HashMap<String, VecDeque<Vec<u8>>>>,
    failing_reads: Mutex<HashMap<String, usize>>,
    attach_delay: Mutex<Duration>,
}

impl ScriptedCdp {
    #[must_use]
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            sent: Mutex::default(),
            sent_changed: Notify::new(),
            sessions: AtomicUsize::new(0),
            frames: Mutex::default(),
            events: broadcast::channel(64).0,
            recordings: Mutex::default(),
            failing_reads: Mutex::default(),
            attach_delay: Mutex::new(Duration::ZERO),
        })
    }

    /// Every `method` command sent so far, in order.
    #[must_use]
    pub fn sent(&self, method: &str) -> Vec<SentCommand> {
        lock(&self.sent)
            .iter()
            .filter(|command| command.method == method)
            .cloned()
            .collect()
    }

    /// Waits (up to 5 s) until at least `count` `method` commands were sent.
    pub async fn wait_for(&self, method: &str, count: usize) -> Vec<SentCommand> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            let notified = self.sent_changed.notified();
            let sent = self.sent(method);
            if sent.len() >= count || tokio::time::Instant::now() >= deadline {
                return sent;
            }
            let _ = tokio::time::timeout_at(deadline, notified).await;
        }
    }

    /// Delivers a screencast frame on `session`, as Chromium would after a page change.
    pub fn emit_frame(&self, session: &str, jpeg: &[u8], timestamp_seconds: f64) {
        if let Some(frames) = lock(&self.frames).as_ref() {
            let _ = frames.send(CdpEvent {
                method: "Page.screencastFrame".to_string(),
                params: json!({
                    "data": BASE64.encode(jpeg),
                    "sessionId": 7,
                    "metadata": { "timestamp": timestamp_seconds },
                }),
                session_id: Some(SessionId::from(session)),
            });
        }
    }

    /// Detaches `session`, as when its tab closes.
    pub fn emit_detached(&self, session: &str) {
        let _ = self.events.send(CdpEvent {
            method: "Target.detachedFromTarget".to_string(),
            params: json!({ "sessionId": session }),
            session_id: None,
        });
    }

    /// Bytes the recording on `session` returns from its next `IO.read`.
    pub fn push_recording(&self, session: &str, bytes: &[u8]) {
        lock(&self.recordings)
            .entry(session.to_string())
            .or_default()
            .push_back(bytes.to_vec());
    }

    /// Makes every later `Target.attachToTarget` take `delay`, like a busy browser.
    pub fn delay_attaches(&self, delay: Duration) {
        *lock(&self.attach_delay) = delay;
    }

    /// Fails the next `count` `IO.read`s on `session`, as Chromium does before a recording's
    /// first bytes are written.
    pub fn fail_reads(&self, session: &str, count: usize) {
        lock(&self.failing_reads).insert(session.to_string(), count);
    }

    fn read_fails(&self, method: &str, session: Option<&SessionId>) -> bool {
        if method != "IO.read" {
            return false;
        }
        let session = session.map(|id| id.0.clone()).unwrap_or_default();
        let mut failing = lock(&self.failing_reads);
        match failing.get_mut(&session) {
            Some(left) if *left > 0 => {
                *left -= 1;
                true
            }
            _ => false,
        }
    }

    fn reply(&self, method: &str, session: Option<&SessionId>) -> Value {
        match method {
            "Target.attachToTarget" => {
                let n = self.sessions.fetch_add(1, Ordering::SeqCst) + 1;
                json!({ "sessionId": format!("capture-{n}") })
            }
            "Page.startScreenRecording" => json!({ "stream": "recording" }),
            "IO.read" => {
                let session = session.map(|id| id.0.clone()).unwrap_or_default();
                match lock(&self.recordings)
                    .get_mut(&session)
                    .and_then(VecDeque::pop_front)
                {
                    Some(bytes) => {
                        json!({ "data": BASE64.encode(bytes), "base64Encoded": true, "eof": false })
                    }
                    None => json!({ "data": "", "base64Encoded": true, "eof": true }),
                }
            }
            _ => json!({}),
        }
    }
}

impl CdpConnection for ScriptedCdp {
    fn send<'a>(
        &'a self,
        method: &'a str,
        params: Value,
        session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<Value, CdpError>> {
        Box::pin(async move {
            if method == "Target.attachToTarget" {
                let delay = *lock(&self.attach_delay);
                if !delay.is_zero() {
                    tokio::time::sleep(delay).await;
                }
            }
            let fails = self.read_fails(method, session);
            let reply = self.reply(method, session);
            lock(&self.sent).push(SentCommand {
                method: method.to_string(),
                params,
                session: session.map(|id| id.0.clone()),
            });
            self.sent_changed.notify_waiters();
            if fails {
                return Err(CdpError::Transport(format!("{method}: Read failed")));
            }
            Ok(reply)
        })
    }

    fn send_raw_json<'a>(
        &'a self,
        method: &'a str,
        _params_json: &'a str,
        _session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<String, CdpError>> {
        Box::pin(async move { Err(CdpError::Transport(format!("{method}: not scripted"))) })
    }

    fn events(&self) -> broadcast::Receiver<CdpEvent> {
        self.events.subscribe()
    }

    fn events_targeted(&self, _method: &str) -> mpsc::UnboundedReceiver<CdpEvent> {
        let (tx, rx) = mpsc::unbounded_channel();
        *lock(&self.frames) = Some(tx);
        rx
    }

    fn is_connected(&self) -> bool {
        true
    }

    fn connection_epoch(&self) -> u64 {
        1
    }
}

/// Hands the hub the scripted browser.
pub struct ScriptedTransport(pub Arc<ScriptedCdp>);

impl CaptureTransport for ScriptedTransport {
    fn connect(&self) -> BoxFuture<'_, AppResult<Arc<dyn CdpConnection>>> {
        Box::pin(async move { Ok(self.0.clone() as Arc<dyn CdpConnection>) })
    }
}

/// Sessions as the test sets them.
#[derive(Default)]
pub struct StaticSessionTabs {
    views: Mutex<HashMap<String, SessionView>>,
}

impl StaticSessionTabs {
    pub fn set(&self, session_id: &str, view: SessionView) {
        lock(&self.views).insert(session_id.to_string(), view);
    }
}

impl SessionTabs for StaticSessionTabs {
    fn view<'a>(&'a self, session_id: &'a str) -> BoxFuture<'a, AppResult<Option<SessionView>>> {
        Box::pin(async move { Ok(lock(&self.views).get(session_id).cloned()) })
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}
