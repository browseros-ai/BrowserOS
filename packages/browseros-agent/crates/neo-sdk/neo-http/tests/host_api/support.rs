//! The harness the host API tests share: a trace store in a temp dir, its writer, a scripted
//! browser, a scripted capture
//! connection for the live views, and the started service's router. Plus recorders for runs and
//! operations, and a reader for the event stream.

use crate::browser::ScriptedBrowser;
use axum::Router;
use axum::body::{Body, BodyDataStream, to_bytes};
use axum::extract::connect_info::MockConnectInfo;
use axum::http::{HeaderMap, HeaderValue, Method, Request, StatusCode, header};
use axum::middleware::map_request;
use axum::response::Response;
use browseros_core::TabId;
use browseros_core::viewport::Viewports;
use futures_util::StreamExt;
use neo_http::{HostAuth, HostToken, PeerInfo, Service, ServiceInputs, Settings, host_router};
use neo_screencast::CaptureTransport;
use neo_screencast::testing::{ScriptedCdp, ScriptedTransport};
use neo_takeover::ScreenSize;
use neo_takeover::testing::FakeXvnc;
use neo_trace::{
    Delivery, Effect, EpochMillis, OperationEnded, OperationId, OperationKind, OperationStarted,
    Outcome, RedactionLimits, Redactor, RequestId, RunEnded, RunId, RunStarted, RunStatus,
    RunTabsScope, TaskId, TraceSink, TraceStore, WriterConfig, screenshot_queue, spawn_writer,
};
use serde_json::Value;
use std::collections::HashMap;
use std::future::Future;
use std::io::{self, Write};
use std::process;
use std::sync::{Arc, Mutex, PoisonError, mpsc};
use std::time::Duration;
use tower::ServiceExt;

pub(crate) type BoxError = Box<dyn std::error::Error + Send + Sync>;
pub(crate) type TestResult = Result<(), BoxError>;

/// Keeps paused Tokio time fixed across real SQLite and filesystem work. Pinned Tokio 1.52.3
/// inhibits auto-advance synchronously when a blocking task is scheduled.
#[must_use = "retain the guard while test deadlines must remain frozen"]
pub(crate) struct ManualClock {
    release: Option<mpsc::Sender<()>>,
    worker: Option<tokio::task::JoinHandle<()>>,
}

impl ManualClock {
    fn pause() -> Self {
        let mut clock = Self {
            release: None,
            worker: None,
        };
        clock.hold();
        tokio::time::pause();
        clock
    }

    fn hold(&mut self) {
        let (release, held) = mpsc::channel();
        self.worker = Some(tokio::task::spawn_blocking(move || {
            // Tokio hang guards cannot expire while this worker holds virtual time fixed.
            // A panic here would stay in an unobserved JoinHandle, so abort the test binary
            // on a wall-clock stall instead of leaving the ordinary SDK gate hung.
            if let Err(mpsc::RecvTimeoutError::Timeout) =
                held.recv_timeout(Duration::from_secs(120))
            {
                let _ = io::stderr()
                    .lock()
                    .write_all(b"test stalled for 120 seconds with the manual clock frozen\n");
                process::abort();
            }
        }));
        self.release = Some(release);
    }

    /// Allows natural timer registration and auto-advance only until a latched milestone.
    /// Callers use this for hung captures and retries, never while waiting on real storage.
    pub(crate) async fn run_timers_until<F: Future>(&mut self, milestone: F) -> F::Output {
        let worker = self.worker.take();
        drop(self.release.take());
        // Poll the milestone before yielding so its watch and timeout are registered before
        // the released worker permits any automatic jump.
        let result = milestone.await;
        // Acquire in this poll, before an external I/O wait can make Tokio jump ahead.
        self.hold();
        if let Some(worker) = worker {
            let _ = worker.await;
        }
        result
    }
}

impl Drop for ManualClock {
    fn drop(&mut self) {
        tokio::time::resume();
        // Unconditional release keeps a panic from stranding runtime shutdown.
        drop(self.release.take());
    }
}

