//! Displays as the service's display route uses them. A test plays the viewer through
//! `Admitted::serve`; the fake Xvnc records what reaches it; the browser is core's scripted
//! connection with tabs 3 and 4 open and tab 3 in front, whose sweep reads take a moment;
//! `Privacy` is real, over a private-flag store the test can hold or fail, which commits through
//! the trace writer as the service's does, so `tab.private` reaches the front-tab watch; and
//! follow reads a real trace store and writer.
//!
//! The trace store reads through sqlx, whose pool does not run under paused time, so these tests
//! use real time. Follow's pause is re-checked each second, so its test takes a few.

use browseros_cdp::{CdpError, CdpEvent};
use browseros_core::testing::{TestCall, TestConnection, cdp_error};
use browseros_core::viewport::Viewports;
use browseros_core::{BrowserSession, BrowserSessionHooks, CdpConnection, SessionId, TabId};
use browseros_takeover::testing::{FakeXvnc, NoStamps};
use browseros_takeover::{
    Admitted, BrowserTabs, DisplayBinding, DisplayEnd, DisplayRequest, Displays, Follow, Journal,
    LockBinding, LockTtl, Locks, PartError, Reads, ScreenSize,
};
use browseros_trace::{
    Effect, EpochMillis, OperationId, OperationKind, OperationStarted, Privacy, PrivacyRecord,
    PrivacyStore, PrivateReason, RedactionLimits, Redactor, RunId, RunStarted, SecretSet,
    SecretSetConfig, TaskId, TraceError, TraceSink, TraceStore, WriterConfig, spawn_writer,
};
use bytes::Bytes;
use futures_util::future::BoxFuture;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;
use tokio::sync::{broadcast, mpsc, watch};
use tokio::task::JoinHandle;
use tokio::time::{Instant, timeout};

type BoxError = Box<dyn std::error::Error + Send + Sync>;
type TestResult = Result<(), BoxError>;

const SCREEN: ScreenSize = ScreenSize {
    width: 1600,
    height: 1000,
};

/// How long a test waits for something that should happen.
const WITHIN: Duration = Duration::from_secs(5);

/// How long a test waits before it decides something did not happen. Longer than follow's 1 s
/// tick, so a paused follower would have acted by then had it not been paused.
const QUIET: Duration = Duration::from_millis(1500);

/// Longer than two of the front-tab watch's 250 ms polls.
const POLLS: Duration = Duration::from_millis(600);

/// The front-tab watch polls every 250 ms. A view that closes this soon after a poll was closed by
/// something other than the next poll.
const BEFORE_THE_NEXT_POLL: Duration = Duration::from_millis(150);

/// The seal's read of a frame's sensitive fields, which only a sweep sends.
const SWEEP_READ: &str = "__clawSeal.sensitiveValues()";

/// How long the browser takes to answer a sweep's read.
const SWEEP_READ_TIME: Duration = Duration::from_millis(300);

/// The version, security type 1, and a ClientInit that asks for exclusive access (0).
const HANDSHAKE: &[u8] = b"RFB 003.008\n\x01\x00";
/// An update request answered at once (not incremental).
const UPDATE: [u8; 10] = [3, 0, 0, 0, 0, 0, 0, 1, 0, 1];
/// Key `a` down, as type 4 and as noVNC's QEMU key event.
const KEY_A: [u8; 8] = [4, 1, 0, 0, 0, 0, 0, 0x61];
const KEY_B: [u8; 8] = [4, 1, 0, 0, 0, 0, 0, 0x62];
const QEMU_A: [u8; 12] = [255, 0, 0, 1, 0, 0, 0, 0x61, 0, 0, 0, 0x1e];
const HOVER: [u8; 6] = [5, 0, 0, 10, 0, 20];
const CLICK: [u8; 6] = [5, 1, 0, 10, 0, 20];
const PASTE: [u8; 11] = [6, 0, 0, 0, 0, 0, 0, 3, b'p', b'w', b'd'];
const PIXEL_FORMAT: [u8; 20] = [
    0, 0, 0, 0, 32, 24, 0, 1, 0, 255, 0, 255, 0, 255, 16, 8, 0, 0, 0, 0,
];
const ENCODINGS: [u8; 12] = [2, 0, 0, 2, 0, 0, 0, 0, 0xff, 0xff, 0xfe, 0xfe];
const CONTINUOUS: [u8; 10] = [150, 1, 0, 0, 0, 0, 0x06, 0x40, 0x03, 0xe8];
const FENCE: [u8; 11] = [248, 0, 0, 0, 0x80, 0, 0, 0x03, 2, 0xaa, 0xbb];
const RESIZE: [u8; 24] = [
    251, 0, 0x01, 0x86, 0x03, 0x4c, 1, 0, 0, 0, 0, 7, 0, 0, 0, 0, 0x01, 0x86, 0x03, 0x4c, 0, 0, 0,
    0,
];

fn tab(id: i64, front: i64) -> Value {
    json!({
        "tabId": id,
        "targetId": format!("target-{id}"),
        "windowId": 1,
        "url": format!("https://example.test/{id}"),
        "title": format!("Tab {id}"),
        "isActive": id == front,
        "isLoading": false,
        "loadProgress": 1.0,
        "isPinned": false,
        "isHidden": false,
    })
}

