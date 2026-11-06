//! Displays as the service's display route uses them. A test plays the viewer through
//! `Admitted::serve`; the fake Xvnc records what reaches it; the browser is core's scripted
//! connection with tabs 3 and 4 open and tab 3 in front, whose sweep reads take a moment;
//! `Privacy` is real, over a private-flag store the test can hold or fail, which commits through
//! the trace writer as the service's does, so `tab.private` reaches the front-tab watch; and
//! follow reads a real trace store and writer. A shared log orders what the test's commit and the
//! fake browser saw: each activation is logged as it lands. A test can hold activations in
//! flight, neither landed nor answered, and check a step meanwhile, so what the fake browser
//! received orders the test, not a delay.
//!
//! The trace store reads through sqlx, whose pool does not run under paused time, so these tests
//! use real time. Follow's pause is re-checked each second, so its test takes a few.

use browseros_cdp::{CdpError, CdpEvent};
use browseros_core::testing::{TestCall, TestConnection, cdp_error};
use browseros_core::viewport::Viewports;
use browseros_core::{BrowserSession, BrowserSessionHooks, CdpConnection, SessionId, TabId};
use browseros_takeover::testing::{FakeXvnc, NoStamps};
use browseros_takeover::{
    Admitted, BrowserTabs, DisplayBinding, DisplayEnd, DisplayRequest, Displays, Follow, Front,
    FrontError, Journal, LockBinding, LockTtl, Locks, OwnerHasScreen, PartError, Reads,
    ScreenHolder, ScreenSize,
};
use browseros_trace::{
    Delivery, Effect, EpochMillis, OperationEnded, OperationId, OperationKind, OperationStarted,
    Outcome, Privacy, PrivacyRecord, PrivacyStore, PrivateReason, RedactionLimits, Redactor, RunId,
    RunStarted, SecretSet, SecretSetConfig, TargetSensitivity, TaskId, TraceError, TraceSink,
    TraceStore, WriterConfig, spawn_writer,
};
use bytes::Bytes;
use futures_util::FutureExt;
use futures_util::future::BoxFuture;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::pin::{Pin, pin};
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
/// An update request Xvnc holds until the screen changes (incremental).
const INCREMENTAL: [u8; 10] = [3, 1, 0, 0, 0, 0, 0, 1, 0, 1];
/// What Xvnc sends before any message: its version, security type None, SecurityResult 0, and
/// ServerInit (1600×1000, 32 bits per pixel, the name "neo").
const SERVER_HANDSHAKE: usize = 12 + 2 + 4 + 24 + 3;
/// The pixel the screen shows once the private tab 4 is in front.
const PRIVATE_PX: [u8; 4] = [0xde, 0xad, 0xbe, 0xef];
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

/// What happened, in order: the test's commits and each activation as it lands.
#[derive(Clone, Default)]
struct Log(Arc<Mutex<Vec<String>>>);

impl Log {
    fn push(&self, entry: impl Into<String>) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(entry.into());
    }

    fn entries(&self) -> Vec<String> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

/// The screen a tab shows once it is in front.
struct Paint {
    tab: i64,
    pixel: [u8; 4],
    xvnc: Arc<FakeXvnc>,
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
    /// `false` while a test holds activations: each `Browser.activateTab` then waits,
    /// subscribed, before it lands and answers, until [`Browser::let_activations_go`]. So a
    /// test checks a step while an activation is in flight, ordered by what the browser got.
    activations: watch::Sender<bool>,
    /// Logs each activation as it lands.
    log: Log,
    /// What the screen paints once a tab comes forward, when a test set it.
    paint: Mutex<Option<Paint>>,
}

impl Browser {
    fn new(script: Arc<TestConnection>) -> Self {
        Self {
            script,
            fields: Mutex::default(),
            listed: Mutex::default(),
            activations: watch::channel(true).0,
            log: Log::default(),
            paint: Mutex::default(),
        }
    }

    /// From now on the browser lists `tabs`, with `front` in front, and an activated tab comes
    /// to the front of them.
    fn list_tabs(&self, tabs: &[i64], front: i64) -> TestResult {
        *self.listed.lock().unwrap_or_else(PoisonError::into_inner) = Some(tabs.to_vec());
        open_tabs(&self.script, tabs, front)
    }

