//! A real BrowserOS neo for end-to-end tests: a headless browser on a throwaway profile, one
//! tab the test drives over its own CDP session, a local fixture server, and Neo runs against
//! that tab. Gated on `NEO_E2E_BROWSER`, the browser binary's path; without it a test passes as
//! a skip.
//!
//! The browser is headless unless `NEO_E2E_HEADED=1`. It always disables the BrowserOS server,
//! since otherwise a browseros-claw-server sidecar outlives it. [`Tab::close`] closes it as Playwright
//! does and fails the test if any of its processes outlives it; a `Tab` dropped without it, as
//! when a test panics first, SIGKILLs the browser's process group.
//!
//! One test binary runs one browser at a time ([`ONE_BROWSER`]), so a timing-bound test (whether
//! a click times out inside its 500 ms, whether the first page call answers in time) runs as
//! close to an idle browser as the machine allows.
//!
//! [`Tab::open`] is the test's setup, and a fresh browser sometimes hangs in it on a loaded
//! machine: within its first seconds its UI thread blocks in macOS's `RunningBoardServices`
//! (`rbs_acquire_appnap_assertion`, waiting in `-[RBSConnection _connection]`) and stops
//! answering for good, so its CDP port never opens, a setup call goes unanswered, or its first
//! page never commits. One in 477 launches hung so at load 45, still hung after 90 s. Each try at
//! the setup is bounded by [`SETUP_TIMEOUT`]; a failed try's browser gets SIGKILL and is replaced
//! once, on a fresh profile. A setup that fails both tries, and a [`Tab::runtime`] the browser
//! never answers, panic with `harness setup:`: the browser failed before Neo ran anything.
//!
//! Each browser keeps its temporary files in its own profile ([`Browser::launch`]), so they go
//! with it: a browser that exits soon after it starts leaves Chromium's temporary directories
//! (`scoped_dir*`, `com.browseros.*`) wherever its temporary directory is.
//!
//! Test crates include this file with `#[path]`. Cargo builds only top-level `tests/*.rs` files
//! as test crates, so it is never one on its own.
#![expect(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    reason = "the browser fixture drives an independent CDP session as the test oracle"
)]

use std::collections::BTreeSet;
use std::env;
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::fmt::{Display, Write as _};
use std::fs;
use std::future::Future;
use std::io;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use browseros_cdp::{CdpClient, ConnectOptions, ReconnectPolicy, SessionId};
use neo_runtime::{PageId, RunOutcome, RunRequest, Runtime};
use serde_json::{Value, json};
use tempfile::TempDir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::{Mutex, MutexGuard};
use tokio::task::JoinHandle;
use tokio::time;
use tokio_util::sync::CancellationToken;
use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Id, Record};
use tracing::subscriber::set_global_default;
use tracing::{Event, Level, Metadata, Subscriber};

/// The browser binary, or `None` (and a note on stderr) when the test should skip. A test that
/// runs also prints the runtime's warnings ([`Warnings`]).
pub fn e2e_browser() -> Option<OsString> {
    // Only the first test of a binary installs it; the rest find it there.
    let _installed = set_global_default(Warnings);
    let binary = env::var_os("NEO_E2E_BROWSER");
    if binary.is_none() {
        let _ = writeln!(
            io::stderr(),
            "skipped: set NEO_E2E_BROWSER to a BrowserOS neo binary"
        );
    }
    binary
}

/// Prints every warning and error the runtime logs to stderr, which the test harness does not
/// capture, so a run against the real browser shows a CDP event the door could not decode
/// (`src/chromium_bridge/cdp.rs` `Raw::decode`): a protocol skew, not Neo's own failure.
struct Warnings;

impl Subscriber for Warnings {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        *metadata.level() <= Level::WARN
    }

    fn new_span(&self, _span: &Attributes<'_>) -> Id {
        Id::from_u64(1)
    }

    fn record(&self, _span: &Id, _values: &Record<'_>) {}

    fn record_follows_from(&self, _span: &Id, _follows: &Id) {}

    fn event(&self, event: &Event<'_>) {
        let mut fields = Fields(String::new());
        event.record(&mut fields);
        let metadata = event.metadata();
        let _ = writeln!(
            io::stderr(),
            "{} {}:{}",
            metadata.level(),
            metadata.target(),
            fields.0
        );
    }

    fn enter(&self, _span: &Id) {}

    fn exit(&self, _span: &Id) {}
}