/// The token every harness request carries. The router refuses requests that did not pass the
/// host auth, so the harness serves it the way production does: behind `host_router`.
pub(crate) const TOKEN: &str = "harness-token-0123456789-abcdefghij";

/// `service`'s routes behind the real host auth, reached as a TCP peer, with the token added to
/// every request.
fn authenticated(service: &Service) -> Result<Router, BoxError> {
    let auth = HostAuth {
        token: HostToken::parse(TOKEN)?,
        peer: None,
    };
    let bearer = HeaderValue::from_str(&format!("Bearer {TOKEN}"))?;
    Ok(host_router(service.router(), auth)
        .layer(MockConnectInfo(PeerInfo::tcp()))
        .layer(map_request(move |mut request: axum::extract::Request| {
            let bearer = bearer.clone();
            async move {
                request
                    .headers_mut()
                    .entry(header::AUTHORIZATION)
                    .or_insert(bearer);
                request
            }
        })))
}

pub(crate) struct Harness {
    pub(crate) dir: tempfile::TempDir,
    pub(crate) sink: TraceSink,
    pub(crate) browser: Arc<ScriptedBrowser>,
    /// Neo's screen, which the fits resize.
    pub(crate) xvnc: FakeXvnc,
    /// The browser as the live views' own capture connection reaches it: the frames and
    /// recording bytes a test emits, and the commands the capture hub sent.
    pub(crate) capture: Arc<ScriptedCdp>,
    /// The started service, for what the app calls directly: the locks and the tab events.
    pub(crate) service: Service,
    /// The settings the service serves, as the app hands them to the parts that read them.
    pub(crate) settings: Settings,
    /// The tab the app keeps for itself (its blank tab), as `ServiceInputs::tab_owner` reports
    /// it; `None` until a test sets one.
    pub(crate) neo_tab: Arc<Mutex<Option<i64>>>,
    /// The task holding each tab, as `ServiceInputs::tab_task` reports it; empty until a test
    /// gives a tab to a task.
    pub(crate) tab_tasks: Arc<Mutex<HashMap<i64, String>>>,
    /// The routes behind the host auth, as a host reaches them.
    pub(crate) router: Router,
    /// `Service::router()` alone, for tests that serve it themselves.
    pub(crate) routes: Router,
}

/// The size Neo's screen rests at between fits in the harness (`NEO_SCREEN`).
pub(crate) const HOME_SCREEN: ScreenSize = ScreenSize {
    width: 1600,
    height: 1000,
};

/// How the harness starts.
struct Boot {
    camera: bool,
    writer: WriterConfig,
    /// The screen's size when the service starts.
    screen: ScreenSize,
    /// The window's state when the service starts.
    window: &'static str,
    /// What the live views capture over, when not the scripted capture connection.
    capture_transport: Option<Arc<dyn CaptureTransport>>,
}

/// A started service over a fresh store, with no camera: no screenshot is taken, and a run is
/// complete when it ends.
pub(crate) async fn harness() -> Result<Harness, BoxError> {
    start(Boot::default()).await
}

/// A started service whose camera takes the writer's screenshots on the scripted browser.
pub(crate) async fn harness_with_camera() -> Result<Harness, BoxError> {
    let boot = Boot {
        camera: true,
        ..Boot::default()
    };
    start(boot).await
}

/// A camera fixture and the guard that owns its semantic clock. The clock field drops first
/// so an intact fixture resumes time before releasing its service.
pub(crate) struct ClockedHarness {
    pub(crate) clock: ManualClock,
    pub(crate) harness: Harness,
}

/// A camera fixture whose deadlines advance only when the test requests it. Single-record
/// batches bypass the writer's batching timer, which cannot fire while the clock is frozen.
pub(crate) async fn harness_with_manual_clock() -> Result<ClockedHarness, BoxError> {
    let harness = start(Boot {
        camera: true,
        writer: WriterConfig {
            batch_max: 1,
            ..WriterConfig::default()
        },
        ..Boot::default()
    })
    .await?;
    Ok(ClockedHarness {
        clock: ManualClock::pause(),
        harness,
    })
}