/// A browser with `tabs` open and `front` in front, answering what core's page attach, the
/// journal, the sweep and follow send. Every frame seals; a sweep reads no values.
fn browser(tabs: &[i64], front: i64) -> Result<Arc<TestConnection>, BoxError> {
    let connection = TestConnection::new(Vec::new());
    open_tabs(&connection, tabs, front)?;
    for method in [
        "Page.enable",
        "DOM.enable",
        "Runtime.enable",
        "Accessibility.enable",
        "Runtime.runIfWaitingForDebugger",
        "Target.setAutoAttach",
        "Runtime.addBinding",
        "Page.removeScriptToEvaluateOnNewDocument",
        "Browser.activateTab",
    ] {
        connection.always(method, json!({}))?;
    }
    connection.always(
        "Page.addScriptToEvaluateOnNewDocument",
        json!({ "identifier": "journal" }),
    )?;
    connection.always(
        "Page.getFrameTree",
        json!({ "frameTree": { "frame": { "id": "main" } } }),
    )?;
    connection.always(
        "DOM.getDocument",
        json!({ "root": { "nodeName": "#document" } }),
    )?;
    connection.always(
        "Page.createIsolatedWorld",
        json!({ "executionContextId": 11 }),
    )?;
    connection.always(
        "Runtime.evaluate",
        json!({ "result": { "type": "number", "value": 1 } }),
    )?;
    Ok(connection)
}

/// From now on the browser lists `tabs`, with `front` in front, as after the agent opened or
/// activated a tab.
fn open_tabs(connection: &TestConnection, tabs: &[i64], front: i64) -> TestResult {
    let tabs = tabs.iter().map(|id| tab(*id, front)).collect::<Vec<_>>();
    connection.always("Browser.getTabs", json!({ "tabs": tabs }))?;
    Ok(())
}

/// The scripted browser, with the one reply a canned script cannot give: a sweep's read of a
/// frame's fields answers what `fields` holds, after a moment, as Chromium does. So a test can
/// tell a sweep that was waited for from one that was not.
struct Browser {
    script: Arc<TestConnection>,
    /// What every frame's password, card and code fields hold.
    fields: Mutex<Vec<String>>,
    /// The tabs a test listed with [`Browser::list_tabs`]. Once set, `Browser.activateTab`
    /// brings its tab to the front of them, as Chromium does; until then the front only moves
    /// when a test moves it.
    listed: Mutex<Option<Vec<i64>>>,
    /// How long `Browser.activateTab` takes to bring its tab forward; none unless a test slows
    /// it with [`Browser::slow_activation`].
    activation: Mutex<Duration>,
}

impl Browser {
    /// From now on the browser lists `tabs`, with `front` in front, and an activated tab comes
    /// to the front of them.
    fn list_tabs(&self, tabs: &[i64], front: i64) -> TestResult {
        *self.listed.lock().unwrap_or_else(PoisonError::into_inner) = Some(tabs.to_vec());
        open_tabs(&self.script, tabs, front)
    }

    /// From now on `Browser.activateTab` takes `delay` to bring its tab forward.
    fn slow_activation(&self, delay: Duration) {
        *self
            .activation
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = delay;
    }

    async fn answer(
        &self,
        epoch: Option<u64>,
        method: &str,
        params: Value,
        session: Option<&SessionId>,
    ) -> Result<Value, CdpError> {
        let sweep_read = method == "Runtime.evaluate" && params["expression"] == SWEEP_READ;
        let activated = (method == "Browser.activateTab")
            .then(|| params["tabId"].as_i64())
            .flatten();
        let reply = match epoch {
            Some(epoch) => {
                self.script
                    .send_at_epoch(epoch, method, params, session)
                    .await
            }
            None => self.script.send(method, params, session).await,
        }?;
        if activated.is_some() {
            let delay = *self
                .activation
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            tokio::time::sleep(delay).await;
        }
        if let Some(tab) = activated
            && let Some(tabs) = self
                .listed
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone()
            && tabs.contains(&tab)
        {
            open_tabs(&self.script, &tabs, tab).map_err(|error| CdpError::Protocol {
                code: -1,
                message: error.to_string(),
            })?;
        }
        if !sweep_read {
            return Ok(reply);
        }
        tokio::time::sleep(SWEEP_READ_TIME).await;
        let fields = self
            .fields
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        Ok(json!({ "result": { "type": "object", "value": fields } }))
    }
}

impl CdpConnection for Browser {
    fn send<'a>(
        &'a self,
        method: &'a str,
        params: Value,
        session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<Value, CdpError>> {
        Box::pin(self.answer(None, method, params, session))
    }

    fn send_raw_json<'a>(
        &'a self,
        method: &'a str,
        params_json: &'a str,
        session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<String, CdpError>> {
        self.script.send_raw_json(method, params_json, session)
    }

    fn send_at_epoch<'a>(
        &'a self,
        expected: u64,
        method: &'a str,
        params: Value,
        session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<Value, CdpError>> {
        Box::pin(self.answer(Some(expected), method, params, session))
    }

    fn send_raw_json_at_epoch<'a>(
        &'a self,
        expected: u64,
        method: &'a str,
        params_json: &'a str,
        session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<String, CdpError>> {
        self.script
            .send_raw_json_at_epoch(expected, method, params_json, session)
    }

    fn events(&self) -> broadcast::Receiver<CdpEvent> {
        self.script.events()
    }

    fn events_targeted(&self, method: &str) -> mpsc::UnboundedReceiver<CdpEvent> {
        self.script.events_targeted(method)
    }

    fn is_connected(&self) -> bool {
        true
    }

    fn connection_epoch(&self) -> u64 {
        1
    }
}