/// An event's fields as ` name=value` pairs, its message among them.
struct Fields(String);

impl Visit for Fields {
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        let _ = write!(self.0, " {}={value:?}", field.name());
    }
}

/// Opens a visible browser for manual inspection when explicitly requested.
pub fn is_headed() -> bool {
    env::var("NEO_E2E_HEADED").is_ok_and(|value| value == "1")
}

/// What the fixture server serves. A path in `pages` gets its HTML; any other path gets a small
/// page titled with the path; `/echo-headers` answers with the request's head. A path starting
/// with `/slow` answers after `slow_response`, so a
/// navigation to it stays in flight before it commits.
#[derive(Clone, Default)]
pub struct Fixture {
    pub slow_response: Duration,
    pub pages: Vec<(String, String)>,
}

/// Held by every [`Tab`] for its whole life, so the tests of one binary take turns. A tokio
/// mutex, because each `#[tokio::test]` runs on its own runtime and tokio's locks belong to none;
/// it has no poisoning, so a failed test lets the next one run.
static ONE_BROWSER: Mutex<()> = Mutex::const_new(());

/// A tab of a fresh headless BrowserOS neo. The test drives it over its own `driver` session,
/// separate from the one a run attaches, as an outside actor would.
pub struct Tab {
    browser: Browser,
    pub cdp: CdpClient,
    pub origin: String,
    pub target_id: String,
    pub driver: SessionId,
    /// The throwaway profile's download folder, where the browser saves a download it handles
    /// itself (Neo's `behavior: 'default'`), never the user's ~/Downloads.
    pub downloads: PathBuf,
    /// Declared last, so it is released after the browser has gone.
    _turn: MutexGuard<'static, ()>,
}

impl Tab {
    /// Opens `<origin><path>` on a fresh browser and returns once it has committed. A try that
    /// fails ([`try_open`]) is retried once on another fresh browser; two failed tries panic.
    pub async fn open(binary: OsString, fixture: Fixture, path: &str) -> Self {
        let turn = ONE_BROWSER.lock().await;
        let origin = serve_fixture(fixture).await;
        let url = format!("{origin}{path}");
        let mut failures = Vec::new();
        for attempt in 1..=SETUP_TRIES {
            match try_open(&binary, &url).await {
                Ok(Opened {
                    browser,
                    cdp,
                    target_id,
                    driver,
                }) => {
                    let downloads = browser.downloads.clone();
                    return Self {
                        browser,
                        cdp,
                        origin,
                        target_id,
                        driver,
                        downloads,
                        _turn: turn,
                    };
                }
                Err(failure) => {
                    let _ = writeln!(
                        io::stderr(),
                        "harness setup: try {attempt} of {SETUP_TRIES} failed: {failure}"
                    );
                    failures.push(failure);
                }
            }
        }
        panic!(
            "harness setup: no browser opened {url} in {SETUP_TRIES} tries, each on a fresh \
             profile: {}",
            failures.join("; ")
        );
    }

    /// A [`Runtime`] on this tab's browser: what every run starts from. The browser has
    /// [`RUN_TIMEOUT`] to answer `Runtime::connect`'s check; a browser that hung after setup
    /// panics with `harness setup:` here, before any script runs, rather than at the client's
    /// own request timeout.
    pub async fn runtime(&self) -> Runtime {
        time::timeout(RUN_TIMEOUT, Runtime::connect(self.cdp.clone()))
            .await
            .unwrap_or_else(|_| {
                panic!(
                    "harness setup: Runtime::connect: the browser did not answer within \
                     {RUN_TIMEOUT:?}"
                )
            })
            .unwrap_or_else(|error| panic!("harness setup: Runtime::connect: {error}"))
    }

    /// Starts `code` as a Neo run against this tab; it reads the tab while the test drives it.
    pub async fn start_run(&self, code: String) -> JoinHandle<RunOutcome> {
        self.start_run_within(code, RUN_TIMEOUT).await
    }

    /// [`Tab::start_run`] with the run's own deadline.
    pub async fn start_run_within(
        &self,
        code: String,
        timeout: Duration,
    ) -> JoinHandle<RunOutcome> {
        self.start_run_cancellable(code, timeout, CancellationToken::new())
            .await
    }

