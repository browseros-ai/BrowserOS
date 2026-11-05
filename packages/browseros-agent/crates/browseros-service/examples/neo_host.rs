//! Serves the host API on a Unix socket the way claw-sdk will, with nothing from the app: a
//! trace store, its writer with the screenshot queue, `Privacy`, a `BrowserSession` over the
//! browser's CDP port, `Service::start`, and `host_service` behind a token file. It is the real
//! surface for checking the routes and the camera by hand, or from the Neo container harness:
//!
//! ```text
//! cargo run -p browseros-service --example neo_host -- --cdp-port 9110 \
//!     --open https://example.com --fake-writes
//! curl --unix-socket <socket> -H "Authorization: Bearer $(cat <token file>)" http://neo/v1/runs
//! ```
//!
//! - `--cdp-port <port>` (required): the browser's CDP port (9110 in Neo's container). The
//!   client keeps reconnecting rather than exiting when the browser restarts. Live views
//!   (`/v1/tabs/{tabId}/screencast`, `/video`) open a second connection to the same port.
//! - `--dir <path>`: keep the trace file, the screenshots, the socket and the token file there,
//!   and leave them at exit. Without it, a temp dir is used and removed at exit.
//! - `--open <url>`: opens the URL in a new tab (`Target.createTarget`) before serving, and
//!   prints the tab and its viewport.
//! - `--fake-writes`: every 2 s, records a run with one write on a real tab (the `--open` tab,
//!   else the first one `Browser.getTabs` lists), so the camera screenshots it without a
//!   dispatcher. Nothing is clicked; only the record is fake.
//! - `--fake-runs`: every 5 s, records a write and a read on tab 1.
//! - `--lock-check`: prints what the app's lock policy (`Service::locks().check`) answers for a
//!   write on that same tab, each time the answer changes: `allowed`, or `refused by <lock>`.
//!   It stands in for the dispatcher, which would refuse the agent's next click there.
//! - `--surface-calls`: also serves test-only calls under `/surface`, behind the same auth, for
//!   what the display and the takeover flow will do with the control journal:
//!   `POST /surface/journal {"tabs": [..]}` arms it (`{"millis"}`: how long arming took),
//!   `DELETE /surface/journal` disarms it, `POST /surface/runs <json>` records a run with that
//!   result (`{"runId"}`, committed when it answers) so a mask can be read back from
//!   `/v1/runs/{id}`, and `GET /surface/secrets` counts the secret set (`{"count"}`).
//! - `--xvnc <addr>`: Xvnc's RFB address for the fits and `GET /v1/display` (`127.0.0.1:5900`,
//!   the image's).
//! - `--neo-screen <WxH>`: the size the screen rests at between fits (`NEO_SCREEN` from the
//!   environment, else `1600x1000`). The service puts the screen there at start.
//! - `--agent-viewport <mobile|desktop>`: before serving, sets the `--open` tab's viewport to
//!   that mode the way the agent's own `setViewport` does, so a fit's restore can be seen to
//!   put the agent's mode back.
//!
//! It logs to stderr (`RUST_LOG`, `info` by default), including the camera's line for each
//! screenshot it drops, and serves until Ctrl-C.

#![expect(
    clippy::print_stdout,
    reason = "the paths and ids it prints are how a person or a script reaches the socket"
)]

use axum::body::Bytes;
use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use browseros_cdp::{CdpClient, ConnectOptions, ReconnectPolicy};
use browseros_core::viewport::{ViewportMode, ViewportRequest, Viewports};
use browseros_core::{BrowserSession, BrowserSessionHooks, BrowserView, PageJs, TabId};
use browseros_service::{HostAuth, HostToken, PeerRule, Service, ServiceInputs, host_service};
use browseros_takeover::{ArmedJournal, BrowserTabs, Journal, LockTarget, Locks, ScreenSize, Xvnc};
use browseros_trace::{
    Delivery, Effect, EpochMillis, OperationEnded, OperationId, OperationKind, OperationStarted,
    Outcome, PageState, Privacy, RedactionLimits, Redactor, RunEnded, RunId, RunStarted, RunStatus,
    SecretSet, SecretSetConfig, TargetSensitivity, TaskId, TraceSink, TraceStore, WriterConfig,
    screenshot_queue, spawn_writer,
};
use serde_json::{Value, json};
use std::error::Error;
use std::fs::Permissions;
use std::future::IntoFuture;
use std::net::SocketAddr;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::UnixListener;
use tokio_util::sync::CancellationToken;
use ulid::Ulid;