/// Private flags, committed by the trace writer, which publishes each as `tab.private`, and
/// listed in memory. A test can hold commits until it lets them through, or fail them.
struct Flags {
    writer: TraceSink,
    committed: Mutex<Vec<TabId>>,
    attempts: AtomicUsize,
    fail: AtomicBool,
    /// While true, every commit waits.
    held: watch::Sender<bool>,
}

impl Flags {
    fn new(writer: TraceSink) -> Arc<Self> {
        Arc::new(Self {
            writer,
            committed: Mutex::default(),
            attempts: AtomicUsize::new(0),
            fail: AtomicBool::new(false),
            held: watch::channel(false).0,
        })
    }

    fn committed(&self) -> Vec<TabId> {
        self.committed
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    async fn attempted(&self, count: usize) -> TestResult {
        eventually(|| self.attempts.load(Ordering::SeqCst) >= count).await
    }
}

impl PrivacyStore for Flags {
    fn persist_private(
        &self,
        tab: TabId,
        record: PrivacyRecord,
    ) -> BoxFuture<'_, Result<(), TraceError>> {
        Box::pin(async move {
            self.attempts.fetch_add(1, Ordering::SeqCst);
            let mut held = self.held.subscribe();
            held.wait_for(|held| !*held)
                .await
                .map_err(|_| TraceError::Closed)?;
            if self.fail.load(Ordering::SeqCst) {
                return Err(TraceError::Closed);
            }
            self.writer.persist_private(tab, record).await?;
            self.committed
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(tab);
            Ok(())
        })
    }

    fn forget_private(&self, _tab: TabId) -> BoxFuture<'_, Result<(), TraceError>> {
        Box::pin(async { Ok(()) })
    }
}

struct Harness {
    _dir: tempfile::TempDir,
    xvnc: FakeXvnc,
    browser: Arc<Browser>,
    flags: Arc<Flags>,
    privacy: Arc<Privacy>,
    sink: TraceSink,
    locks: Arc<Locks>,
    displays: Arc<Displays>,
}

async fn harness() -> Result<Harness, BoxError> {
    let xvnc = FakeXvnc::start(SCREEN).await?;
    let addr = xvnc.addr();
    harness_at(xvnc, addr).await
}

/// A harness whose displays dial `addr` for Xvnc.
async fn harness_at(xvnc: FakeXvnc, addr: SocketAddr) -> Result<Harness, BoxError> {
    let dir = tempfile::tempdir()?;
    let store = TraceStore::open(&dir.path().join("trace.db")).await?;
    let secrets = Arc::new(SecretSet::new(SecretSetConfig::default()));
    let sink = spawn_writer(
        &store,
        Redactor::new(Arc::clone(&secrets), RedactionLimits::default()),
        WriterConfig::default(),
        None,
    );
    let flags = Flags::new(sink.clone());
    let privacy = Privacy::new(
        Arc::clone(&secrets),
        Arc::clone(&flags) as Arc<dyn PrivacyStore>,
        HashMap::new(),
    );
    let browser = Arc::new(Browser {
        script: browser(&[3, 4], 3)?,
        fields: Mutex::default(),
        listed: Mutex::default(),
        activation: Mutex::default(),
    });
    let tabs = BrowserTabs::new(
        BrowserSession::new(
            Arc::clone(&browser) as Arc<dyn CdpConnection>,
            BrowserSessionHooks::default(),
        ),
        Arc::new(Viewports::default()),
    );
    let locks = Locks::new(store.clone(), sink.clone());
    let displays = Displays::new(
        addr,
        tabs.clone(),
        Arc::clone(&privacy),
        Arc::new(NoStamps),
        Journal::new(tabs, secrets),
        Arc::clone(&locks),
        sink.clone(),
        store,
    );
    Ok(Harness {
        _dir: dir,
        xvnc,
        browser,
        flags,
        privacy,
        sink,
        locks,
        displays,
    })
}

impl Harness {
    async fn view(&self, follow: Option<Follow>) -> Result<Viewer, BoxError> {
        Ok(serve(
            self.displays.admit(DisplayRequest::View { follow }).await?,
        ))
    }

    async fn control(&self, binding: Option<DisplayBinding>) -> Result<Viewer, BoxError> {
        Ok(serve(
            self.displays
                .admit(DisplayRequest::Control { binding })
                .await?,
        ))
    }

    fn calls(&self, method: &str) -> Result<Vec<TestCall>, BoxError> {
        Ok(self
            .browser
            .script
            .calls()?
            .into_iter()
            .filter(|call| call.method == method)
            .collect())
    }

    /// How many times anything listed the browser's tabs, the tab in front with them.
    fn tab_reads(&self) -> Result<usize, BoxError> {
        Ok(self.calls("Browser.getTabs")?.len())
    }

    /// Makes `tab` private, durably, as the host's `POST /v1/tabs/{tab}/private` does.
    async fn make_private(&self, tab: i64) -> TestResult {
        self.privacy
            .make_private(TabId(tab), PrivateReason::Host, EpochMillis::now())
            .await?;
        Ok(())
    }

    /// The sweep's reads of a frame's sensitive fields.
    fn sweeps(&self) -> Result<usize, BoxError> {
        Ok(self
            .calls("Runtime.evaluate")?
            .iter()
            .filter(|call| call.params["expression"] == SWEEP_READ)
            .count())
    }

    /// Each tab follow brought to the front, in order.
    fn activated(&self) -> Result<Vec<i64>, BoxError> {
        Ok(self
            .calls("Browser.activateTab")?
            .iter()
            .filter_map(|call| call.params["tabId"].as_i64())
            .collect())
    }