/// A service started over what a Neo restart in the middle of a fit leaves: the screen at the
/// owner's phone size and the window fullscreen.
pub(crate) async fn harness_after_a_crash_mid_fit(screen: ScreenSize) -> Result<Harness, BoxError> {
    let boot = Boot {
        screen,
        window: "fullscreen",
        ..Boot::default()
    };
    start(boot).await
}

/// A started service whose live views capture over `transport` rather than the scripted
/// capture connection.
pub(crate) async fn harness_with_capture(
    transport: Arc<dyn CaptureTransport>,
) -> Result<Harness, BoxError> {
    let boot = Boot {
        capture_transport: Some(transport),
        ..Boot::default()
    };
    start(boot).await
}

impl Default for Boot {
    fn default() -> Self {
        Self {
            camera: false,
            writer: WriterConfig::default(),
            screen: HOME_SCREEN,
            window: "maximized",
            capture_transport: None,
        }
    }
}

async fn start(
    Boot {
        camera,
        writer,
        screen,
        window,
        capture_transport,
    }: Boot,
) -> Result<Harness, BoxError> {
    let dir = tempfile::tempdir()?;
    let store = TraceStore::open(&dir.path().join("trace.db")).await?;
    // Without a camera the writer gets no queue, and the service's end of it sees the writer
    // gone at once.
    let (queue, jobs) = screenshot_queue();
    let sink = spawn_writer(
        &store,
        Redactor::new(RedactionLimits::default()),
        writer,
        camera.then_some(queue),
    );
    let settings = Settings::load(dir.path().join("settings.json"), Default::default()).await;
    let neo_tab: Arc<Mutex<Option<i64>>> = Arc::default();
    let tab_tasks: Arc<Mutex<HashMap<i64, String>>> = Arc::default();
    let browser = ScriptedBrowser::new();
    browser.set_window(window);
    let xvnc = FakeXvnc::start(screen).await?;
    let capture = ScriptedCdp::new();
    let service = Service::start_with_capture(
        ServiceInputs {
            store: store.clone(),
            sink: sink.clone(),
            browser: browser.session(),
            viewports: Arc::new(Viewports::default()),
            screenshots: jobs,
            xvnc: xvnc.xvnc(HOME_SCREEN),
            // Never dialed: the live views capture over the transport below.
            cdp_port: 0,
            settings: settings.clone(),
            tab_owner: {
                let neo_tab = Arc::clone(&neo_tab);
                Arc::new(move |tab: TabId| {
                    let held = *neo_tab.lock().unwrap_or_else(PoisonError::into_inner);
                    (held == Some(tab.0)).then_some("neo")
                })
            },
            tab_task: {
                let tab_tasks = Arc::clone(&tab_tasks);
                Arc::new(move |tab: TabId| {
                    tab_tasks
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .get(&tab.0)
                        .cloned()
                })
            },
            tab_opener: Arc::new(|_| None),
        },
        capture_transport.unwrap_or_else(|| Arc::new(ScriptedTransport(Arc::clone(&capture)))),
    );
    // The service puts the screen home in the background as it starts. Waiting for that keeps
    // a test's own resizes, and its scripted refusals, the only ones after it.
    tokio::time::timeout(Duration::from_secs(5), async {
        while xvnc.resizes().is_empty() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await?;
    Ok(Harness {
        dir,
        sink,
        browser,
        xvnc,
        capture,
        router: authenticated(&service)?,
        routes: service.router(),
        service,
        settings,
        neo_tab,
        tab_tasks,
    })
}

/// A response as a host reads it.
pub(crate) struct Reply {
    pub(crate) status: StatusCode,
    pub(crate) headers: HeaderMap,
    /// The body as JSON; `Null` when it is empty.
    pub(crate) body: Value,
}

impl Reply {
    /// `response` as a host reads it.
    pub(crate) async fn read(response: Response) -> Result<Self, BoxError> {
        let status = response.status();
        let headers = response.headers().clone();
        let bytes = to_bytes(response.into_body(), usize::MAX).await?;
        let body = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes)?
        };
        Ok(Self {
            status,
            headers,
            body,
        })
    }

    pub(crate) fn request_id(&self) -> Option<&str> {
        self.headers
            .get("x-request-id")
            .and_then(|id| id.to_str().ok())
    }

    /// The contract's error: `status`, `code`, and a `requestId` equal to `x-request-id`.
    pub(crate) fn assert_error(&self, status: StatusCode, code: &str) {
        assert_eq!(self.status, status, "{}", self.body);
        assert_eq!(self.body["code"], code, "{}", self.body);
        assert!(self.request_id().is_some(), "an error without x-request-id");
        assert_eq!(self.body["requestId"].as_str(), self.request_id());
    }

    /// The `id` of each item in a run list.
    pub(crate) fn run_ids(&self) -> Vec<String> {
        self.body["items"]
            .as_array()
            .map(|runs| {
                runs.iter()
                    .filter_map(|run| run["id"].as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default()
    }
}

impl Harness {
    pub(crate) async fn get(&self, uri: &str) -> Result<Reply, BoxError> {
        self.call(Method::GET, uri, Body::empty()).await
    }

    pub(crate) async fn post(&self, uri: &str, body: &str) -> Result<Reply, BoxError> {
        self.call(Method::POST, uri, Body::from(body.to_string()))
            .await
    }

    pub(crate) async fn delete(&self, uri: &str) -> Result<Reply, BoxError> {
        self.call(Method::DELETE, uri, Body::empty()).await
    }

    pub(crate) async fn call(
        &self,
        method: Method,
        uri: &str,
        body: Body,
    ) -> Result<Reply, BoxError> {
        let request = Request::builder()
            .method(method)
            .uri(uri)
            .header(header::CONTENT_TYPE, "application/json")
            .body(body)?;
        Reply::read(self.router.clone().oneshot(request).await?).await
    }

    /// A response's status, headers and raw body, for a route that answers bytes.
    pub(crate) async fn get_bytes(
        &self,
        uri: &str,
    ) -> Result<(StatusCode, HeaderMap, Vec<u8>), BoxError> {
        let request = Request::builder().uri(uri).body(Body::empty())?;
        let response = self.router.clone().oneshot(request).await?;
        let status = response.status();
        let headers = response.headers().clone();
        let bytes = to_bytes(response.into_body(), usize::MAX).await?;
        Ok((status, headers, bytes.to_vec()))
    }

    /// Waits until everything recorded so far is committed.
    pub(crate) async fn flushed(&self) -> TestResult {
        self.sink.flush().await?;
        Ok(())
    }
}

/// Recorded times are offsets from this moment, in Nov 2023, so they are in the past.
pub(crate) const BASE_MILLIS: i64 = 1_700_000_000_000;

pub(crate) fn at(millis: i64) -> EpochMillis {
    EpochMillis::from_millis(BASE_MILLIS + millis)
}

/// How a recorded run starts.
#[derive(Clone, Copy, Default)]
pub(crate) struct Start<'a> {
    pub(crate) task: Option<&'a str>,
    pub(crate) request: Option<&'a str>,
    pub(crate) at: i64,
}

pub(crate) fn start_run(sink: &TraceSink, start: Start<'_>) -> RunId {
    let run = RunId::new();
    start_known_run(sink, run, start);
    run
}

pub(crate) fn start_known_run(sink: &TraceSink, run: RunId, start: Start<'_>) {
    sink.run_started(RunStarted {
        id: run,
        code: "await page.goto(url)".to_string(),
        request_id: start.request.and_then(|id| RequestId::parse(id).ok()),
        task_id: start.task.and_then(|id| TaskId::parse(id).ok()),
        host_data: None,
        tabs_scope: RunTabsScope::Browser,
        at: at(start.at),
    });
}

/// How a recorded operation starts.
#[derive(Clone, Copy)]
pub(crate) struct Op<'a> {
    pub(crate) name: &'a str,
    pub(crate) effect: Effect,
    pub(crate) tab: Option<i64>,
    pub(crate) at: i64,
}

impl Op<'_> {
    /// A write on `tab` at `at`.
    pub(crate) fn write(tab: i64, at: i64) -> Self {
        Self {
            name: "locator.click",
            effect: Effect::Write,
            tab: Some(tab),
            at,
        }
    }
}