    /// [`Tab::start_run_within`] that `cancel` cancels, as a host cancels a run: for a test that
    /// watches the tab while the run is still live, then ends it.
    pub async fn start_run_cancellable(
        &self,
        code: String,
        timeout: Duration,
        cancel: CancellationToken,
    ) -> JoinHandle<RunOutcome> {
        let runtime = self.runtime().await;
        let page = PageId::new(self.target_id.clone());
        tokio::spawn(async move {
            runtime
                .run(RunRequest {
                    code,
                    page,
                    timeout,
                    cancel,
                    artifacts: None,
                })
                .await
        })
    }

    /// Evaluates `expression` in the page's main world and returns its value as JSON.
    pub async fn evaluate(&self, expression: &str) -> Value {
        let reply = self
            .cdp
            .send(
                "Runtime.evaluate",
                json!({ "expression": expression, "returnByValue": true }),
                Some(&self.driver),
            )
            .await
            .unwrap_or_else(|error| panic!("{expression}: {error}"));
        assert!(
            reply.get("exceptionDetails").is_none(),
            "{expression} threw: {reply}"
        );
        reply["result"]["value"].clone()
    }

    /// Navigates to `path` on the fixture's origin and returns once it has committed, within
    /// the suite's run bound.
    pub async fn navigate(&self, path: &str) {
        let url = format!("{}{path}", self.origin);
        let deadline = Instant::now() + RUN_TIMEOUT;
        self.cdp
            .send("Page.navigate", json!({ "url": url }), Some(&self.driver))
            .await
            .unwrap_or_else(|error| panic!("navigate to {url}: {error}"));
        wait_for_url(&self.cdp, &self.driver, &url, deadline)
            .await
            .unwrap_or_else(|failure| panic!("{failure}"));
    }

    /// The browser's CDP port, for a test that reads its HTTP endpoints, as `/json/protocol`.
    pub fn port(&self) -> u16 {
        self.browser.port
    }

    /// Closes the browser ([`Browser::close`]), failing the test if any of its processes
    /// outlived it, or its download folder outlived the profile it lives in. The turn, still in
    /// `self`, is released when this returns.
    pub async fn close(self) {
        let Self {
            mut browser,
            cdp,
            downloads,
            ..
        } = self;
        browser.close(cdp).await;
        // Dropping the browser removes its profile, and every download in it.
        drop(browser);
        assert!(
            !downloads.exists(),
            "{} outlived the browser's throwaway profile",
            downloads.display()
        );
    }
}

/// What one successful try at [`Tab::open`]'s setup leaves: the browser, the test's connection to
/// it, and the committed tab with its driver session.
struct Opened {
    browser: Browser,
    cdp: CdpClient,
    target_id: String,
    driver: SessionId,
}

/// One try at [`Tab::open`]'s setup, all of it within [`SETUP_TIMEOUT`]: a fresh browser, its CDP
/// port, the test's connection, the tab, its driver session, the commit of `url` and an answered
/// `Runtime::connect`. A failed try disconnects the client and drops the browser, which SIGKILLs
/// its process group and removes its profile.
async fn try_open(binary: &OsStr, url: &str) -> Result<Opened, String> {
    let deadline = Instant::now() + SETUP_TIMEOUT;
    let mut browser = Browser::launch(binary);
    browser.port = browser.published_port(deadline).await?;
    let defaults = ConnectOptions::new(browser.port);
    let connect = CdpClient::connect(ConnectOptions {
        host: Some("127.0.0.1".to_string()),
        // The default exits the test process once the browser is gone at teardown.
        reconnect_policy: ReconnectPolicy::KeepTrying,
        // A loaded machine can stall a live browser for longer than the default 10 s; the
        // keepalive then marks the link lost and fails every request in flight with "CDP
        // connection lost" (a SIGSTOPped browser reproduces it at 30 + 10 s). A dead browser
        // closes the socket at once anyway, so each request's own timeout is the bound.
        keepalive_timeout: defaults.request_timeout,
        ..defaults
    });
    let cdp = within(deadline, "connect to the browser", connect).await?;
    match open_tab(&cdp, url, deadline).await {
        Ok((target_id, driver)) => Ok(Opened {
            browser,
            cdp,
            target_id,
            driver,
        }),
        Err(failure) => {
            // Otherwise `KeepTrying` redials the killed browser until the test ends.
            cdp.disconnect().await;
            Err(failure)
        }
    }
}