    /// Waits until connection `connection` has received `count` messages, and returns them.
    async fn received(&self, connection: usize, count: usize) -> Result<Vec<Vec<u8>>, BoxError> {
        eventually(|| self.xvnc.received(connection).len() >= count).await?;
        Ok(self.xvnc.received(connection))
    }

    /// Records an operation of task `task` on `tab`, as the dispatcher would.
    fn operation(&self, task: &TaskId, tab: i64) {
        let run = RunId::new();
        self.sink.run_started(RunStarted {
            id: run,
            code: "await page.click('#next')".into(),
            request_id: None,
            task_id: Some(task.clone()),
            host_data: None,
            at: EpochMillis::now(),
        });
        self.sink.op_started(OperationStarted {
            id: OperationId::new(run, 1),
            parent: None,
            name: "locator.click".into(),
            kind: OperationKind::Builtin,
            effect: Effect::Write,
            tab: Some(TabId(tab)),
            args: json!({ "selector": "#next" }),
            policy: Arc::default(),
            at: EpochMillis::now(),
        });
    }
}

/// A viewer's end of a display: what it sends, what reaches it, and how the display ended.
struct Viewer {
    to_display: Option<mpsc::UnboundedSender<Bytes>>,
    from_display: mpsc::UnboundedReceiver<Bytes>,
    serving: JoinHandle<DisplayEnd>,
}

fn serve(admitted: Admitted) -> Viewer {
    let (to_display, mut from_viewer) = mpsc::unbounded_channel::<Bytes>();
    let (to_viewer, from_display) = mpsc::unbounded_channel::<Bytes>();
    let from_viewer = futures_util::stream::poll_fn(move |cx| from_viewer.poll_recv(cx));
    let to_viewer = futures_util::sink::unfold(to_viewer, |to_viewer, bytes: Bytes| async move {
        to_viewer.send(bytes).map_err(|_| "the viewer is gone")?;
        Ok::<_, &'static str>(to_viewer)
    });
    Viewer {
        to_display: Some(to_display),
        from_display,
        serving: tokio::spawn(admitted.serve(from_viewer, to_viewer)),
    }
}

impl Viewer {
    fn send(&self, bytes: &[u8]) {
        if let Some(to_display) = &self.to_display {
            let _ = to_display.send(Bytes::copy_from_slice(bytes));
        }
    }

    /// The first `count` bytes Xvnc sent through the display.
    async fn read(&mut self, count: usize) -> Result<Vec<u8>, BoxError> {
        let mut bytes = Vec::new();
        while bytes.len() < count {
            let chunk = timeout(WITHIN, self.from_display.recv())
                .await?
                .ok_or("the display ended")?;
            bytes.extend_from_slice(&chunk);
        }
        Ok(bytes)
    }

    fn leave(&mut self) {
        self.to_display = None;
    }

    async fn ended(self) -> Result<DisplayEnd, BoxError> {
        Ok(timeout(WITHIN, self.serving).await??)
    }
}