pub(crate) fn op_started(sink: &TraceSink, id: OperationId, op: Op<'_>) {
    sink.op_started(OperationStarted {
        site: None,
        id,
        parent: None,
        name: op.name.to_string(),
        kind: OperationKind::Builtin,
        effect: op.effect,
        tab: op.tab.map(TabId),
        args: serde_json::json!({"page": op.tab}),
        at: at(op.at),
    });
}

pub(crate) fn op_ended(
    sink: &TraceSink,
    id: OperationId,
    delivery: Delivery,
    result: Option<Value>,
) {
    sink.op_ended(OperationEnded {
        id,
        outcome: Outcome::Ok,
        delivery,
        sent_at: None,
        result,
        error: None,
        page_after: None,
        main_world: false,
        frames_incomplete: false,
        at: at(10_000),
    });
}

pub(crate) fn end_run(sink: &TraceSink, run: RunId, result: Option<Value>) {
    sink.run_ended(RunEnded {
        id: run,
        status: RunStatus::Ok,
        error: None,
        result,
        replay: None,
        at: at(20_000),
    });
}

/// One server-sent event.
#[derive(Debug)]
pub(crate) struct Sse {
    pub(crate) id: String,
    pub(crate) kind: String,
    pub(crate) data: Value,
}

impl Sse {
    /// The seq in `<boot>-<seq>`.
    pub(crate) fn seq(&self) -> Option<u64> {
        self.id
            .rsplit_once('-')
            .and_then(|(_, seq)| seq.parse().ok())
    }