/// The command line.
struct Args {
    cdp_port: u16,
    dir: Option<PathBuf>,
    open: Option<String>,
    fake_writes: bool,
    fake_runs: bool,
    lock_check: bool,
    surface_calls: bool,
    xvnc: SocketAddr,
    neo_screen: ScreenSize,
    agent_viewport: Option<ViewportMode>,
}

impl Args {
    fn parse() -> Result<Self, String> {
        let mut args = std::env::args().skip(1);
        let mut parsed = Self {
            cdp_port: 0,
            dir: None,
            open: None,
            fake_writes: false,
            fake_runs: false,
            lock_check: false,
            surface_calls: false,
            xvnc: Xvnc::NEO_ADDR,
            neo_screen: std::env::var("NEO_SCREEN")
                .ok()
                .and_then(|screen| screen.parse().ok())
                .unwrap_or(ScreenSize {
                    width: 1600,
                    height: 1000,
                }),
            agent_viewport: None,
        };
        while let Some(arg) = args.next() {
            let mut value = || args.next().ok_or_else(|| format!("{arg} needs a value"));
            match arg.as_str() {
                "--cdp-port" => {
                    parsed.cdp_port = value()?
                        .parse()
                        .map_err(|_| "--cdp-port must be a port number".to_string())?;
                }
                "--dir" => parsed.dir = Some(PathBuf::from(value()?)),
                "--open" => parsed.open = Some(value()?),
                "--fake-writes" => parsed.fake_writes = true,
                "--fake-runs" => parsed.fake_runs = true,
                "--lock-check" => parsed.lock_check = true,
                "--surface-calls" => parsed.surface_calls = true,
                "--xvnc" => {
                    parsed.xvnc = value()?
                        .parse()
                        .map_err(|_| "--xvnc must be an address such as 127.0.0.1:5900")?;
                }
                "--neo-screen" => {
                    parsed.neo_screen = value()?
                        .parse()
                        .map_err(|_| "--neo-screen must be a size such as 1600x1000")?;
                }
                "--agent-viewport" => {
                    parsed.agent_viewport = Some(match value()?.as_str() {
                        "mobile" => ViewportMode::Mobile,
                        "desktop" => ViewportMode::Desktop,
                        _ => return Err("--agent-viewport must be mobile or desktop".into()),
                    });
                }
                other => return Err(format!("unknown argument {other}")),
            }
        }
        if parsed.cdp_port == 0 {
            return Err("--cdp-port <port> is required (9110 in Neo's container)".into());
        }
        Ok(parsed)
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let _ = tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_ansi(std::io::IsTerminal::is_terminal(&std::io::stderr()))
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .try_init();
    let args = Args::parse()?;
    let temp = match args.dir {
        Some(_) => None,
        None => Some(tempfile::Builder::new().prefix("neo-host-").tempdir()?),
    };
    let dir = match (&args.dir, &temp) {
        (Some(dir), _) => dir.clone(),
        (None, Some(temp)) => temp.path().to_path_buf(),
        (None, None) => return Err("no directory".into()),
    };
    std::fs::create_dir_all(&dir)?;

    // The agent's connection, as the app's browser link holds it. The default policy would
    // exit the process when reconnects run out.
    let client = CdpClient::connect(ConnectOptions {
        reconnect_policy: ReconnectPolicy::KeepTrying,
        reconnect_max_retries: usize::MAX,
        ..ConnectOptions::new(args.cdp_port)
    })
    .await?;
    let browser = BrowserSession::new(Arc::new(client), BrowserSessionHooks::default());

    let store = TraceStore::open(&dir.join("trace.db")).await?;
    let secrets = Arc::new(SecretSet::new(SecretSetConfig::default()));
    let (shots, shot_jobs) = screenshot_queue();
    let sink = spawn_writer(
        &store,
        Redactor::new(Arc::clone(&secrets), RedactionLimits::default()),
        WriterConfig::default(),
        Some(shots),
    );
    let privacy = Privacy::new(secrets, Arc::new(sink.clone()), store.private_tabs().await?);
    let viewports = Arc::new(Viewports::default());
    let surface = SurfaceCalls {
        journal: Journal::new(
            BrowserTabs::new(Arc::clone(&browser), Arc::clone(&viewports)),
            Arc::clone(privacy.secrets()),
        ),
        armed: Arc::default(),
        sink: sink.clone(),
        secrets: Arc::clone(privacy.secrets()),
    };
    let service = Service::start(ServiceInputs {
        store,
        sink: sink.clone(),
        privacy,
        browser: Arc::clone(&browser),
        viewports: Arc::clone(&viewports),
        screenshots: shot_jobs,
        xvnc: Xvnc {
            addr: args.xvnc,
            home: args.neo_screen,
        },
        cdp_port: args.cdp_port,
    });

    // What the app does at startup: read the token file, bind the socket owner-and-group only,
    // and take the peer rule from the socket it just made.
    let token_file = dir.join("token");
    std::fs::write(&token_file, format!("{}{}\n", Ulid::new(), Ulid::new()))?;
    let token = HostToken::parse(&std::fs::read_to_string(&token_file)?)?;
    let socket = dir.join("host.sock");
    if socket.exists() {
        std::fs::remove_file(&socket)?;
    }
    let listener = UnixListener::bind(&socket)?;
    std::fs::set_permissions(&socket, Permissions::from_mode(0o660))?;
    let auth = HostAuth {
        token,
        peer: Some(PeerRule::from_socket(&socket)?),
    };

    println!("socket: {}", socket.display());
    println!("token file: {}", token_file.display());
    println!("trace dir: {}", dir.display());
    let opened = match &args.open {
        Some(url) => Some(open_tab(&browser, url).await?),
        None => None,
    };
    if let Some(tab) = opened {
        println!("tab: {}", tab.0);
        if let Some(mode) = args.agent_viewport {
            set_agent_viewport(&browser, &viewports, tab, mode).await?;
        }
        println!("viewport: {}", viewport(&browser, tab).await?);
    }
    if args.fake_writes || args.lock_check {
        let tab = match opened {
            Some(tab) => tab,
            None => first_tab(&browser).await?,
        };
        if args.fake_writes {
            tokio::spawn(record_fake_writes(sink.clone(), tab));
        }
        if args.lock_check {
            tokio::spawn(print_lock_checks(Arc::clone(service.locks()), tab));
        }
    }
    if args.fake_runs {
        tokio::spawn(record_fake_runs(sink));
    }
    let routes = if args.surface_calls {
        service.router().merge(surface.routes())
    } else {
        service.router()
    };
    tokio::select! {
        served = axum::serve(listener, host_service(routes, auth)).into_future() => served?,
        _ = tokio::signal::ctrl_c() => {}
    }
    Ok(())
}

/// The test-only calls `--surface-calls` serves: the control journal as the display and the
/// takeover flow will arm it, a run whose result the writer masks, and the secret set's size.
#[derive(Clone)]
struct SurfaceCalls {
    journal: Journal,
    armed: Arc<tokio::sync::Mutex<Option<ArmedJournal>>>,
    sink: TraceSink,
    secrets: Arc<SecretSet>,
}

type SurfaceReply = Result<Json<Value>, (StatusCode, String)>;

impl SurfaceCalls {
    fn routes(self) -> Router {
        Router::new()
            .route("/surface/journal", post(Self::arm).delete(Self::disarm))
            .route("/surface/runs", post(Self::fake_run))
            .route("/surface/secrets", get(Self::count))
            .with_state(self)
    }