/// Opens the tab on `url`, attaches the driver session, waits for the commit and checks that the
/// browser answers a run's first call, by `deadline`.
async fn open_tab(
    cdp: &CdpClient,
    url: &str,
    deadline: Instant,
) -> Result<(String, SessionId), String> {
    let create = cdp.send("Target.createTarget", json!({ "url": url }), None);
    let created = within(deadline, "open a tab", create).await?;
    let target_id = created["targetId"]
        .as_str()
        .ok_or_else(|| format!("Target.createTarget answered {created}"))?
        .to_string();
    let attach = cdp.send(
        "Target.attachToTarget",
        json!({ "targetId": target_id, "flatten": true }),
        None,
    );
    let attached = within(deadline, "attach the driver session", attach).await?;
    let driver = SessionId::from(
        attached["sessionId"]
            .as_str()
            .ok_or_else(|| format!("Target.attachToTarget answered {attached}"))?,
    );
    wait_for_url(cdp, &driver, url, deadline).await?;
    // The check every run starts with ([`Tab::runtime`]): a browser that hangs right after its
    // first commit fails here, while setup can still replace it.
    within(deadline, "Runtime::connect", Runtime::connect(cdp.clone())).await?;
    Ok((target_id, driver))
}

/// `step`'s result, or a failure naming `what` if `deadline` passes first.
async fn within<T, E: Display>(
    deadline: Instant,
    what: &str,
    step: impl Future<Output = Result<T, E>>,
) -> Result<T, String> {
    match time::timeout(deadline.saturating_duration_since(Instant::now()), step).await {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(error)) => Err(format!("{what}: {error}")),
        Err(_) => Err(format!(
            "{what}: no answer within the setup's {SETUP_TIMEOUT:?}"
        )),
    }
}

/// Waits until the tab's committed URL is `url`, or fails once `deadline` passes. A read can fail
/// for a few milliseconds while a cross-document navigation commits ("Not attached to an active
/// page"), so failures retry. Each read is bounded by the deadline too, so a browser that stopped
/// answering fails the wait at its deadline, not at the client's request timeout after it.
async fn wait_for_url(
    cdp: &CdpClient,
    session: &SessionId,
    url: &str,
    deadline: Instant,
) -> Result<(), String> {
    loop {
        let read = cdp.send("Page.getNavigationHistory", json!({}), Some(session));
        let current =
            match time::timeout(deadline.saturating_duration_since(Instant::now()), read).await {
                Ok(Ok(history)) => history["currentIndex"]
                    .as_u64()
                    .and_then(|index| usize::try_from(index).ok())
                    .map_or(Value::Null, |index| {
                        history["entries"][index]["url"].clone()
                    }),
                Ok(Err(error)) => json!(error.to_string()),
                Err(_) => json!("no answer to Page.getNavigationHistory"),
            };
        if current == url {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(format!("the tab never committed {url}; it is at {current}"));
        }
        time::sleep(Duration::from_millis(20)).await;
    }
}

/// Serves `fixture` on a loopback port and returns the origin.
async fn serve_fixture(fixture: Fixture) -> String {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .unwrap_or_else(|error| panic!("bind the fixture server: {error}"));
    let address = listener
        .local_addr()
        .unwrap_or_else(|error| panic!("read the fixture server's port: {error}"));
    let mut listeners = vec![listener];
    // `localhost` can resolve to either family. Both listeners serve this same fixture.
    if let Ok(localhost) = TcpListener::bind(("::1", address.port())).await {
        listeners.push(localhost);
    }
    let fixture = Arc::new(fixture);
    for listener in listeners {
        let fixture = Arc::clone(&fixture);
        tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                let fixture = Arc::clone(&fixture);
                tokio::spawn(async move {
                    let mut request = Vec::new();
                    let mut buffer = [0_u8; 1024];
                    while !request.windows(4).any(|part| part == b"\r\n\r\n") {
                        match stream.read(&mut buffer).await {
                            Ok(0) | Err(_) => return,
                            Ok(read) => request.extend_from_slice(&buffer[..read]),
                        }
                    }
                    let head = String::from_utf8_lossy(&request);
                    let path = head.split(' ').nth(1).unwrap_or("/").to_string();
                    if path.starts_with("/slow") {
                        time::sleep(fixture.slow_response).await;
                    }
                    // `/echo-headers` answers with the request's own head, for the headers it sent.
                    let body = if path == "/echo-headers" {
                        head.to_string()
                    } else {
                        fixture
                            .pages
                            .iter()
                            .find(|(page, _)| *page == path)
                            .map_or_else(
                                || format!("<title>{path}</title><h1>{path}</h1>"),
                                |(_, html)| html.clone(),
                            )
                    };
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    // The browser may cancel a request it no longer needs.
                    let _ = stream.write_all(response.as_bytes()).await;
                });
            }
        });
    }
    format!("http://{address}")
}