    pub(crate) fn boot(&self) -> Option<&str> {
        self.id.rsplit_once('-').map(|(boot, _)| boot)
    }
}

/// `GET /v1/events`, read one event at a time, each within a few seconds.
pub(crate) struct EventStream {
    body: BodyDataStream,
    buffer: String,
}

impl EventStream {
    pub(crate) async fn open(
        harness: &Harness,
        uri: &str,
        last_event_id: Option<&str>,
    ) -> Result<Self, BoxError> {
        let mut request = Request::builder().uri(uri);
        if let Some(id) = last_event_id {
            request = request.header("last-event-id", id);
        }
        let response = harness
            .router
            .clone()
            .oneshot(request.body(Body::empty())?)
            .await?;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response
                .headers()
                .get(header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok()),
            Some("text/event-stream")
        );
        Ok(Self {
            body: response.into_body().into_data_stream(),
            buffer: String::new(),
        })
    }

    pub(crate) async fn next(&mut self) -> Result<Sse, BoxError> {
        loop {
            if let Some(end) = self.buffer.find("\n\n") {
                let block = self.buffer.drain(..end + 2).collect::<String>();
                if let Some(event) = parse_sse(&block)? {
                    return Ok(event);
                }
                continue;
            }
            let chunk = tokio::time::timeout(Duration::from_secs(5), self.body.next())
                .await?
                .ok_or("the event stream ended")??;
            self.buffer.push_str(std::str::from_utf8(&chunk)?);
        }
    }

    /// The next `count` events.
    pub(crate) async fn take(&mut self, count: usize) -> Result<Vec<Sse>, BoxError> {
        let mut events = Vec::with_capacity(count);
        for _ in 0..count {
            events.push(self.next().await?);
        }
        Ok(events)
    }
}

/// One SSE block; `None` for a keep-alive comment.
fn parse_sse(block: &str) -> Result<Option<Sse>, BoxError> {
    let (mut id, mut kind, mut data) = (None, None, None);
    for line in block.lines() {
        if let Some(value) = line.strip_prefix("id: ") {
            id = Some(value.to_string());
        } else if let Some(value) = line.strip_prefix("event: ") {
            kind = Some(value.to_string());
        } else if let Some(value) = line.strip_prefix("data: ") {
            data = Some(serde_json::from_str::<Value>(value)?);
        }
    }
    let Some(data) = data else {
        return Ok(None);
    };
    Ok(Some(Sse {
        id: id.ok_or("an event without an id")?,
        kind: kind.ok_or("an event without a type")?,
        data,
    }))
}