    /// Every `Browser.activateTab` from now on waits, its tab not forward and its answer not
    /// sent, until [`Browser::let_activations_go`].
    fn hold_activations(&self) {
        self.activations.send_replace(false);
    }

    /// Lands and answers every held activation, and every later one.
    fn let_activations_go(&self) {
        self.activations.send_replace(true);
    }

    /// How many activations are waiting to land.
    fn held_activations(&self) -> usize {
        self.activations.receiver_count()
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
        if let Some(tab) = activated {
            let _ = self.activations.subscribe().wait_for(|open| *open).await;
            self.log.push(format!("activate {tab}"));
            if let Some(paint) = &*self.paint.lock().unwrap_or_else(PoisonError::into_inner)
                && paint.tab == tab
            {
                paint.xvnc.paint(paint.pixel);
            }
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
    xvnc: Arc<FakeXvnc>,
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
    let browser = Arc::new(Browser::new(browser(&[3, 4], 3)?));
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
        xvnc: Arc::new(xvnc),
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
        Ok(serve(self.admit_view(follow).await?))
    }

    /// A view admitted and not yet served. Its follower has subscribed to its task's operations
    /// and to Neo's moves, and reads neither until [`serve`] runs it.
    async fn admit_view(&self, follow: Option<Follow>) -> Result<Admitted, BoxError> {
        Ok(self.displays.admit(DisplayRequest::View { follow }).await?)
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

    /// What happened so far, in order.
    fn log(&self) -> Vec<String> {
        self.browser.log.entries()
    }

    /// From now on the screen paints `pixel` once `tab` comes forward.
    fn paint_when_in_front(&self, tab: i64, pixel: [u8; 4]) {
        *self
            .browser
            .paint
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(Paint {
            tab,
            pixel,
            xvnc: Arc::clone(&self.xvnc),
        });
    }

    fn front(&self) -> Front {
        self.displays.front()
    }

    /// The agent's `page.bringToFront()` on `tab` for `task`, whose commit is logged and passes.
    async fn bring_to_front(&self, tab: i64, task: Option<&TaskId>) -> Result<(), FrontError<()>> {
        let log = &self.browser.log;
        self.front()
            .bring_to_front(TabId(tab), task.cloned(), || {
                log.push("commit");
                Ok(())
            })
            .await
    }

    /// Whether a new view is refused now. Admission refuses a paused view before it awaits
    /// anything, so this answers synchronously, as a commit must.
    fn views_refused(&self) -> bool {
        matches!(
            self.displays
                .admit(DisplayRequest::View { follow: None })
                .now_or_never(),
            Some(Err(PartError::DisplayPaused))
        )
    }

    /// Whether a new view is refused once its admission has read what is in front, as with a
    /// private tab there (decision 34).
    async fn front_refuses_views(&self) -> bool {
        matches!(
            self.displays
                .admit(DisplayRequest::View { follow: None })
                .await,
            Err(PartError::DisplayPaused)
        )
    }

    /// From now on `Browser.activateTab` answers an error, as when it gave no answer in time.
    fn fail_activations(&self) -> TestResult {
        self.browser
            .script
            .always("Browser.activateTab", cdp_error("no answer in time"))?;
        Ok(())
    }

    /// Waits until the browser holds the activation of `tab`: sent, neither landed nor answered.
    async fn activation_held(&self, tab: i64) -> TestResult {
        eventually(|| {
            self.browser.held_activations() == 1
                && self.activated().is_ok_and(|tabs| tabs.last() == Some(&tab))
        })
        .await
    }

    /// Polls the agent's move by hand until it is past its activation's answer: it returned, or
    /// it asked the browser what is in front. Left unpolled, it keeps what it holds, its private
    /// switch included, until the test polls it again.
    async fn poll_past_the_activation(&self, mut moving: Pin<&mut impl Future>) -> TestResult {
        timeout(WITHIN, async {
            while moving.as_mut().now_or_never().is_none()
                && !self
                    .tab_reads_since_activation()
                    .is_ok_and(|reads| reads > 0)
            {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await?;
        Ok(())
    }

    /// How many times the browser listed its tabs since it was last asked to bring one forward.
    fn tab_reads_since_activation(&self) -> Result<usize, BoxError> {
        let calls = self.browser.script.calls()?;
        let since = calls
            .iter()
            .rposition(|call| call.method == "Browser.activateTab")
            .ok_or("no activation yet")?;
        Ok(calls[since..]
            .iter()
            .filter(|call| call.method == "Browser.getTabs")
            .count())
    }

    /// Each tab the browser was asked to bring to the front, in order.
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

    /// Records an operation of task `task` on `tab` that starts now, as the dispatcher would.
    fn operation(&self, task: &TaskId, tab: i64) {
        self.operation_at(task, tab, EpochMillis::now());
    }

    /// Records an operation of task `task` on `tab` that started at `at`. The dispatcher stamps
    /// an operation as it admits it, and the writer publishes its start only after its commit,
    /// so a follower can read an operation after a move that came later.
    fn operation_at(&self, task: &TaskId, tab: i64, at: EpochMillis) {
        let run = RunId::new();
        self.sink.run_started(run_of(run, task, at));
        let click = first_operation(run, "locator.click", Effect::Write, Some(TabId(tab)), at);
        self.sink.op_started(OperationStarted {
            args: json!({ "selector": "#next" }),
            ..click
        });
    }

    /// Records an operation of task `task` with no page, started and ended at `at`: a
    /// context-level call, or one refused before it had a page. Its end names no tab.
    fn operation_with_no_page(&self, task: &TaskId, at: EpochMillis) {
        let run = RunId::new();
        self.sink.run_started(run_of(run, task, at));
        let cookies = first_operation(run, "context.cookies", Effect::Read, None, at);
        let id = cookies.id;
        self.sink.op_started(cookies);
        self.sink.op_ended(OperationEnded {
            id,
            outcome: Outcome::Ok,
            delivery: Delivery::Done,
            sent_at: Some(at),
            result: None,
            error: None,
            page_after: None,
            target: TargetSensitivity::Ordinary,
            also_mask: None,
            at,
        });
    }

    /// Commits more trace events than the writer's event stream holds (1024), so a subscriber
    /// that reads none of them meanwhile has lost some.
    async fn overflow_the_event_stream(&self) -> TestResult {
        for _ in 0..1100 {
            self.sink.run_started(RunStarted {
                id: RunId::new(),
                code: "await page.title()".into(),
                request_id: None,
                task_id: None,
                host_data: None,
                also_mask: None,
                at: EpochMillis::now(),
            });
        }
        self.sink.flush().await?;
        Ok(())
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

    /// Every byte that reaches the viewer until its display ends.
    async fn until_the_end(&mut self) -> Result<Vec<u8>, BoxError> {
        let mut bytes = Vec::new();
        while let Some(chunk) = timeout(WITHIN, self.from_display.recv()).await? {
            bytes.extend_from_slice(&chunk);
        }
        Ok(bytes)
    }

    async fn ended(self) -> Result<DisplayEnd, BoxError> {
        Ok(timeout(WITHIN, self.serving).await??)
    }
}

/// Waits until `done` holds.
/// A run of task `task`, started at `at`.
fn run_of(run: RunId, task: &TaskId, at: EpochMillis) -> RunStarted {
    RunStarted {
        id: run,
        code: "await page.click('#next')".into(),
        request_id: None,
        task_id: Some(task.clone()),
        host_data: None,
        also_mask: None,
        at,
    }
}

/// `run`'s first operation, `name` on `tab`, admitted at `at`.
fn first_operation(
    run: RunId,
    name: &str,
    effect: Effect,
    tab: Option<TabId>,
    at: EpochMillis,
) -> OperationStarted {
    OperationStarted {
        id: OperationId::new(run, 1),
        parent: None,
        name: name.into(),
        kind: OperationKind::Builtin,
        effect,
        tab,
        args: json!({}),
        policy: Arc::default(),
        also_mask: None,
        at,
    }
}

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
async fn a_screen_hold_closes_open_views_and_refuses_new_ones_until_dropped() -> TestResult {
    let h = harness().await?;
    let view = h.view(None).await?;
    let hold = h.displays.hold_screen();
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
    h.make_private(3).await?;
    h.browser.hold_activations();
    let admitting = tokio::spawn({
        let displays = Arc::clone(&h.displays);
        async move {
            displays
                .admit(DisplayRequest::View {
                    follow: Some(Follow::Tab(TabId(4))),
                })
                .await
        }
    });
    h.activation_held(4).await?;
    // In flight for two of the watch's polls, which would close a view already registered.
    tokio::time::sleep(POLLS).await;
    h.browser.let_activations_go();

    let view = serve(timeout(WITHIN, admitting).await???);
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
    let _hold = h.displays.hold_screen();
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
    let browser = Browser::new(Arc::clone(&script));
    let mut bindings = browser.events_targeted("Runtime.bindingCalled");
    script.emit("Runtime.bindingCalled", json!({ "name": "probe" }), None);
    assert_eq!(bindings.try_recv()?.params["name"], "probe");
    Ok(())
}

/// The agent brings the private tab 4 forward under a plain view and a follow view. Its commit
/// (the door's recheck, then mark_sent) already finds new views refused: the switch that refuses
/// them cancelled every open view's close in the same step. The activation comes after the
/// commit, and both views end.
#[tokio::test]
async fn a_private_tab_comes_forward_only_after_every_views_close() -> TestResult {
    let h = harness().await?;
    h.browser.list_tabs(&[3, 4], 3)?;
    h.make_private(4).await?;
    let task = TaskId::parse("checkout")?;
    h.operation(&task, 3);
    h.sink.flush().await?;
    let plain = h.view(None).await?;
    let follower = h.view(Some(Follow::Task(task))).await?;

    let log = &h.browser.log;
    h.front()
        .bring_to_front(TabId(4), None, || {
            if h.views_refused() {
                log.push("views refused");
            }
            log.push("commit");
            Ok::<_, ()>(())
        })
        .await?;
    assert_eq!(
        h.log(),
        ["activate 3", "views refused", "commit", "activate 4"],
        "the follow view brought 3 forward as it opened; then views were out before the commit, \
         and the activation came after it"
    );
    assert_eq!(plain.ended().await?, DisplayEnd::Paused);
    assert_eq!(follower.ended().await?, DisplayEnd::Paused);
    Ok(())
}

/// The screen paints the private tab as soon as the browser brings it forward, and Xvnc answers
/// the update request the view holds with it. None of it reaches the viewer.
#[tokio::test]
async fn no_byte_xvnc_sends_after_the_switch_reaches_a_viewer() -> TestResult {
    let h = harness().await?;
    h.browser.list_tabs(&[3, 4], 3)?;
    h.make_private(4).await?;
    h.paint_when_in_front(4, PRIVATE_PX);
    let mut view = h.view(None).await?;
    view.send(HANDSHAKE);
    view.send(&UPDATE);
    view.read(SERVER_HANDSHAKE + 1).await?;
    view.send(&INCREMENTAL);
    h.received(0, 5).await?;

    h.bring_to_front(4, None).await?;
    let shown = view.until_the_end().await?;
    assert!(
        !shown
            .windows(PRIVATE_PX.len())
            .any(|pixel| pixel == PRIVATE_PX),
        "the private tab's pixels reached the viewer"
    );
    assert_eq!(view.ended().await?, DisplayEnd::Paused);
    Ok(())
}

#[tokio::test]
async fn no_view_is_admitted_while_a_private_tab_comes_forward() -> TestResult {
    let h = harness().await?;
    h.browser.list_tabs(&[3, 4], 3)?;
    h.make_private(4).await?;
    h.browser.hold_activations();
    let moving = tokio::spawn({
        let front = h.front();
        async move {
            front
                .bring_to_front(TabId(4), None, || Ok::<_, ()>(()))
                .await
        }
    });
    h.activation_held(4).await?;
    let dials = h.xvnc.connections();
    assert!(
        matches!(
            h.displays
                .admit(DisplayRequest::View { follow: None })
                .await,
            Err(PartError::DisplayPaused)
        ),
        "refused while the activation is in flight"
    );
    assert_eq!(h.xvnc.connections(), dials, "and it never reached Xvnc");

    h.browser.let_activations_go();
    timeout(WITHIN, moving).await???;
    assert!(
        matches!(
            h.displays
                .admit(DisplayRequest::View { follow: None })
                .await,
            Err(PartError::DisplayPaused)
        ),
        "refused once the switch is over: the private tab is in front"
    );
    let view = h.view(Some(Follow::Tab(TabId(3)))).await?;
    view.send(HANDSHAKE);
    assert_eq!(
        h.received(dials, 3).await?,
        handshake_received(),
        "a followTab=3 view brings 3 back, the host's newer request, and is admitted"
    );
    assert_eq!(h.log(), ["activate 4", "activate 3"]);
    Ok(())
}

#[tokio::test]
async fn a_tab_that_is_not_private_comes_forward_under_open_views() -> TestResult {
    let h = harness().await?;
    h.browser.list_tabs(&[3, 4], 3)?;
    let mut view = h.view(None).await?;
    view.send(HANDSHAKE);
    view.read(SERVER_HANDSHAKE).await?;

    h.bring_to_front(4, None).await?;
    view.send(&UPDATE);
    view.read(1).await?;
    assert!(
        !view.serving.is_finished(),
        "the view still passes the screen"
    );
    assert_eq!(h.log(), ["commit", "activate 4"]);
    Ok(())
}

#[tokio::test]
async fn the_owner_holding_the_screen_refuses_the_move_and_sends_nothing() -> TestResult {
    let h = harness().await?;
    h.browser.list_tabs(&[3, 4], 3)?;
    h.make_private(4).await?;
    let mut view = h.view(None).await?;
    view.send(HANDSHAKE);
    view.read(SERVER_HANDSHAKE).await?;
    let commits = AtomicUsize::new(0);
    let commit = || {
        commits.fetch_add(1, Ordering::SeqCst);
        Ok::<_, ()>(())
    };

    let mut control = h.control(None).await?;
    let refused = h.front().bring_to_front(TabId(4), None, commit).await;
    assert!(
        matches!(
            refused,
            Err(FrontError::OwnerHasScreen(OwnerHasScreen {
                by: ScreenHolder::ControlDisplay
            }))
        ),
        "an open control display, with no input: {refused:?}"
    );
    view.send(&UPDATE);
    view.read(1).await?;
    assert!(
        !view.serving.is_finished(),
        "the refused move closed no view"
    );
    control.leave();
    control.ended().await?;

    let hold = h.displays.hold_screen();
    let refused = h.front().bring_to_front(TabId(4), None, commit).await;
    assert!(
        matches!(
            refused,
            Err(FrontError::OwnerHasScreen(OwnerHasScreen {
                by: ScreenHolder::Takeover
            }))
        ),
        "a takeover's hold: {refused:?}"
    );
    assert_eq!(
        view.ended().await?,
        DisplayEnd::Paused,
        "the hold itself closed the view, as a takeover's start does"
    );
    drop(hold);
    assert_eq!(commits.load(Ordering::SeqCst), 0, "commit never ran");
    assert_eq!(h.activated()?, [0; 0], "and nothing was sent");
    Ok(())
}

#[tokio::test]
async fn a_refused_commit_sends_nothing_and_lets_views_back() -> TestResult {
    let h = harness().await?;
    h.browser.list_tabs(&[3, 4], 3)?;
    h.make_private(4).await?;
    let refused = h
        .front()
        .bring_to_front(TabId(4), None, || Err::<(), _>("the run was stopped"))
        .await;
    assert!(
        matches!(refused, Err(FrontError::Refused("the run was stopped"))),
        "the door's own refusal stands: {refused:?}"
    );
    assert_eq!(h.activated()?, [0; 0], "nothing was sent");
    let view = h.view(None).await?;
    view.send(HANDSHAKE);
    assert_eq!(
        h.received(0, 3).await?,
        handshake_received(),
        "views are let back, with 3 still in front"
    );
    Ok(())
}

/// Whether the agent's move ended with its activation's own error, the browser's.
fn activation_failed(moved: &Result<(), FrontError<()>>) -> bool {
    matches!(moved, Err(FrontError::Activation(PartError::Browser(_))))
}

/// `Browser.activateTab` answers an error, as when it gave no answer in time, yet the browser may
/// still switch to the private tab afterwards. Views stay out past the error, until the browser
/// reads what is in front, exactly as after an activation that answered. The test polls the move
/// itself, so while it checks, the move waits in that read whatever the clock does.
#[tokio::test]
async fn a_private_move_whose_activation_failed_keeps_views_out_until_the_front_is_read()
-> TestResult {
    let h = harness().await?;
    h.make_private(4).await?;
    h.fail_activations()?;
    let mut moving = pin!(h.bring_to_front(4, None));
    h.poll_past_the_activation(moving.as_mut()).await?;
    assert!(
        h.views_refused(),
        "a view asked for before the browser reads the front is refused"
    );

    // The switch the browser did not confirm lands after all.
    open_tabs(&h.browser.script, &[3, 4], 4)?;
    let moved = timeout(WITHIN, moving).await?;
    assert!(activation_failed(&moved), "the move's error: {moved:?}");
    assert!(
        h.front_refuses_views().await,
        "and with the private tab in front, views stay out (decision 34)"
    );
    Ok(())
}

/// A follow view's admission holds the turn while its activation is in flight. The agent's move
/// passes its first check and waits its turn behind it; then a control display takes the screen
/// and waits the turn out behind the move. Under the turn the move finds the owner holding the
/// screen, and sends nothing.
#[tokio::test]
async fn a_move_waiting_its_turn_is_refused_once_a_control_display_opens() -> TestResult {
    let h = harness().await?;
    h.browser.list_tabs(&[3, 4], 3)?;
    h.browser.hold_activations();
    let following = tokio::spawn({
        let displays = Arc::clone(&h.displays);
        async move {
            displays
                .admit(DisplayRequest::View {
                    follow: Some(Follow::Tab(TabId(4))),
                })
                .await
                .map(drop)
        }
    });
    h.activation_held(4).await?;
    let front = h.front();
    let mut moving = pin!(front.bring_to_front(TabId(3), None, || Ok::<_, ()>(())));
    assert!(
        moving.as_mut().now_or_never().is_none(),
        "the move passed its first check and waits its turn"
    );
    let control = tokio::spawn({
        let displays = Arc::clone(&h.displays);
        async move {
            displays
                .admit(DisplayRequest::Control { binding: None })
                .await
                .map(drop)
        }
    });
    eventually(|| front.check().is_err()).await?;
    h.browser.let_activations_go();

    let moved = timeout(WITHIN, moving).await?;
    assert!(
        matches!(
            moved,
            Err(FrontError::OwnerHasScreen(OwnerHasScreen {
                by: ScreenHolder::ControlDisplay
            }))
        ),
        "{moved:?}"
    );
    assert_eq!(h.log(), ["activate 4"], "it never sent activate 3");
    timeout(WITHIN, following).await???;
    timeout(WITHIN, control).await???;
    Ok(())
}

/// A control display asked for while the agent's move is in flight opens only once the move has
/// landed, so the owner never takes the screen in the middle of it.
#[tokio::test]
async fn a_control_display_opens_only_once_a_move_in_flight_has_landed() -> TestResult {
    let h = harness().await?;
    h.browser.list_tabs(&[3, 4], 3)?;
    h.browser.hold_activations();
    let moving = tokio::spawn({
        let front = h.front();
        async move {
            front
                .bring_to_front(TabId(4), None, || Ok::<_, ()>(()))
                .await
        }
    });
    h.activation_held(4).await?;
    let mut control = pin!(h.control(None));
    assert!(
        timeout(QUIET, control.as_mut()).await.is_err(),
        "the owner's display waits while the move is in flight"
    );

    h.browser.let_activations_go();
    let _control = timeout(WITHIN, control).await??;
    h.browser.log.push("control admitted");
    timeout(WITHIN, moving).await???;
    assert_eq!(h.log(), ["activate 4", "control admitted"]);
    Ok(())
}

/// A followTab=3 view and a followTask=T view are open, after T's op on 3. Another task's
/// bringToFront(4) moves the front, and neither follower undoes it. T's next op on 3 is its
/// follower's own edge, and brings 3 back.
#[tokio::test]
async fn follow_sees_the_agents_move_and_does_not_undo_it() -> TestResult {
    let h = harness().await?;
    h.browser.list_tabs(&[3, 4], 3)?;
    let (task, other) = (TaskId::parse("checkout")?, TaskId::parse("search")?);
    h.operation(&task, 3);
    h.sink.flush().await?;
    let _tab_view = h.view(Some(Follow::Tab(TabId(3)))).await?;
    let _task_view = h.view(Some(Follow::Task(task.clone()))).await?;

    h.bring_to_front(4, Some(&other)).await?;
    tokio::time::sleep(QUIET).await;
    assert_eq!(
        h.log(),
        ["activate 3", "activate 3", "commit", "activate 4"],
        "each view brought 3 forward as it opened, and neither undid the move to 4"
    );

    h.operation(&task, 3);
    eventually(|| {
        h.log()
            == [
                "activate 3",
                "activate 3",
                "commit",
                "activate 4",
                "activate 3",
            ]
    })
    .await?;
    Ok(())
}

/// The agent's bringToFront(4) for task T: T's follower adopts it, so T's op on 4 brings nothing
/// forward, and T's op on 3 brings 3 back.
#[tokio::test]
async fn a_task_follower_adopts_its_own_tasks_move() -> TestResult {
    let h = harness().await?;
    h.browser.list_tabs(&[3, 4], 3)?;
    let task = TaskId::parse("checkout")?;
    h.operation(&task, 3);
    h.sink.flush().await?;
    let _view = h.view(Some(Follow::Task(task.clone()))).await?;

    h.bring_to_front(4, Some(&task)).await?;
    h.operation(&task, 4);
    tokio::time::sleep(QUIET).await;
    assert_eq!(
        h.log(),
        ["activate 3", "commit", "activate 4"],
        "the task's own move is the follower's: its op on 4 brings nothing forward"
    );

    h.operation(&task, 3);
    eventually(|| h.log() == ["activate 3", "commit", "activate 4", "activate 3"]).await?;
    Ok(())
}

/// An operation's start reaches a follower through the trace writer, after the writer's commit,
/// while Neo's moves reach it at once. T's op on 3 began before another task's bringToFront(4)
/// but is read after it: it is old news, and the follower leaves 4 in front. T's op on 5, which
/// starts after the move, is the follower's own edge, and is read after the old one.
#[tokio::test]
async fn an_op_that_began_before_a_move_does_not_undo_it() -> TestResult {
    let h = harness().await?;
    h.browser.list_tabs(&[3, 4, 5], 3)?;
    let (task, other) = (TaskId::parse("checkout")?, TaskId::parse("search")?);
    let _view = h.view(Some(Follow::Task(task.clone()))).await?;
    h.operation(&task, 3);
    eventually(|| h.log() == ["activate 3"]).await?;

    let began = EpochMillis::now();
    h.bring_to_front(4, Some(&other)).await?;
    h.operation_at(&task, 3, began);
    // The records' clock reads milliseconds; an op in the move's own millisecond is older.
    h.operation_at(&task, 5, EpochMillis::now().plus_millis(1));
    eventually(|| h.log().last().is_some_and(|last| last == "activate 5")).await?;
    assert_eq!(
        h.log(),
        ["activate 3", "commit", "activate 4", "activate 5"],
        "the op that began before the move brought nothing back"
    );
    Ok(())
}

/// Another task's bringToFront(4) lands after a followTask=T view on 3 is admitted and before its
/// follower's first pass, and T's next op on 3 is committed in the same gap. The follower reads
/// both from its admission on, the move first, so T's op brings 3 back.
#[tokio::test]
async fn a_follower_reads_what_happened_between_its_admission_and_its_first_pass() -> TestResult {
    let h = harness().await?;
    h.browser.list_tabs(&[3, 4], 3)?;
    let (task, other) = (TaskId::parse("checkout")?, TaskId::parse("search")?);
    h.operation(&task, 3);
    h.sink.flush().await?;
    let admitted = h
        .displays
        .admit(DisplayRequest::View {
            follow: Some(Follow::Task(task.clone())),
        })
        .await?;
    h.bring_to_front(4, Some(&other)).await?;
    // The records' clock reads milliseconds; an op in the move's own millisecond is older.
    h.operation_at(&task, 3, EpochMillis::now().plus_millis(1));
    h.sink.flush().await?;

    let _view = serve(admitted);
    eventually(|| h.log() == ["activate 3", "commit", "activate 4", "activate 3"]).await?;
    Ok(())
}

/// A followTask=T view on 3 is admitted, another task's bringToFront(4) lands, and more trace
/// events arrive than the stream holds, all before the follower's first pass. It reads the move,
/// then finds it lost events and re-reads T's last tab: 3, from an op that began before the move,
/// so 4 stays in front. T's next op, on 5, comes forward. It is recorded only after a follower
/// that would bring 3 back has had the time to, or the store would already name 5.
#[tokio::test]
async fn a_follower_that_lost_events_never_brings_back_a_tab_older_than_the_move() -> TestResult {
    let h = harness().await?;
    h.browser.list_tabs(&[3, 4, 5], 3)?;
    let (task, other) = (TaskId::parse("checkout")?, TaskId::parse("search")?);
    h.operation(&task, 3);
    h.sink.flush().await?;
    let admitted = h.admit_view(Some(Follow::Task(task.clone()))).await?;
    h.bring_to_front(4, Some(&other)).await?;
    h.overflow_the_event_stream().await?;

    let _view = serve(admitted);
    tokio::time::sleep(QUIET).await;
    h.operation(&task, 5);
    eventually(|| h.log().last().is_some_and(|last| last == "activate 5")).await?;
    assert_eq!(
        h.log(),
        ["activate 3", "commit", "activate 4", "activate 5"],
        "the lost events brought nothing back over the move"
    );
    Ok(())
}

/// An operation of the task with no page ends with no tab, so the follower reads the task's last
/// tab back from the store. That tab is judged by when the task acted on it, not by this
/// operation's end, so T's tab 3, older than another task's move to 4, stays where the move left
/// it.
#[tokio::test]
async fn an_operation_with_no_page_never_brings_back_a_tab_older_than_the_move() -> TestResult {
    let h = harness().await?;
    h.browser.list_tabs(&[3, 4, 5], 3)?;
    let (task, other) = (TaskId::parse("checkout")?, TaskId::parse("search")?);
    h.operation(&task, 3);
    h.sink.flush().await?;
    let _view = h.view(Some(Follow::Task(task.clone()))).await?;
    eventually(|| h.log() == ["activate 3"]).await?;
    h.bring_to_front(4, Some(&other)).await?;

    // A millisecond past the move's stamp, so this operation is newer than the move.
    h.operation_with_no_page(&task, EpochMillis::now().plus_millis(1));
    h.sink.flush().await?;
    tokio::time::sleep(QUIET).await;
    h.operation(&task, 5);
    eventually(|| h.log().last().is_some_and(|last| last == "activate 5")).await?;
    assert_eq!(
        h.log(),
        ["activate 3", "commit", "activate 4", "activate 5"],
        "the operation with no page brought nothing back over the move"
    );
    Ok(())
}

/// A follower's own activation is no move its task's ops are judged against. T's op on 5 starts
/// before the admission brings 3 forward, and T's op on 4 before the follower brings 5 forward;
/// each is read after that activation, and each is T's newest work, so each comes forward.
#[tokio::test]
async fn a_followers_own_moves_never_make_its_tasks_ops_old_news() -> TestResult {
    let h = harness().await?;
    h.browser.list_tabs(&[3, 4, 5], 3)?;
    let task = TaskId::parse("checkout")?;
    h.operation(&task, 3);
    h.sink.flush().await?;
    let on_five = EpochMillis::now();
    let _view = h.view(Some(Follow::Task(task.clone()))).await?;
    h.operation_at(&task, 5, on_five);
    let on_four = EpochMillis::now();
    eventually(|| h.log() == ["activate 3", "activate 5"]).await?;

    h.operation_at(&task, 4, on_four);
    eventually(|| h.log() == ["activate 3", "activate 5", "activate 4"]).await?;
    Ok(())
}