/// The suite's bound on one run ([`Tab::start_run`]), which a loaded machine needs as much for a
/// fixture's frames to load as a run needs for its calls.
pub const RUN_TIMEOUT: Duration = Duration::from_secs(30);

/// How long one try at [`Tab::open`]'s setup may take, from the launch to the commit: the suite's
/// run bound. A live browser needs a few seconds of it even on a loaded machine; one that hung
/// never answers again, so waiting longer only delays its replacement.
const SETUP_TIMEOUT: Duration = RUN_TIMEOUT;

/// Tries at [`Tab::open`]'s setup, each on a fresh browser and profile: one replacement for a
/// browser that hung while it started.
const SETUP_TRIES: usize = 2;

/// How long [`Browser::close`] waits for the browser to exit after `Browser.close` before it
/// SIGKILLs the browser's process group: Playwright's `closeOrKill` timeout
/// (`DEFAULT_PLAYWRIGHT_TIMEOUT`).
const CLOSE_TIMEOUT: Duration = Duration::from_secs(30);

/// How long a helper may outlive a browser that closed gracefully before [`Browser::close`]
/// reports it as a survivor and SIGKILLs it. `crates/neo-sdk/neo-quickjs-injected/tests/parity/harness.mjs`
/// uses the same value.
const HELPER_GRACE: Duration = Duration::from_secs(5);

/// How often [`Browser::close`] samples the processes left.
const POLL: Duration = Duration::from_millis(20);

/// How long a browser dropped unclosed has to answer its HTTP endpoint before it is reported as
/// hung ([`answers`]); a live one answers in milliseconds.
const LIVENESS_TIMEOUT: Duration = Duration::from_secs(2);

/// A BrowserOS neo on a throwaway profile, leading a process group of its own. Every
/// helper it starts (GPU, utility, renderers) joins that group (`ps -o pid,pgid` on
/// 155.0.8318.26), so one signal to the group reaches them all. [`Browser::close`] ends it; one
/// dropped without it, as when a test panics first, sends its whole group SIGKILL.
struct Browser {
    child: Child,
    port: u16,
    profile: TempDir,
    /// The profile's download folder ([`seed_downloads`]).
    downloads: PathBuf,
    /// Set by [`Browser::close`], after which `Drop` has nothing left to end.
    closed: bool,
}