    /// Arms the journal on the body's `tabs`, disarming one armed before.
    async fn arm(State(calls): State<Self>, body: Bytes) -> SurfaceReply {
        let request: Value = serde_json::from_slice(&body)
            .map_err(|error| (StatusCode::BAD_REQUEST, error.to_string()))?;
        let tabs = request["tabs"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_i64)
            .map(TabId)
            .collect::<Vec<_>>();
        let mut armed = calls.armed.lock().await;
        if let Some(previous) = armed.take() {
            previous.disarm().await;
        }
        let started = std::time::Instant::now();
        *armed = Some(calls.journal.arm(&tabs).await);
        let millis = started.elapsed().as_secs_f64() * 1000.0;
        println!("journal armed on {} tab(s) in {millis:.1} ms", tabs.len());
        Ok(Json(json!({ "millis": millis })))
    }

    async fn disarm(State(calls): State<Self>) -> SurfaceReply {
        let armed = calls.armed.lock().await.take();
        let disarmed = armed.is_some();
        if let Some(armed) = armed {
            armed.disarm().await;
        }
        println!("journal disarmed: {disarmed}");
        Ok(Json(json!({ "disarmed": disarmed })))
    }

    /// Records a run whose result is the body, as the agent's would be, and answers once the
    /// writer committed it.
    async fn fake_run(State(calls): State<Self>, body: Bytes) -> SurfaceReply {
        let result: Value = serde_json::from_slice(&body)
            .map_err(|error| (StatusCode::BAD_REQUEST, error.to_string()))?;
        let run = RunId::new();
        calls.sink.run_started(RunStarted {
            id: run,
            code: "return await page.content()".into(),
            request_id: None,
            task_id: TaskId::parse("surface").ok(),
            host_data: None,
            at: EpochMillis::now(),
        });
        calls.sink.run_ended(RunEnded {
            id: run,
            status: RunStatus::Ok,
            error: None,
            result: Some(result),
            replay: None,
            at: EpochMillis::now(),
        });
        calls
            .sink
            .flush()
            .await
            .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
        Ok(Json(json!({ "runId": run.to_string() })))
    }

