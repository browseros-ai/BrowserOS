//! Neo: Test adapters for the capture ports: a scripted CDP connection standing in for the browser,
//! and a session table set by the test. Used by the hub's own tests and the `/v1` route tests.

use std::future;
use tokio::time;

use super::ports::{CaptureTransport, SessionTabs, SessionView};
use crate::{CaptureConnection, CaptureResult};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use browseros_cdp::{CdpError, CdpEvent, SessionId};
use futures_util::future::BoxFuture;
use serde_json::{Value, json};
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
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
    replies: Mutex<HashMap<String, Result<Value, CdpError>>>,
    blocked: Mutex<Vec<(String, Option<String>)>>,
    epoch: AtomicU64,
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
            replies: Mutex::default(),
            blocked: Mutex::default(),
            epoch: AtomicU64::new(1),
        })
    }

    /// Every `method` command sent so far, in order.
    #[must_use]
    pub fn sent(&self, method: &str) -> Vec<SentCommand> {
        self.sent
            .lock()
            .ok()
            .map(|sent| {
                sent.iter()
                    .filter(|command| command.method == method)
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Waits (up to 5 s) until at least `count` `method` commands were sent.
    pub async fn wait_for(&self, method: &str, count: usize) -> Vec<SentCommand> {
        let deadline = time::Instant::now() + Duration::from_secs(5);
        loop {
            let notified = self.sent_changed.notified();
            let sent = self.sent(method);
            if sent.len() >= count || time::Instant::now() >= deadline {
                return sent;
            }
            let _ = time::timeout_at(deadline, notified).await;
        }
    }

    /// Delivers a screencast frame on `session`, as Chromium would after a page change.
    pub fn emit_frame(&self, session: &str, jpeg: &[u8], timestamp_seconds: f64) {
        if let Some(frames) = self.frames.lock().ok().as_deref().and_then(Option::as_ref) {
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
        if let Ok(mut recordings) = self.recordings.lock() {
            recordings
                .entry(session.to_string())
                .or_default()
                .push_back(bytes.to_vec());
        }
    }

    /// Makes every later `Target.attachToTarget` take `delay`, like a busy browser.
    pub fn delay_attaches(&self, delay: Duration) {
        if let Ok(mut value) = self.attach_delay.lock() {
            *value = delay;
        }
    }

    /// Fails the next `count` `IO.read`s on `session`, as Chromium does before a recording's
    /// first bytes are written.
    pub fn fail_reads(&self, session: &str, count: usize) {
        if let Ok(mut reads) = self.failing_reads.lock() {
            reads.insert(session.to_string(), count);
        }
    }

    /// Overrides a command reply for host route tests, including protocol failures.
    pub fn set_reply(&self, method: &str, reply: Result<Value, CdpError>) {
        if let Ok(mut replies) = self.replies.lock() {
            replies.insert(method.to_string(), reply);
        }
    }

    /// Leaves the command unanswered, optionally for just one flat session.
    pub fn block(&self, method: &str, session: Option<&str>) {
        if let Ok(mut blocked) = self.blocked.lock() {
            blocked.push((method.to_string(), session.map(str::to_string)));
        }
    }

    /// Simulates a page socket reconnect without breaking the independent capture socket.
    pub fn reconnect(&self) {
        self.epoch.fetch_add(1, Ordering::SeqCst);
    }

    fn read_fails(&self, method: &str, session: Option<&SessionId>) -> bool {
        if method != "IO.read" {
            return false;
        }
        let session = session.map(|id| id.0.clone()).unwrap_or_default();
        let Ok(mut failing) = self.failing_reads.lock() else {
            return true;
        };
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
                match self
                    .recordings
                    .lock()
                    .ok()
                    .and_then(|mut r| r.get_mut(&session).and_then(VecDeque::pop_front))
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

impl CaptureConnection for ScriptedCdp {
    fn send<'a>(
        &'a self,
        method: &'a str,
        params: Value,
        session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<Value, CdpError>> {
        Box::pin(async move {
            if method == "Target.attachToTarget" {
                let delay = self.attach_delay.lock().map(|d| *d).unwrap_or_default();
                if !delay.is_zero() {
                    time::sleep(delay).await;
                }
            }
            let fails = self.read_fails(method, session);
            let reply = self.reply(method, session);
            self.sent
                .lock()
                .map_err(|_| CdpError::Transport("scripted command lock poisoned".into()))?
                .push(SentCommand {
                    method: method.to_string(),
                    params,
                    session: session.map(|id| id.0.clone()),
                });
            self.sent_changed.notify_waiters();
            let blocked = self.blocked.lock().ok().is_some_and(|blocked| {
                blocked.iter().any(|(m, s)| {
                    m == method
                        && s.as_ref()
                            .is_none_or(|s| session.is_some_and(|id| &id.0 == s))
                })
            });
            if blocked {
                return future::pending().await;
            }
            if let Some(reply) = self
                .replies
                .lock()
                .ok()
                .and_then(|replies| replies.get(method).cloned())
            {
                return reply;
            }
            if fails {
                return Err(CdpError::Transport(format!("{method}: Read failed")));
            }
            Ok(reply)
        })
    }

    fn events(&self) -> broadcast::Receiver<CdpEvent> {
        self.events.subscribe()
    }

    fn events_targeted(&self, _method: &str) -> mpsc::UnboundedReceiver<CdpEvent> {
        let (tx, rx) = mpsc::unbounded_channel();
        if let Ok(mut frames) = self.frames.lock() {
            *frames = Some(tx);
        }
        rx
    }

    fn is_connected(&self) -> bool {
        true
    }

    fn connection_epoch(&self) -> u64 {
        self.epoch.load(Ordering::SeqCst)
    }
}

/// Hands the hub the scripted browser.
pub struct ScriptedTransport(pub Arc<ScriptedCdp>);

impl CaptureTransport for ScriptedTransport {
    fn connect(&self) -> BoxFuture<'_, CaptureResult<Arc<dyn CaptureConnection>>> {
        Box::pin(async move { Ok(Arc::clone(&self.0) as Arc<dyn CaptureConnection>) })
    }
}

/// Sessions as the test sets them.
#[derive(Default)]
pub struct StaticSessionTabs {
    views: Mutex<HashMap<String, SessionView>>,
}

impl StaticSessionTabs {
    pub fn set(&self, session_id: &str, view: SessionView) {
        if let Ok(mut views) = self.views.lock() {
            views.insert(session_id.to_string(), view);
        }
    }
}

impl SessionTabs for StaticSessionTabs {
    fn view<'a>(
        &'a self,
        session_id: &'a str,
    ) -> BoxFuture<'a, CaptureResult<Option<SessionView>>> {
        Box::pin(async move {
            Ok(self
                .views
                .lock()
                .ok()
                .and_then(|v| v.get(session_id).cloned()))
        })
    }
}