impl Browser {
    /// Starts the browser on a throwaway profile, leading its own process group; its CDP port
    /// comes later ([`Browser::published_port`]).
    ///
    /// Chromium creates temporary directories as it starts (the singleton socket's, the bundled
    /// extensions' unpacking), and leaves them behind when it exits early or gets SIGKILL: on a
    /// shared `TMPDIR`, thousands a day across the suites. `MAC_CHROMIUM_TMPDIR` (Chromium's
    /// `GetTempDir` on macOS) and `TMPDIR` (elsewhere) put them in the profile's own `tmp`, which
    /// the profile's removal takes too.
    fn launch(binary: &OsStr) -> Self {
        let profile = tempfile::tempdir()
            .unwrap_or_else(|error| panic!("create a profile directory: {error}"));
        let downloads = seed_downloads(profile.path());
        let temporary = profile.path().join("tmp");
        fs::create_dir(&temporary)
            .unwrap_or_else(|error| panic!("create {}: {error}", temporary.display()));
        let mut command = Command::new(binary);
        command
            .env("MAC_CHROMIUM_TMPDIR", &temporary)
            .env("TMPDIR", &temporary);
        if !is_headed() {
            command.arg("--headless=new");
        }
        let mut child = command
            // Otherwise the browser starts a browseros-claw-server sidecar that outlives it.
            .arg("--disable-browseros-server")
            // Port 0 lets the browser pick a free port and write it to DevToolsActivePort.
            .arg("--remote-debugging-port=0")
            .arg(format!("--user-data-dir={}", profile.path().display()))
            .arg("--no-first-run")
            .arg("about:blank")
            // Its own group, so one SIGKILL reaches every helper: Playwright spawns browsers
            // `detached` for the same reason.
            .process_group(0)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap_or_else(|error| panic!("launch the browser: {error}"));
        // Every group signal below goes to `-pid`. Were the browser in the test binary's group,
        // that would kill `cargo test` with it, so confirm it leads its own before anything
        // relies on it.
        let pid = child.id().to_string();
        let group = Command::new("ps")
            .args(["-o", "pgid=", "-p", &pid])
            .output()
            .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
            .unwrap_or_default();
        if group != pid {
            let _ = child.kill();
            let _ = child.wait();
            panic!(
                "the browser (pid {pid}) is in process group {group:?}, not its own; refusing \
                 group signals"
            );
        }
        // Owned before anything else can fail: a `Browser` dropped unclosed SIGKILLs its group,
        // while a bare `Child` dropped by a panic would leave the browser running. `port` is set
        // once the browser publishes it.
        Self {
            child,
            port: 0,
            profile,
            downloads,
            closed: false,
        }
    }

    /// The CDP port the browser publishes in its profile's `DevToolsActivePort`, or why it never
    /// did by `deadline`. A browser that exits first fails at once.
    async fn published_port(&mut self, deadline: Instant) -> Result<u16, String> {
        let port_file = self.profile.path().join("DevToolsActivePort");
        loop {
            let port = fs::read_to_string(&port_file)
                .ok()
                .and_then(|text| text.lines().next()?.parse::<u16>().ok());
            if let Some(port) = port {
                return Ok(port);
            }
            if let Ok(Some(status)) = self.child.try_wait() {
                return Err(format!(
                    "the browser never opened its CDP port: it exited ({status})"
                ));
            }
            if Instant::now() >= deadline {
                return Err(format!(
                    "the browser never opened its CDP port within {SETUP_TIMEOUT:?}"
                ));
            }
            time::sleep(Duration::from_millis(50)).await;
        }
    }

    /// Closes the browser as Playwright does (`packages/utils/processLauncher.ts`
    /// `gracefullyClose` and `killProcess`, `server/browserType.ts` `closeOrKill`, at
    /// 26137face): `Browser.close` over CDP, then wait for the process to exit, and SIGKILL the
    /// whole group if it has not within [`CLOSE_TIMEOUT`]. Then it waits out the helpers and
    /// SIGKILLs the group again; a process still running after that fails the test.
    ///
    /// Every shutdown prints its timings on stderr as `browser shutdown {…}`, in the parity
    /// harness's format: the browser's exit after `Browser.close` (`forced` when it needed
    /// SIGKILL), each helper's exit after the browser's, and the `survivors` still running after
    /// [`HELPER_GRACE`]. A survivor is reported, not failed: it is Chromium's shutdown, which
    /// Neo has no part in. Measured on 155.0.8318.26, 16 cores: no helper outlived any of 600
    /// graceful closes at load averages up to 32; under 24 CPU burners (load 14 to 71), one
    /// `utility` outlived one of 66, while the browser itself took 10 s to exit. After the
    /// abrupt exit SIGTERM gives, renderers outlived the browser by 15 s in 41 of 180 shutdowns.
    async fn close(&mut self, cdp: CdpClient) {
        self.closed = true;
        let mut helpers = self.processes();
        let stopping = Instant::now();
        // The browser may drop the socket before it answers.
        let _ = time::timeout(CLOSE_TIMEOUT, cdp.send("Browser.close", json!({}), None)).await;
        cdp.disconnect().await;
        while stopping.elapsed() < CLOSE_TIMEOUT && matches!(self.child.try_wait(), Ok(None)) {
            time::sleep(POLL).await;
        }
        let forced = matches!(self.child.try_wait(), Ok(None));
        if forced {
            self.kill_group();
            let _ = writeln!(
                io::stderr(),
                "harness: the browser did not exit within {CLOSE_TIMEOUT:?} of Browser.close and got \
                 SIGKILL: it had stopped answering; a failure in this test may be the browser's \
                 hang, not Neo's"
            );
        }
        let _ = self.child.wait();
        let browser_exit = stopping.elapsed();

        let browser_gone = Instant::now();
        let mut exits = Vec::new();
        let mut alive = self.alive();
        loop {
            helpers.retain(|(pid, kind)| {
                let running = alive.contains(pid);
                if !running {
                    exits.push(json!({
                        "pid": pid,
                        "type": kind,
                        "exitMs": millis(browser_gone.elapsed()),
                    }));
                }
                running
            });
            if alive.is_empty() || browser_gone.elapsed() >= HELPER_GRACE {
                break;
            }
            time::sleep(POLL).await;
            alive = self.alive();
        }
        let survivors: Vec<(&String, String)> = alive
            .iter()
            .map(|pid| {
                let kind = helpers
                    .iter()
                    .find(|(known, _)| known == pid)
                    .map_or_else(|| process_type(pid), |(_, kind)| kind.clone());
                (pid, kind)
            })
            .collect();
        let left = self.kill_all();
        let shutdown = json!({
            "browserExitMs": millis(browser_exit),
            "forced": forced,
            "helpers": exits,
            "survivors": survivors
                .iter()
                .map(|(pid, kind)| json!({ "pid": pid, "type": kind }))
                .collect::<Vec<_>>(),
        });
        let _ = writeln!(io::stderr(), "browser shutdown {shutdown}");
        assert!(
            left.is_empty(),
            "browser processes outlived the test even after SIGKILL: {left:?}"
        );
    }