    async fn count(State(calls): State<Self>) -> SurfaceReply {
        Ok(Json(json!({ "count": calls.secrets.count() })))
    }
}

/// Opens `url` in a new tab and returns its tab id, once `Browser.getTabs` lists it.
async fn open_tab(browser: &BrowserSession, url: &str) -> Result<TabId, Box<dyn Error>> {
    let created = browser
        .cdp("Target.createTarget", json!({ "url": url }), None)
        .await?;
    let target = created["targetId"]
        .as_str()
        .ok_or("Target.createTarget returned no targetId")?;
    for _ in 0..50 {
        let pages = browser.pages.list().await?;
        if let Some(page) = pages.iter().find(|page| page.target_id.0 == target) {
            return Ok(page.tab_id);
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    Err(format!("the tab of target {target} never appeared").into())
}

/// Sets `tab`'s viewport to `mode` at the mode's own size, as the agent's `setViewport` does,
/// through the same `Viewports` the service reads, and prints `agent viewport: <mode>`.
async fn set_agent_viewport(
    browser: &Arc<BrowserSession>,
    viewports: &Arc<Viewports>,
    tab: TabId,
    mode: ViewportMode,
) -> Result<(), Box<dyn Error>> {
    let pages = browser.pages.resolve_tab_ids(&[tab]).await?;
    let page = pages.get(&tab).cloned().ok_or("the opened tab closed")?;
    BrowserView::unbound(Arc::clone(browser), None)
        .with_viewports(Arc::clone(viewports))
        .set_viewport(
            page,
            ViewportRequest::new(mode, None)?,
            &CancellationToken::new(),
        )
        .await?;
    println!("agent viewport: {mode:?}");
    Ok(())
}

/// The first tab `Browser.getTabs` lists.
async fn first_tab(browser: &BrowserSession) -> Result<TabId, Box<dyn Error>> {
    let pages = browser.pages.list().await?;
    let page = pages.first().ok_or("the browser has no tab to write on")?;
    println!("tab: {}", page.tab_id.0);
    Ok(page.tab_id)
}

/// The tab's viewport as `[innerWidth, innerHeight, devicePixelRatio]`: a screenshot of it
/// is `innerWidth × innerHeight` CSS pixels at that ratio.
async fn viewport(browser: &BrowserSession, tab: TabId) -> Result<String, Box<dyn Error>> {
    let pages = browser.pages.resolve_tab_ids(&[tab]).await?;
    let page = pages.get(&tab).cloned().ok_or("the opened tab closed")?;
    let session = browser.pages.get_session(page).await?;
    // Window geometry reads the same in either world, so this needs no page JavaScript.
    Ok(PageJs::isolated(&session.session)
        .eval::<String>("JSON.stringify([innerWidth, innerHeight, devicePixelRatio])")
        .await?)
}

/// One run with one write on `tab` every two seconds, for the camera to picture.
async fn record_fake_writes(sink: TraceSink, tab: TabId) {
    let mut every = tokio::time::interval(Duration::from_secs(2));
    loop {
        every.tick().await;
        let run = RunId::new();
        let op = OperationId::new(run, 1);
        sink.run_started(RunStarted {
            id: run,
            code: "await page.locator('h1').click()".into(),
            request_id: None,
            task_id: TaskId::parse("surface").ok(),
            host_data: None,
            at: EpochMillis::now(),
        });
        sink.op_started(OperationStarted {
            id: op,
            parent: None,
            name: "locator.click".into(),
            kind: OperationKind::Builtin,
            effect: Effect::Write,
            tab: Some(tab),
            args: json!({"selector": "h1"}),
            policy: Arc::default(),
            at: EpochMillis::now(),
        });
        sink.op_ended(OperationEnded {
            id: op,
            outcome: Outcome::Ok,
            delivery: Delivery::Done,
            sent_at: Some(EpochMillis::now()),
            result: None,
            error: None,
            page_after: Some(PageState {
                page_id: None,
                tab_id: Some(tab),
                url: None,
                title: None,
            }),
            target: TargetSensitivity::Ordinary,
            at: EpochMillis::now(),
        });
        sink.run_ended(RunEnded {
            id: run,
            status: RunStatus::Ok,
            error: None,
            result: None,
            replay: None,
            at: EpochMillis::now(),
        });
        println!("fake write: run {run} op {op} tab {}", tab.0);
    }
}

/// Prints the lock policy's answer for a write on `tab` whenever it changes, checking every
/// 100 ms: what the dispatcher would answer for the agent's next click there.
async fn print_lock_checks(locks: Arc<Locks>, tab: TabId) {
    let mut last = None;
    let mut every = tokio::time::interval(Duration::from_millis(100));
    loop {
        every.tick().await;
        let answer = match locks.check(LockTarget::Tab(tab), Effect::Write) {
            Ok(()) => "allowed".to_string(),
            Err(refused) => format!("refused by {}: {refused}", refused.lock),
        };
        if last.as_ref() != Some(&answer) {
            println!("lock check: a write on tab {}: {answer}", tab.0);
            last = Some(answer);
        }
    }
}

/// One run every five seconds for task `demo`: a write on tab 1, then a read of it.
async fn record_fake_runs(sink: TraceSink) {
    let mut every = tokio::time::interval(Duration::from_secs(5));
    loop {
        every.tick().await;
        let run = RunId::new();
        sink.run_started(RunStarted {
            id: run,
            code: "await page.goto('https://example.com'); return await page.title()".into(),
            request_id: None,
            task_id: TaskId::parse("demo").ok(),
            host_data: None,
            at: EpochMillis::now(),
        });
        for (seq, name, effect) in [
            (1, "page.goto", Effect::Write),
            (2, "page.title", Effect::Read),
        ] {
            let id = OperationId::new(run, seq);
            sink.op_started(OperationStarted {
                id,
                parent: None,
                name: name.into(),
                kind: OperationKind::Builtin,
                effect,
                tab: Some(TabId(1)),
                args: json!({"page": {"$neo": "page", "id": 1}}),
                policy: Arc::default(),
                at: EpochMillis::now(),
            });
            tokio::time::sleep(Duration::from_millis(250)).await;
            sink.op_ended(OperationEnded {
                id,
                outcome: Outcome::Ok,
                delivery: Delivery::Done,
                sent_at: (effect == Effect::Write).then(EpochMillis::now),
                result: (effect == Effect::Read).then(|| json!("Example Domain")),
                error: None,
                page_after: Some(PageState {
                    page_id: Some(1),
                    tab_id: Some(TabId(1)),
                    url: Some("https://example.com/".into()),
                    title: Some("Example Domain".into()),
                }),
                target: TargetSensitivity::Ordinary,
                at: EpochMillis::now(),
            });
        }
        sink.run_ended(RunEnded {
            id: run,
            status: RunStatus::Ok,
            error: None,
            result: Some(json!("Example Domain")),
            replay: None,
            at: EpochMillis::now(),
        });
    }
}