/// Waits until `done` holds.
async fn eventually(done: impl Fn() -> bool) -> TestResult {
    timeout(WITHIN, async {
        while !done() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await?;
    Ok(())
}

/// The handshake as Xvnc must get it: the version, security type 1, and ClientInit 1 (shared).
fn handshake_received() -> Vec<Vec<u8>> {
    vec![b"RFB 003.008\n".to_vec(), vec![1], vec![1]]
}

fn ttl(seconds: u64) -> Result<LockTtl, &'static str> {
    LockTtl::new(Duration::from_secs(seconds)).ok_or("a TTL outside 1 s to 20 minutes")
}

#[tokio::test]
async fn a_view_display_passes_the_screen_and_drops_every_input() -> TestResult {
    let h = harness().await?;
    let mut viewer = h.view(None).await?;
    viewer.send(HANDSHAKE);
    let mut everything = Vec::new();
    for message in [
        &PIXEL_FORMAT[..],
        &ENCODINGS,
        &UPDATE,
        &KEY_A,
        &QEMU_A,
        &CLICK,
        &HOVER,
        &PASTE,
        &RESIZE,
        &CONTINUOUS,
        &FENCE,
        &UPDATE,
    ] {
        everything.extend_from_slice(message);
    }
    // Merged into one frame, then split at an odd place: the filter frames them all the same.
    let (head, tail) = everything.split_at(37);
    viewer.send(head);
    viewer.send(tail);

    let mut expected = handshake_received();
    for message in [
        &PIXEL_FORMAT[..],
        &ENCODINGS,
        &UPDATE,
        &CONTINUOUS,
        &FENCE,
        &UPDATE,
    ] {
        expected.push(message.to_vec());
    }
    assert_eq!(h.received(0, expected.len()).await?, expected);
    // Xvnc's own bytes reach the viewer as they were sent: its handshake, byte for byte.
    let mut server = b"RFB 003.008\n".to_vec();
    server.extend_from_slice(&[1, 1, 0, 0, 0, 0, 0x06, 0x40, 0x03, 0xe8]);
    server.extend_from_slice(&[32, 24, 0, 1, 0, 255, 0, 255, 0, 255, 16, 8, 0, 0, 0, 0]);
    server.extend_from_slice(&[0, 0, 0, 3, b'n', b'e', b'o']);
    let read = viewer.read(server.len()).await?;
    assert_eq!(read.get(..server.len()), Some(server.as_slice()));
    assert!(
        h.privacy.is_private(TabId(3)).is_none(),
        "a view taints nothing"
    );

    viewer.leave();
    assert_eq!(viewer.ended().await?, DisplayEnd::ViewerLeft);
    Ok(())
}

#[tokio::test]
async fn a_viewer_sending_a_message_the_filter_does_not_know_is_refused() -> TestResult {
    let h = harness().await?;
    let viewer = h.view(None).await?;
    viewer.send(HANDSHAKE);
    viewer.send(&[9]);
    assert!(matches!(viewer.ended().await?, DisplayEnd::Refused(_)));
    let control = h.control(None).await?;
    control.send(HANDSHAKE);
    control.send(&[255, 1]);
    assert!(
        matches!(control.ended().await?, DisplayEnd::Refused(_)),
        "a QEMU message other than the key event"
    );
    Ok(())
}

#[tokio::test]
async fn a_control_display_holds_the_first_key_until_every_open_tab_is_private() -> TestResult {
    let h = harness().await?;
    let control = h.control(None).await?;
    control.send(HANDSHAKE);
    control.send(&HOVER);
    assert_eq!(
        h.received(0, 4).await?[3],
        HOVER,
        "a hover move is not input"
    );
    assert!(h.privacy.is_private(TabId(3)).is_none());

    h.flags.held.send_replace(true);
    control.send(&QEMU_A);
    h.flags.attempted(1).await?;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(
        h.xvnc.received(0).len(),
        4,
        "the key waits while the private flag commits"
    );
    assert!(
        h.privacy.is_private(TabId(3)).is_some(),
        "a tab is private in memory before its commit"
    );

    h.flags.held.send_replace(false);
    assert_eq!(h.received(0, 5).await?[4], QEMU_A);
    assert_eq!(h.flags.committed(), [TabId(3), TabId(4)]);
    for tab in [3, 4] {
        let record = h
            .privacy
            .is_private(TabId(tab))
            .ok_or("the tab is not private")?;
        assert_eq!(record.reason, PrivateReason::DisplayInput);
    }
    // Later input goes at once: no new commit.
    control.send(&KEY_B);
    assert_eq!(h.received(0, 6).await?[5], KEY_B);
    assert_eq!(h.flags.attempts.load(Ordering::SeqCst), 2);
    Ok(())
}

#[tokio::test]
async fn a_failed_commit_drops_that_input_and_the_next_input_tries_again() -> TestResult {
    let h = harness().await?;
    let control = h.control(None).await?;
    control.send(HANDSHAKE);
    h.flags.fail.store(true, Ordering::SeqCst);
    control.send(&KEY_A);
    h.flags.attempted(1).await?;
    tokio::time::sleep(Duration::from_millis(100)).await;

    h.flags.fail.store(false, Ordering::SeqCst);
    control.send(&KEY_B);
    let mut expected = handshake_received();
    expected.push(KEY_B.to_vec());
    assert_eq!(
        h.received(0, 4).await?,
        expected,
        "key a never reached Xvnc"
    );
    assert_eq!(h.flags.committed(), [TabId(3), TabId(4)]);
    Ok(())
}

#[tokio::test]
async fn views_close_at_the_owners_first_input_and_stay_out_while_that_display_is_open()
-> TestResult {
    let h = harness().await?;
    let view = h.view(None).await?;
    view.send(HANDSHAKE);
    let mut control = h.control(None).await?;
    control.send(HANDSHAKE);
    control.send(&HOVER);
    control.send(&RESIZE);
    h.received(1, 5).await?;
    assert!(
        !view.serving.is_finished(),
        "a hover move and a resize are not input"
    );

    control.send(&CLICK);
    assert_eq!(view.ended().await?, DisplayEnd::Paused);
    assert!(matches!(
        h.displays
            .admit(DisplayRequest::View { follow: None })
            .await,
        Err(PartError::DisplayPaused)
    ));

    control.leave();
    assert_eq!(control.ended().await?, DisplayEnd::ViewerLeft);
    // The first input made every open tab private, for good, and tab 3 is still in front.
    assert!(matches!(
        h.displays
            .admit(DisplayRequest::View { follow: None })
            .await,
        Err(PartError::DisplayPaused)
    ));
    open_tabs(&h.browser.script, &[3, 4, 5], 5)?;
    let back = h.view(None).await?;
    back.send(HANDSHAKE);
    assert_eq!(
        h.received(2, 3).await?,
        handshake_received(),
        "views are let in again once a tab that opened after the owner left is in front"
    );
    Ok(())
}

#[tokio::test]
async fn a_views_hold_closes_open_views_and_refuses_new_ones_until_dropped() -> TestResult {
    let h = harness().await?;
    let view = h.view(None).await?;
    let hold = h.displays.hold_views_closed();
    assert_eq!(view.ended().await?, DisplayEnd::Paused);
    assert!(matches!(
        h.displays
            .admit(DisplayRequest::View { follow: None })
            .await,
        Err(PartError::DisplayPaused)
    ));
    let control = h.control(None).await?;
    assert!(
        !control.serving.is_finished(),
        "the owner's display is not a view"
    );
    drop(hold);
    h.view(None).await?;
    Ok(())
}

#[tokio::test]
async fn a_closed_binding_ends_its_display_after_the_close_sweep_of_every_tab() -> TestResult {
    let h = harness().await?;
    let binding = DisplayBinding::new();
    let control = h.control(Some(binding.clone())).await?;
    control.send(HANDSHAKE);
    control.send(&KEY_A);
    h.received(0, 4).await?;
    assert_eq!(h.sweeps()?, 0);
    // A value that reached a field with no input event the journal saw, such as autofill.
    *h.browser
        .fields
        .lock()
        .unwrap_or_else(PoisonError::into_inner) = vec!["FILLED_4b1d".to_owned()];

    timeout(WITHIN, binding.close()).await?;
    assert_eq!(
        h.sweeps()?,
        2,
        "close waits for the sweep of both open tabs"
    );
    let mut result = "the field held FILLED_4b1d".to_owned();
    assert!(
        h.privacy.secrets().mask_str(&mut result),
        "the sweep registered what the fields held before close returned"
    );
    assert!(
        !h.calls("Page.removeScriptToEvaluateOnNewDocument")?
            .is_empty(),
        "and for the journal's disarm"
    );
    assert_eq!(control.ended().await?, DisplayEnd::Released);
    assert!(binding.is_closed());
    Ok(())
}

#[tokio::test]
async fn a_display_bound_after_its_binding_closed_ends_at_once() -> TestResult {
    let h = harness().await?;
    let binding = DisplayBinding::new();
    timeout(WITHIN, binding.close()).await?;
    let control = h.control(Some(binding)).await?;
    control.send(HANDSHAKE);
    control.send(&KEY_A);
    assert_eq!(control.ended().await?, DisplayEnd::Released);
    assert!(h.xvnc.received(0).is_empty(), "nothing reached Xvnc");
    Ok(())
}

#[tokio::test]
async fn a_control_display_arms_the_journal_on_every_tab_and_on_tabs_that_open() -> TestResult {
    let h = harness().await?;
    let mut control = h.control(None).await?;
    let armed = |h: &Harness| -> Result<Vec<String>, BoxError> {
        let mut sessions = h
            .calls("Page.addScriptToEvaluateOnNewDocument")?
            .into_iter()
            .filter_map(|call| call.session.map(|session| session.0))
            .collect::<Vec<_>>();
        sessions.sort();
        sessions.dedup();
        Ok(sessions)
    };
    assert_eq!(armed(&h)?.len(), 2, "tabs 3 and 4, each on its own session");

    open_tabs(&h.browser.script, &[3, 4, 5], 3)?;
    h.displays.tab_opened(TabId(5)).await;
    assert_eq!(
        armed(&h)?.len(),
        3,
        "and tab 5, which opened while it was open"
    );

    control.leave();
    assert_eq!(control.ended().await?, DisplayEnd::ViewerLeft);
    assert_eq!(
        h.sweeps()?,
        0,
        "a display that took no input does not sweep"
    );
    let removed = h.calls("Page.removeScriptToEvaluateOnNewDocument")?.len();
    assert_eq!(removed, 3, "its end disarmed the journal on every session");

    h.displays.tab_opened(TabId(6)).await;
    assert_eq!(armed(&h)?.len(), 3, "no display is open to arm a new tab");
    Ok(())
}

#[tokio::test]
async fn a_display_ends_when_xvnc_drops_it_and_is_refused_when_xvnc_is_gone() -> TestResult {
    let h = harness().await?;
    let control = h.control(None).await?;
    control.send(HANDSHAKE);
    h.received(0, 3).await?;
    h.xvnc.drop_connections();
    assert_eq!(control.ended().await?, DisplayEnd::ScreenClosed);

    // A port whose listener is already closed.
    let gone = std::net::TcpListener::bind("127.0.0.1:0")?.local_addr()?;
    let h = harness_at(FakeXvnc::start(SCREEN).await?, gone).await?;
    for request in [
        DisplayRequest::View { follow: None },
        DisplayRequest::Control { binding: None },
    ] {
        let refused = h.displays.admit(request).await;
        assert!(matches!(refused, Err(PartError::Screen(_))));
    }
    Ok(())
}

#[tokio::test]
async fn follow_brings_a_tasks_new_tab_to_the_front_and_pauses_while_it_is_locked() -> TestResult {
    let h = harness().await?;
    let task = TaskId::parse("checkout")?;
    h.operation(&task, 3);
    h.sink.flush().await?;
    let _view = h.view(Some(Follow::Task(task.clone()))).await?;
    eventually(|| h.activated().is_ok_and(|tabs| tabs == [3])).await?;

    h.operation(&task, 4);
    eventually(|| h.activated().is_ok_and(|tabs| tabs == [3, 4])).await?;

    let lock = h
        .locks
        .lock(TabId(5), Reads::Block, ttl(60)?, LockBinding::Host)
        .await;
    h.operation(&task, 5);
    tokio::time::sleep(QUIET).await;
    assert_eq!(h.activated()?, [3, 4], "a locked tab is the owner's");

    h.locks.unlock_host(lock.id)?;
    eventually(|| h.activated().is_ok_and(|tabs| tabs == [3, 4, 5])).await?;

    let mut control = h.control(None).await?;
    h.operation(&task, 3);
    tokio::time::sleep(QUIET).await;
    assert_eq!(
        h.activated()?,
        [3, 4, 5],
        "an open control display pauses follow"
    );
    control.leave();
    control.ended().await?;
    eventually(|| h.activated().is_ok_and(|tabs| tabs == [3, 4, 5, 3])).await?;
    Ok(())
}

#[tokio::test]
async fn follow_keeps_a_tab_in_front() -> TestResult {
    let h = harness().await?;
    let _view = h.view(Some(Follow::Tab(TabId(4)))).await?;
    eventually(|| h.activated().is_ok_and(|tabs| tabs == [4])).await?;
    tokio::time::sleep(QUIET).await;
    assert_eq!(h.activated()?, [4], "once, while nothing moved it");
    Ok(())
}

#[tokio::test]
async fn a_view_is_refused_while_a_private_tab_is_in_front() -> TestResult {
    let h = harness().await?;
    h.make_private(3).await?;
    assert!(matches!(
        h.displays
            .admit(DisplayRequest::View { follow: None })
            .await,
        Err(PartError::DisplayPaused)
    ));
    h.browser
        .script
        .always("Browser.getTabs", cdp_error("Browser closed"))?;
    assert!(
        matches!(
            h.displays
                .admit(DisplayRequest::View { follow: None })
                .await,
            Err(PartError::Browser(_))
        ),
        "what is in front is unknown"
    );
    assert_eq!(h.xvnc.connections(), 0, "a refused view never reaches Xvnc");

    open_tabs(&h.browser.script, &[3, 4], 4)?;
    let view = h.view(None).await?;
    view.send(HANDSHAKE);
    assert_eq!(
        h.received(0, 3).await?,
        handshake_received(),
        "a private tab behind the one in front keeps no view out"
    );
    Ok(())
}

#[tokio::test]
async fn an_open_view_closes_as_soon_as_the_tab_in_front_goes_private() -> TestResult {
    let h = harness().await?;
    let view = h.view(None).await?;
    view.send(HANDSHAKE);
    h.received(0, 3).await?;
    // Just after a poll, so only the flag's `tab.private` can close the view this soon.
    let reads = h.tab_reads()?;
    eventually(|| h.tab_reads().is_ok_and(|now| now > reads)).await?;
    h.make_private(3).await?;
    assert_eq!(
        timeout(BEFORE_THE_NEXT_POLL, view.ended()).await??,
        DisplayEnd::Paused
    );
    assert!(matches!(
        h.displays
            .admit(DisplayRequest::View { follow: None })
            .await,
        Err(PartError::DisplayPaused)
    ));
    Ok(())
}

#[tokio::test]
async fn an_open_view_closes_when_a_private_tab_comes_to_the_front() -> TestResult {
    let h = harness().await?;
    h.make_private(4).await?;
    let view = h.view(None).await?;
    view.send(HANDSHAKE);
    h.received(0, 3).await?;

    // The agent activates tab 4. The browser sends no event for that, so the poll finds it.
    let activated = Instant::now();
    open_tabs(&h.browser.script, &[3, 4], 4)?;
    assert_eq!(view.ended().await?, DisplayEnd::Paused);
    let closed_after = activated.elapsed();
    assert!(
        closed_after < Duration::from_secs(1),
        "closed {closed_after:?} after the activation, more than a poll or so"
    );
    assert!(matches!(
        h.displays
            .admit(DisplayRequest::View { follow: None })
            .await,
        Err(PartError::DisplayPaused)
    ));

    open_tabs(&h.browser.script, &[3, 4], 3)?;
    let back = h.view(None).await?;
    back.send(HANDSHAKE);
    assert_eq!(
        h.received(1, 3).await?,
        handshake_received(),
        "views are let in again once tab 3 is back in front"
    );
    Ok(())
}

/// sdk-privacy leak test finding 4: a control display's first input makes every open tab
/// private, and the flag outlives the display. Once the tab the owner worked in closed, a
/// private tab was left in front, and a view following the agent's next tab was refused before
/// its follower could bring that tab forward. The screen stayed on the private tab, where a raw
/// control display's keys then went.
#[tokio::test]
async fn a_follow_view_brings_its_tab_forward_over_a_private_tab_left_in_front() -> TestResult {
    let h = harness().await?;
    h.browser.list_tabs(&[3, 4], 3)?;
    h.make_private(3).await?;

    let view = h.view(Some(Follow::Tab(TabId(4)))).await?;
    view.send(HANDSHAKE);
    assert_eq!(
        h.received(0, 3).await?,
        handshake_received(),
        "the view shows tab 4"
    );
    assert_eq!(h.activated()?.first(), Some(&4), "brought forward first");
    Ok(())
}

/// The front-tab watch polls every 250 ms. A view registered before its tab came forward could be
/// closed by a poll that still saw the private tab it was replacing.
#[tokio::test]
async fn a_view_whose_tab_comes_forward_slowly_is_not_closed_by_the_tab_it_replaces() -> TestResult
{
    let h = harness().await?;
    h.browser.list_tabs(&[3, 4], 3)?;
    h.browser.slow_activation(POLLS);
    h.make_private(3).await?;

    let view = h.view(Some(Follow::Tab(TabId(4)))).await?;
    view.send(HANDSHAKE);
    assert_eq!(
        h.received(0, 3).await?,
        handshake_received(),
        "the view still shows tab 4"
    );
    Ok(())
}

#[tokio::test]
async fn a_view_admitted_while_views_are_held_brings_nothing_forward() -> TestResult {
    let h = harness().await?;
    h.browser.list_tabs(&[3, 4], 3)?;
    let _hold = h.displays.hold_views_closed();
    assert!(matches!(
        h.displays
            .admit(DisplayRequest::View {
                follow: Some(Follow::Tab(TabId(4)))
            })
            .await,
        Err(PartError::DisplayPaused)
    ));
    assert!(
        h.activated()?.is_empty(),
        "a takeover holding views keeps its tab in front"
    );
    Ok(())
}

#[tokio::test]
async fn a_view_that_cannot_bring_its_tab_forward_is_still_refused_over_a_private_tab() -> TestResult
{
    let h = harness().await?;
    h.browser.list_tabs(&[3, 4], 3)?;
    h.make_private(3).await?;
    h.make_private(4).await?;
    assert!(
        matches!(
            h.displays
                .admit(DisplayRequest::View {
                    follow: Some(Follow::Tab(TabId(4)))
                })
                .await,
            Err(PartError::DisplayPaused)
        ),
        "its own tab is private: follow never brings it forward"
    );
    let task = TaskId::parse("checkout")?;
    assert!(
        matches!(
            h.displays
                .admit(DisplayRequest::View {
                    follow: Some(Follow::Task(task))
                })
                .await,
            Err(PartError::DisplayPaused)
        ),
        "a task with no operation yet names no tab to bring forward"
    );
    assert!(h.activated()?.is_empty());
    assert_eq!(h.xvnc.connections(), 0, "a refused view never reaches Xvnc");
    Ok(())
}

#[tokio::test]
async fn follow_never_brings_a_private_tab_to_the_front() -> TestResult {
    let h = harness().await?;
    h.make_private(4).await?;
    let task = TaskId::parse("checkout")?;
    let _tab_view = h.view(Some(Follow::Tab(TabId(4)))).await?;
    let _task_view = h.view(Some(Follow::Task(task.clone()))).await?;
    h.operation(&task, 4);
    tokio::time::sleep(QUIET).await;
    assert!(
        h.activated()?.is_empty(),
        "tab 4 is private: neither follower brought it forward"
    );

    h.operation(&task, 3);
    eventually(|| h.activated().is_ok_and(|tabs| tabs == [3])).await?;
    Ok(())
}

#[tokio::test]
async fn the_tab_in_front_is_read_only_while_a_view_is_open() -> TestResult {
    let h = harness().await?;
    let idle = h.tab_reads()?;
    tokio::time::sleep(POLLS).await;
    assert_eq!(h.tab_reads()?, idle, "no view, no reads");

    let mut first = h.view(None).await?;
    let mut second = h.view(None).await?;
    let opened = (h.tab_reads()?, Instant::now());
    tokio::time::sleep(Duration::from_secs(1)).await;
    let reads = h.tab_reads()? - opened.0;
    let most = usize::try_from(opened.1.elapsed().as_millis() / 250)? + 1;
    assert!(
        (2..=most).contains(&reads),
        "{reads} reads in {:?}: one 250 ms poll for both views",
        opened.1.elapsed()
    );

    first.leave();
    first.ended().await?;
    let one_left = h.tab_reads()?;
    tokio::time::sleep(POLLS).await;
    assert!(h.tab_reads()? > one_left, "a view is still open");

    second.leave();
    second.ended().await?;
    let none_left = h.tab_reads()?;
    tokio::time::sleep(POLLS).await;
    assert_eq!(
        h.tab_reads()?,
        none_left,
        "the last view closed: no more reads"
    );
    Ok(())
}

#[tokio::test]
async fn a_control_display_is_refused_when_the_tabs_cannot_be_listed() -> TestResult {
    let h = harness().await?;
    h.browser
        .script
        .always("Browser.getTabs", cdp_error("Browser closed"))?;
    assert!(
        matches!(
            h.displays
                .admit(DisplayRequest::Control { binding: None })
                .await,
            Err(PartError::Browser(_))
        ),
        "its journal could not be armed on the open tabs"
    );
    h.privacy.tab_opened(TabId(9), EpochMillis::now()).await?;
    assert!(
        h.privacy.is_private(TabId(9)).is_none(),
        "the refused display left no control display open, which would start new tabs private"
    );

    open_tabs(&h.browser.script, &[3, 4], 3)?;
    h.control(None).await?;
    Ok(())
}

#[tokio::test]
async fn a_first_input_waiting_on_a_commit_that_never_returns_ends_with_its_display() -> TestResult
{
    let h = harness().await?;
    h.flags.held.send_replace(true);
    let mut left = h.control(None).await?;
    left.send(HANDSHAKE);
    left.send(&KEY_A);
    h.flags.attempted(1).await?;
    left.leave();
    assert_eq!(left.ended().await?, DisplayEnd::ViewerLeft);

    let dropped = h.control(None).await?;
    dropped.send(HANDSHAKE);
    dropped.send(&KEY_B);
    h.flags.attempted(2).await?;
    h.xvnc.drop_connections();
    assert_eq!(dropped.ended().await?, DisplayEnd::ScreenClosed);

    for connection in [0, 1] {
        assert_eq!(
            h.received(connection, 3).await?,
            handshake_received(),
            "the waiting key never reached Xvnc"
        );
    }
    // Neither display stays to keep views out. Tab 3, which the stalled commits made private in
    // memory, is in front, so a tab that opened since comes forward.
    open_tabs(&h.browser.script, &[3, 4, 5], 5)?;
    h.view(None).await?;
    Ok(())
}

#[tokio::test]
async fn a_viewer_that_floods_while_its_first_input_waits_is_refused() -> TestResult {
    let h = harness().await?;
    h.flags.held.send_replace(true);
    let control = h.control(None).await?;
    control.send(HANDSHAKE);
    control.send(&KEY_A);
    h.flags.attempted(1).await?;
    control.send(&UPDATE.repeat(220_000));
    assert!(matches!(control.ended().await?, DisplayEnd::Refused(_)));
    Ok(())
}

#[test]
fn the_browser_hands_its_targeted_subscriber_what_its_script_emits() -> TestResult {
    let script = TestConnection::new([]);
    let browser = Browser {
        script: Arc::clone(&script),
        fields: Mutex::default(),
        listed: Mutex::default(),
        activation: Mutex::default(),
    };
    let mut bindings = browser.events_targeted("Runtime.bindingCalled");
    script.emit("Runtime.bindingCalled", json!({ "name": "probe" }), None);
    assert_eq!(bindings.try_recv()?.params["name"], "probe");
    Ok(())
}