    /// Every process in the browser's group or started from its profile, the browser excepted,
    /// as (pid, type), from one `ps` listing.
    fn processes(&self) -> Vec<(String, String)> {
        let leader = self.child.id().to_string();
        let profile = self.profile.path().to_string_lossy().into_owned();
        let listing = Command::new("ps")
            .args(["-Ao", "pid=,pgid=,command="])
            .stderr(Stdio::null())
            .output()
            .map(|output| String::from_utf8_lossy(&output.stdout).into_owned())
            .unwrap_or_default();
        listing
            .lines()
            .filter_map(|line| {
                let (pid, rest) = line.trim_start().split_once(' ')?;
                let (group, command) = rest.trim_start().split_once(' ')?;
                let ours = pid != leader && (group == leader || command.contains(&profile));
                ours.then(|| (pid.to_string(), command_type(command)))
            })
            .collect()
    }

    /// The pids still running in the browser's group or from its profile, the browser excepted.
    fn alive(&self) -> BTreeSet<String> {
        let leader = self.child.id().to_string();
        let mut alive: BTreeSet<String> = pgrep(&["-g".as_ref(), leader.as_ref()])
            .into_iter()
            .collect();
        alive.extend(pgrep(&["-f".as_ref(), self.profile.path().as_os_str()]));
        alive.remove(&leader);
        alive
    }

    /// SIGKILL to the browser's whole group, as Playwright's `killProcess` sends it.
    /// [`Browser::launch`] checked that the browser leads that group. Once the browser has
    /// exited, the group's id stays reserved only while a member is alive (POSIX never reuses the
    /// id of a live group), so the signal goes only to a group that still has members: an empty
    /// group's id may be someone else's.
    fn kill_group(&self) {
        let leader = self.child.id().to_string();
        if pgrep(&["-g".as_ref(), leader.as_ref()]).is_empty() {
            return;
        }
        let _ = Command::new("kill")
            .args(["-KILL", "--", &format!("-{}", self.child.id())])
            .stderr(Stdio::null())
            .status();
    }

    /// SIGKILLs the group and anything else started from the profile, then returns what is
    /// still running a second later.
    fn kill_all(&self) -> BTreeSet<String> {
        self.kill_group();
        for pid in self.alive() {
            let _ = Command::new("kill")
                .args(["-KILL", &pid])
                .stderr(Stdio::null())
                .status();
        }
        let deadline = Instant::now() + Duration::from_secs(1);
        let mut left = self.alive();
        while !left.is_empty() && Instant::now() < deadline {
            thread::sleep(POLL);
            left = self.alive();
        }
        left
    }
}

