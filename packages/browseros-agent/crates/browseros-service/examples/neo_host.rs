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
//!   client keeps reconnecting rather than exiting when the browser restarts.
//! - `--dir <path>`: keep the trace file, the screenshots, the socket and the token file there,
//!   and leave them at exit. Without it, a temp dir is used and removed at exit.
//! - `--open <url>`: opens the URL in a new tab (`Target.createTarget`) before serving, and
//!   prints the tab and its viewport.
//! - `--fake-writes`: every 2 s, records a run with one write on a real tab (the `--open` tab,
//!   else the first one `Browser.getTabs` lists), so the camera screenshots it without a
//!   dispatcher. Nothing is clicked; only the record is fake.
//! - `--fake-runs`: every 5 s, records a write and a read on tab 1.
//!
//! It logs to stderr (`RUST_LOG`, `info` by default), including the camera's line for each
//! screenshot it drops, and serves until Ctrl-C.

#![expect(
    clippy::print_stdout,
    reason = "the paths and ids it prints are how a person or a script reaches the socket"
)]

use browseros_cdp::{CdpClient, ConnectOptions, ReconnectPolicy};
use browseros_core::viewport::Viewports;
use browseros_core::{BrowserSession, BrowserSessionHooks, TabId};
use browseros_service::{HostAuth, HostToken, PeerRule, Service, ServiceInputs, host_service};
use browseros_trace::{
    Delivery, Effect, EpochMillis, OperationEnded, OperationId, OperationKind, OperationStarted,
    Outcome, PageState, Privacy, RedactionLimits, Redactor, RunEnded, RunId, RunStarted, RunStatus,
    SecretSet, SecretSetConfig, TargetSensitivity, TaskId, TraceSink, TraceStore, WriterConfig,
    screenshot_queue, spawn_writer,
};
use serde_json::json;
use std::error::Error;
use std::fs::Permissions;
use std::future::IntoFuture;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::UnixListener;
use ulid::Ulid;

/// The command line.
struct Args {
    cdp_port: u16,
    dir: Option<PathBuf>,
    open: Option<String>,
    fake_writes: bool,
    fake_runs: bool,
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
    let service = Service::start(ServiceInputs {
        store,
        sink: sink.clone(),
        privacy,
        browser: Arc::clone(&browser),
        viewports: Arc::new(Viewports::default()),
        screenshots: shot_jobs,
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
        println!("viewport: {}", viewport(&browser, tab).await?);
    }
    if args.fake_writes {
        let tab = match opened {
            Some(tab) => tab,
            None => first_tab(&browser).await?,
        };
        tokio::spawn(record_fake_writes(sink.clone(), tab));
    }
    if args.fake_runs {
        tokio::spawn(record_fake_runs(sink));
    }
    tokio::select! {
        served = axum::serve(listener, host_service(service.router(), auth)).into_future() => served?,
        _ = tokio::signal::ctrl_c() => {}
    }
    Ok(())
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
    let reply = browser
        .cdp(
            "Runtime.evaluate",
            json!({
                "expression": "JSON.stringify([innerWidth, innerHeight, devicePixelRatio])",
                "returnByValue": true,
            }),
            Some(&session.session_id),
        )
        .await?;
    Ok(reply["result"]["value"]
        .as_str()
        .ok_or("the page reported no viewport")?
        .to_string())
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