/// A browser [`Browser::close`] never ran for, as when a test panicked first: SIGKILL its group,
/// Playwright's fallback, so nothing outlives the test. A `Drop` cannot fail the test, so what
/// it could not end goes to stderr, and so does a browser that had stopped answering: a test
/// that failed on a browser that hung after setup reads like a Neo failure otherwise.
impl Drop for Browser {
    fn drop(&mut self) {
        if self.closed {
            return;
        }
        let hung = self.port != 0 && !answers(self.port);
        self.kill_group();
        let _ = self.child.wait();
        let left = self.kill_all();
        let _ = writeln!(
            io::stderr(),
            "browser killed without Browser.close; still running: {left:?}"
        );
        if hung {
            let _ = writeln!(
                io::stderr(),
                "harness: the browser had stopped answering CDP (its HTTP endpoint gave no answer \
                 within {LIVENESS_TIMEOUT:?}); a failure above may be the browser's hang, not Neo's"
            );
        }
    }
}

/// Whether the browser's CDP HTTP endpoint answers `/json/version` within [`LIVENESS_TIMEOUT`].
/// Chromium answers it on its UI thread, so a browser that hung there (the module docs) does
/// not.
fn answers(port: u16) -> bool {
    let address = SocketAddr::from(([127, 0, 0, 1], port));
    let Ok(mut stream) = TcpStream::connect_timeout(&address, LIVENESS_TIMEOUT) else {
        return false;
    };
    let _ = stream.set_read_timeout(Some(LIVENESS_TIMEOUT));
    let _ = stream.set_write_timeout(Some(LIVENESS_TIMEOUT));
    let request = b"GET /json/version HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n";
    let mut head = [0_u8; 12];
    stream.write_all(request).is_ok()
        && stream.read_exact(&mut head).is_ok()
        && head.starts_with(b"HTTP/1.1 200")
}

/// Creates the profile's own `downloads` folder and makes it the profile's download folder, in
/// `Default/Preferences` before the browser first reads them, and returns it. A download the
/// browser saves itself (Neo's `behavior: 'default'`) goes to the profile's default folder,
/// which a fresh profile puts at the user's ~/Downloads. "Ask where to save each file" is off, as
/// a headless browser cancels a download that asks.
/// `crates/neo-sdk/neo-quickjs-injected/tests/parity/harness.mjs` seeds its profiles the same way.
fn seed_downloads(profile: &Path) -> PathBuf {
    let downloads = profile.join("downloads");
    let settings = profile.join("Default");
    for folder in [&downloads, &settings] {
        fs::create_dir_all(folder)
            .unwrap_or_else(|error| panic!("create {}: {error}", folder.display()));
    }
    let folder = downloads.to_string_lossy();
    let preferences = json!({
        "download": {
            "default_directory": folder,
            "prompt_for_download": false,
            "directory_upgrade": true,
        },
        "savefile": { "default_directory": folder },
    });
    fs::write(settings.join("Preferences"), preferences.to_string())
        .unwrap_or_else(|error| panic!("write the profile's Preferences: {error}"));
    downloads
}

/// Process ids `pgrep` matches; empty when it matches none or cannot run.
fn pgrep(args: &[&OsStr]) -> Vec<String> {
    Command::new("pgrep")
        .args(args)
        .output()
        .map(|output| {
            String::from_utf8_lossy(&output.stdout)
                .lines()
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// A Chromium process's `--type=` (renderer, gpu-process, utility…), else its executable.
fn command_type(command: &str) -> String {
    match command.split_once("--type=") {
        Some((_, rest)) => rest.split(' ').next().unwrap_or_default().to_string(),
        None => command
            .split(" --")
            .next()
            .and_then(|path| path.rsplit('/').next())
            .filter(|name| !name.is_empty())
            .unwrap_or("already gone")
            .to_string(),
    }
}

/// [`command_type`] for a pid outside the listing [`Browser::processes`] took.
fn process_type(pid: &str) -> String {
    let command = Command::new("ps")
        .args(["-o", "command=", "-p", pid])
        .stderr(Stdio::null())
        .output()
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .unwrap_or_default();
    command_type(&command)
}

/// Whole milliseconds, for the shutdown line.
fn millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}
