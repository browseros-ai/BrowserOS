//! Milestones M1 and M2 on a real browser: the built binary, launched the way Chromium launches
//! it, against a headless BrowserOS, driven over MCP by an rmcp client, and read back over the
//! host socket the way zuzu reads it, including a host's lock on a tab that is then closed from
//! outside Neo.
//!
//! ```text
//!  BrowserOS --headless --remote-debugging-port=<cdp>  ◀── CDP ──  browseros-claw-sdk
//!                                                                   ▲ POST /mcp: run
//!  this test ── launch JSON {ports: {server, cdp}} ─────────────────┤
//!            ── GET /v1/runs/{id} on CLAW_HOST_SOCKET, bearer token ┘
//!            ── after SIGTERM: $BROWSERCLAW_DIR/claw-sdk/trace.db
//! ```
//!
//! It needs BrowserOS, not stock Chrome: core lists and opens tabs with BrowserOS-only CDP
//! (`Browser.getTabs`, `Browser.createTab`). It runs only when `CLAW_SDK_E2E_BROWSER` names the
//! browser binary, e.g. `/Applications/BrowserOS.app/Contents/MacOS/BrowserOS`, and passes
//! without doing anything otherwise, so CI without a browser stays green.
//!
//! No test here has a CAPTCHA provider key, and none reaches a provider. One Neo runs without a
//! key, as a deployment without one does. The other gets a placeholder key file, which turns the
//! solver on, and a provider URL on a closed local port: its page holds no CAPTCHA, so the solve
//! ends at detection, which is the real crate on the real browser.

use std::env;
use std::error::Error;
use std::fs::{self, File};
use std::net::{Ipv4Addr, TcpListener};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use browseros_trace::{Delivery, Effect, Outcome, RunId, RunStatus, TraceStore};
use claw_sdk::test_support::CLOSED_XVNC;
use rmcp::ServiceExt;
use rmcp::model::{
    CallToolRequestParams, CallToolResult, ClientInfo, MetaObject, ProtocolVersion,
    RequestMetaObject,
};
use rmcp::service::{RoleClient, RunningService};
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use serde_json::{Value, json};
use tempfile::TempDir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpStream, UnixStream};
use tokio::process::{Child, Command};
use tokio::time;

type TestResult = Result<(), Box<dyn Error>>;
type Client = RunningService<RoleClient, ClientInfo>;

const BIN: &str = env!("CARGO_BIN_EXE_browseros-claw-sdk");
/// Names the BrowserOS binary; unset, the test does nothing.
const BROWSER_ENV: &str = "CLAW_SDK_E2E_BROWSER";
/// The `_meta` key a host's ids and the run's outcome ride under.
const RUN_META: &str = "com.browseros.neo/run";
/// A cold BrowserOS start plus the link's first connect, on a loaded machine.
const PATIENCE: Duration = Duration::from_secs(60);
/// The whole drive. A step that hangs fails the test here, so the guards still kill both
/// processes: a killed test process runs no `Drop`.
const TEST_BUDGET: Duration = Duration::from_secs(240);
/// A page whose `<title>` the tab strip shows once it loads.
const PAGE: &str = "data:text/html,<title>Neo e2e</title><p>claw-sdk end to end</p>";
const TITLE: &str = "Neo e2e";
/// The page a host locks, then the owner closes.
const LOCKED_PAGE: &str = "data:text/html,<title>Neo e2e lock</title><p>the owner's now</p>";
const LOCKED_TITLE: &str = "Neo e2e lock";
/// The bearer token the host presents on the host socket; browseros-service wants 32 bytes.
const HOST_TOKEN: &str = "e2e-host-token-0123456789-abcdefghij";

/// Whether Neo starts with a CAPTCHA provider key.
#[derive(Clone, Copy)]
enum CaptchaKey {
    /// No key: `neo.solveCaptcha` is off.
    Absent,
    /// A placeholder key file and a provider URL nothing listens on.
    Placeholder,
}

/// The two processes and their scratch directory. Fields drop in order: both children are
/// killed (`kill_on_drop`) before the directory they write into is removed, on every path out
/// of the test, a failed assertion included.
struct Rig {
    neo: Child,
    _browser: Child,
    server_port: u16,
    cdp_port: u16,
    dir: TempDir,
}

impl Rig {
    /// BrowserOS on a fresh profile, then claw-sdk pointed at its CDP port.
    fn start(browser: &Path, captcha: CaptchaKey) -> Result<Self, Box<dyn Error>> {
        let dir = tempfile::tempdir()?;
        let cdp_port = free_port()?;
        let server_port = free_port()?;

        // The flags hosted Neo's image uses minus the display: no bundled server, updater or
        // extensions, which would race claw-sdk for the browser.
        let browser = Command::new(browser)
            .arg("--headless=new")
            .arg(format!(
                "--user-data-dir={}",
                dir.path().join("profile").display()
            ))
            .arg(format!("--remote-debugging-port={cdp_port}"))
            .args([
                "--disable-browseros-server",
                "--disable-browseros-server-updater",
                "--disable-browseros-extensions",
                "--no-first-run",
                "--no-default-browser-check",
                "about:blank",
            ])
            .stdin(Stdio::null())
            .stdout(log_sink(&dir.path().join("browser.log"))?)
            .stderr(log_sink(&dir.path().join("browser.log"))?)
            .kill_on_drop(true)
            .spawn()?;

        let launch = dir.path().join("server_config.json");
        fs::write(
            &launch,
            format!(
                r#"{{"ports":{{"server":{server_port},"cdp":{cdp_port}}},"directories":{{"resources":"r"}}}}"#
            ),
        )?;
        let token_file = dir.path().join("host-token");
        fs::write(&token_file, format!("{HOST_TOKEN}\n"))?;
        let mut neo = Command::new(BIN);
        neo.arg("--config")
            .arg(&launch)
            .env_clear()
            .env("HOME", dir.path().join("home"))
            .env("BROWSERCLAW_DIR", dir.path().join("state"))
            .env("BROWSERCLAW_HOSTED_MODE", "1")
            .env("CLAW_HOST_TOKEN_FILE", &token_file)
            .env("CLAW_HOST_SOCKET", dir.path().join("host.sock"))
            // The headless browser has no Xvnc, and 5900 may be a developer's Screen Sharing.
            .env("CLAW_XVNC_ADDR", CLOSED_XVNC.addr.to_string());
        if let CaptchaKey::Placeholder = captcha {
            let key_file = dir.path().join("capsolver-key");
            fs::write(&key_file, "placeholder")?;
            neo.env("BROWSEROS_CAPSOLVER_API_KEY_FILE", &key_file)
                .env("BROWSEROS_CAPSOLVER_API_URL", "http://127.0.0.1:9");
        }
        let neo = neo
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(log_sink(&dir.path().join("neo.log"))?)
            .kill_on_drop(true)
            .spawn()?;
        Ok(Self {
            neo,
            _browser: browser,
            server_port,
            cdp_port,
            dir,
        })
    }

    /// One GET on the host socket, as zuzu sends it, with the bearer token when given.
    async fn host_get(&self, path: &str, token: Option<&str>) -> Result<String, Box<dyn Error>> {
        self.host_call("GET", path, token, None).await
    }

    /// One request on the host socket, as zuzu sends it, with the bearer token when given.
    async fn host_call(
        &self,
        method: &str,
        path: &str,
        token: Option<&str>,
        body: Option<&Value>,
    ) -> Result<String, Box<dyn Error>> {
        let mut stream = UnixStream::connect(self.dir.path().join("host.sock")).await?;
        let auth = token.map_or_else(String::new, |token| {
            format!("Authorization: Bearer {token}\r\n")
        });
        let body = body.map(Value::to_string).unwrap_or_default();
        let request = format!(
            "{method} {path} HTTP/1.1\r\nHost: neo\r\n{auth}Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        stream.write_all(request.as_bytes()).await?;
        let mut response = String::new();
        time::timeout(PATIENCE, stream.read_to_string(&mut response)).await??;
        Ok(response)
    }

    fn trace_path(&self) -> PathBuf {
        self.dir
            .path()
            .join("state")
            .join("claw-sdk")
            .join("trace.db")
    }

    /// `/system/ready` answers 200 once the browser link is up.
    async fn until_ready(&self) -> TestResult {
        time::timeout(PATIENCE, async {
            loop {
                let ready = get(self.server_port, "/system/ready").await;
                if ready.is_ok_and(|response| response.starts_with("HTTP/1.1 200")) {
                    return;
                }
                time::sleep(Duration::from_millis(100)).await;
            }
        })
        .await
        .map_err(|_elapsed| format!("not ready within {PATIENCE:?}").into())
    }

    /// A client on the legacy protocol, as most hosts speak it today.
    async fn client(&self) -> Result<Client, Box<dyn Error>> {
        let mut info = ClientInfo::default();
        info.protocol_version = ProtocolVersion::V_2025_11_25;
        let url = format!("http://127.0.0.1:{}/mcp", self.server_port);
        let transport = StreamableHttpClientTransport::from_config(
            StreamableHttpClientTransportConfig::with_uri(url),
        );
        Ok(info.serve(transport).await?)
    }

    /// SIGTERM, as Chromium stops it, and the exit code; the trace writer commits on this path.
    async fn stop_neo(&mut self) -> Result<Option<i32>, Box<dyn Error>> {
        let pid = self.neo.id().ok_or("claw-sdk exited before SIGTERM")?;
        let kill = Command::new("kill")
            .args(["-TERM", &pid.to_string()])
            .status()
            .await?;
        if !kill.success() {
            return Err(format!("kill -TERM {pid} failed").into());
        }
        Ok(time::timeout(PATIENCE, self.neo.wait()).await??.code())
    }

    /// Both processes' output, for a failure message.
    fn logs(&self) -> String {
        let read = |name: &str| {
            fs::read_to_string(self.dir.path().join(name))
                .unwrap_or_else(|error| format!("<{name}: {error}>"))
        };
        format!(
            "--- claw-sdk ---\n{}\n--- BrowserOS ---\n{}",
            read("neo.log"),
            read("browser.log")
        )
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn placeholder_built_ins_drive_a_real_browseros_page() -> TestResult {
    let Some(browser) = env::var_os(BROWSER_ENV) else {
        skipped();
        return Ok(());
    };
    let mut rig = Rig::start(Path::new(&browser), CaptchaKey::Absent)?;
    let failure = match time::timeout(TEST_BUDGET, drive(&mut rig)).await {
        Ok(Ok(())) => return Ok(()),
        Ok(Err(error)) => error.to_string(),
        Err(_elapsed) => format!("the drive did not finish within {TEST_BUDGET:?}"),
    };
    Err(format!("{failure}\n{}", rig.logs()).into())
}

async fn drive(rig: &mut Rig) -> TestResult {
    rig.until_ready().await?;
    let client = rig.client().await?;
    let code = format!(
        r"
        const p = await context.newPage();
        await p.goto({PAGE:?});
        const t = await p.title();
        await p.close();
        return t;"
    );
    let meta = json!({ "requestId": "e2e-1", "taskId": "e2e" });

    let first = run(&client, &code, &meta).await?;
    assert_eq!(
        first.structured_content,
        Some(json!({ "value": TITLE })),
        "the tab strip's title of the page BrowserOS loaded: {first:?}"
    );
    let first_meta = outcome(&first)?;
    assert_eq!(first_meta.get("status"), Some(&json!("ok")), "{first_meta}");
    let id: RunId = first_meta
        .get("id")
        .and_then(Value::as_str)
        .ok_or("every result names its run")?
        .parse()?;

    let targets = get(rig.cdp_port, "/json/list").await?;
    assert!(
        !targets.contains(TITLE),
        "page.close closed the tab in the browser: {targets}"
    );

    let repeat = run(&client, &code, &meta).await?;
    assert_eq!(
        outcome(&repeat)?.get("id").and_then(Value::as_str),
        Some(id.to_string().as_str()),
        "a repeated requestId replays the first run instead of running again"
    );
    assert_eq!(
        repeat.structured_content, first.structured_content,
        "with its value"
    );
    solve_captcha_without_a_key(&client).await?;
    let (locked_run, locked_tab) = a_hosts_lock_ends_when_its_tab_closes(rig, &client).await?;
    client.cancel().await?;
    read_back_over_the_host_socket(rig, id).await?;

    assert_eq!(rig.stop_neo().await?, Some(0), "SIGTERM is a normal stop");
    let store = TraceStore::open(&rig.trace_path()).await?;
    let detail = store.run(id).await?.ok_or("the run is in the trace")?;
    assert_eq!(detail.summary.status, RunStatus::Ok);
    assert_eq!(
        (
            detail.summary.request_id.map(|request| request.to_string()),
            detail.summary.task_id.map(|task| task.to_string()),
        ),
        (Some("e2e-1".to_owned()), Some("e2e".to_owned())),
        "the host's ids are recorded with the run"
    );
    let ops: Vec<_> = detail
        .operations
        .iter()
        .map(|op| (op.name.as_str(), op.effect, op.outcome, op.delivery))
        .collect();
    assert_eq!(
        ops,
        [
            (
                "context.newPage",
                Effect::Write,
                Some(Outcome::Ok),
                Some(Delivery::Done)
            ),
            (
                "page.goto",
                Effect::Write,
                Some(Outcome::Ok),
                Some(Delivery::Done)
            ),
            (
                "page.title",
                Effect::Read,
                Some(Outcome::Ok),
                Some(Delivery::NotSent)
            ),
            (
                "page.close",
                Effect::Write,
                Some(Outcome::Ok),
                Some(Delivery::Done)
            ),
        ],
        "four ops, once each (the replay ran nothing): writes done once sent, the read never sent"
    );
    new_pages_load_is_a_child_op_on_its_tab(&store, locked_run, locked_tab).await
}

/// The trace of the run that opened the locked tab: `context.newPage({ url })` and, under it,
/// the load as `page.goto` on the new tab, where the door's rules and the trace see the tab.
async fn new_pages_load_is_a_child_op_on_its_tab(
    store: &TraceStore,
    run: RunId,
    tab: i64,
) -> TestResult {
    let opened = store
        .run(run)
        .await?
        .ok_or("the run that opened the locked tab is in the trace")?;
    let ops: Vec<(&str, Option<i64>, bool)> = opened
        .operations
        .iter()
        .map(|op| {
            (
                op.name.as_str(),
                op.tab.map(|tab| tab.0),
                op.parent.is_some(),
            )
        })
        .collect();
    assert_eq!(
        ops,
        [
            ("context.newPage", None, false),
            ("page.goto", Some(tab), true)
        ],
        "newPage loads its URL as a child op on the new tab, where the door's rules see it"
    );
    Ok(())
}

/// `neo.solveCaptcha` on a Neo with no provider key: the real prelude's `neo`, refused for the
/// host to fix, before anything reaches the page.
async fn solve_captcha_without_a_key(client: &Client) -> TestResult {
    let code = r"
        try {
          await neo.solveCaptcha(page);
          return 'solved without a key';
        } catch (e) {
          return { code: e.code, owner: e.owner, delivery: e.delivery };
        }";
    let result = run(client, code, &json!({ "taskId": "e2e-captcha-off" })).await?;
    assert_eq!(
        result.structured_content,
        Some(json!({ "value": {
            "code": "captcha_unavailable",
            "owner": "host",
            "delivery": "not_sent",
        } })),
        "{result:?}"
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn solve_captcha_detects_a_clear_page_on_a_real_browseros() -> TestResult {
    let Some(browser) = env::var_os(BROWSER_ENV) else {
        skipped();
        return Ok(());
    };
    let rig = Rig::start(Path::new(&browser), CaptchaKey::Placeholder)?;
    let failure = match time::timeout(TEST_BUDGET, solve_on_a_clear_page(&rig)).await {
        Ok(Ok(())) => return Ok(()),
        Ok(Err(error)) => error.to_string(),
        Err(_elapsed) => format!("the drive did not finish within {TEST_BUDGET:?}"),
    };
    Err(format!("{failure}\n{}", rig.logs()).into())
}

/// One solve on a page with no CAPTCHA: the solver's detection runs through claw-sdk's door on
/// the real browser and answers `already_clear` as a value, without a provider.
async fn solve_on_a_clear_page(rig: &Rig) -> TestResult {
    rig.until_ready().await?;
    let client = rig.client().await?;
    let code = format!(
        r"
        const p = await context.newPage();
        await p.goto({PAGE:?});
        const solved = await neo.solveCaptcha(p);
        await p.close();
        return solved;"
    );
    let result = run(&client, &code, &json!({ "taskId": "e2e-captcha" })).await?;
    let solved = result
        .structured_content
        .as_ref()
        .and_then(|content| content.get("value"))
        .ok_or_else(|| format!("the solve answered a value: {result:?}"))?;
    let field = |name: &str| solved.get(name).cloned().unwrap_or(Value::Null);
    assert_eq!(
        (
            field("verdict"),
            field("reason"),
            field("type"),
            field("paidTasks")
        ),
        (
            json!("already_clear"),
            json!("no_captcha"),
            Value::Null,
            json!(0)
        ),
        "nothing blocks the page, and no provider was asked: {solved}"
    );
    assert_eq!(
        field("message"),
        json!("No CAPTCHA blocks the page (no CAPTCHA was found); carry on with the task.")
    );
    client.cancel().await?;
    Ok(())
}

/// A host locks a tab the agent opened with `context.newPage({ url })`, the owner closes it from
/// outside Neo (Chromium's own `/json/close`), and the lock ends with the tab: the close reached
/// browseros-service from BrowserOS's target events. Returns the run that opened the tab and
/// the tab, for the trace check after the stop.
async fn a_hosts_lock_ends_when_its_tab_closes(
    rig: &Rig,
    client: &Client,
) -> Result<(RunId, i64), Box<dyn Error>> {
    let code = format!("return await context.newPage({{ url: {LOCKED_PAGE:?} }});");
    let opened = run(client, &code, &json!({ "taskId": "e2e-lock" })).await?;
    let page = opened
        .structured_content
        .as_ref()
        .and_then(|content| content.get("value"))
        .ok_or_else(|| format!("the run returns its page: {opened:?}"))?;
    let tab = page
        .get("tab")
        .and_then(Value::as_i64)
        .ok_or_else(|| format!("a page carries its tab: {page}"))?;
    let run_id: RunId = outcome(&opened)?
        .get("id")
        .and_then(Value::as_str)
        .ok_or("every result names its run")?
        .parse()?;

    let take = json!({ "tabId": tab, "ttlSeconds": 600 });
    let locked = rig
        .host_call("POST", "/v1/locks", Some(HOST_TOKEN), Some(&take))
        .await?;
    let (head, body) = locked.split_once("\r\n\r\n").ok_or("an HTTP response")?;
    assert!(
        head.starts_with("HTTP/1.1 200"),
        "the host locks the tab: {locked}"
    );
    let lock: Value = serde_json::from_str(body)?;
    let id = lock
        .get("id")
        .and_then(Value::as_str)
        .ok_or("a lock has an id")?;

    let listed = get(rig.cdp_port, "/json/list").await?;
    let (_, targets) = listed.split_once("\r\n\r\n").ok_or("an HTTP response")?;
    let targets: Value = serde_json::from_str(targets)?;
    let target = targets
        .as_array()
        .and_then(|targets| {
            targets
                .iter()
                .find(|target| target.get("title").and_then(Value::as_str) == Some(LOCKED_TITLE))
        })
        .and_then(|target| target.get("id"))
        .and_then(Value::as_str)
        .ok_or_else(|| format!("the locked page is a target: {listed}"))?;
    let closed = get(rig.cdp_port, &format!("/json/close/{target}")).await?;
    assert!(
        closed.starts_with("HTTP/1.1 200"),
        "Chromium closes the tab: {closed}"
    );

    time::timeout(PATIENCE, async {
        loop {
            let read = rig
                .host_get(&format!("/v1/locks/{id}"), Some(HOST_TOKEN))
                .await?;
            let (_, body) = read.split_once("\r\n\r\n").ok_or("an HTTP response")?;
            let lock: Value = serde_json::from_str(body)?;
            if lock.get("releasedAt").is_some_and(Value::is_i64) {
                return Ok::<_, Box<dyn Error>>(());
            }
            time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .map_err(|_elapsed| format!("the lock did not end within {PATIENCE:?} of its tab closing"))??;
    Ok((run_id, tab))
}

/// The run as zuzu reads it over the host socket: refused without the bearer token, and with it
/// the run the trace recorded, with its four ops.
async fn read_back_over_the_host_socket(rig: &Rig, id: RunId) -> TestResult {
    let refused = rig.host_get(&format!("/v1/runs/{id}"), None).await?;
    assert!(
        refused.starts_with("HTTP/1.1 401"),
        "the host socket wants the bearer token: {refused}"
    );
    // `wait` holds the answer until the run ended and every screenshot job it made was
    // answered, so `complete` says browseros-service's camera took the writer's jobs here.
    let recorded = rig
        .host_get(&format!("/v1/runs/{id}?wait=30"), Some(HOST_TOKEN))
        .await?;
    let (head, body) = recorded.split_once("\r\n\r\n").ok_or("an HTTP response")?;
    assert!(head.starts_with("HTTP/1.1 200"), "{recorded}");
    let host_run: Value = serde_json::from_str(body)?;
    let field = |name: &str| host_run.get(name).cloned().unwrap_or(Value::Null);
    assert_eq!(
        (field("id"), field("status"), field("complete")),
        (json!(id.to_string()), json!("ok"), json!(true)),
        "the host reads the run as the trace recorded it: {host_run}"
    );
    let names: Vec<&str> = host_run
        .get("operations")
        .and_then(Value::as_array)
        .ok_or("a run lists its operations")?
        .iter()
        .filter_map(|op| op.get("name").and_then(Value::as_str))
        .collect();
    assert_eq!(
        names,
        ["context.newPage", "page.goto", "page.title", "page.close"]
    );
    Ok(())
}

/// `run` with `_meta["com.browseros.neo/run"] = meta`.
async fn run(client: &Client, code: &str, meta: &Value) -> Result<CallToolResult, Box<dyn Error>> {
    let Value::Object(arguments) = json!({ "code": code }) else {
        return Err("arguments are an object".into());
    };
    let mut object = MetaObject::new();
    object.insert(RUN_META.to_owned(), meta.clone());
    let mut params = CallToolRequestParams::new("run").with_arguments(arguments);
    params.meta = Some(RequestMetaObject(object));
    Ok(time::timeout(PATIENCE, client.call_tool(params)).await??)
}

/// `_meta["com.browseros.neo/run"]` of a result.
fn outcome(result: &CallToolResult) -> Result<&Value, Box<dyn Error>> {
    result
        .meta
        .as_ref()
        .and_then(|meta| meta.get(RUN_META))
        .ok_or_else(|| "every result carries the run's _meta".into())
}

/// One plain HTTP/1.1 GET on 127.0.0.1: the head and a `Content-Length` body. Chromium's
/// remote-debugging HTTP server ignores `Connection: close` and keeps the socket open, so
/// reading to EOF would never end.
async fn get(port: u16, path: &str) -> Result<String, Box<dyn Error>> {
    let mut stream = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).await?;
    let request = format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n");
    stream.write_all(request.as_bytes()).await?;
    let mut response = Vec::new();
    let mut chunk = [0_u8; 8192];
    loop {
        let read = stream.read(&mut chunk).await?;
        response.extend_from_slice(chunk.get(..read).unwrap_or_default());
        if read == 0 || complete(&response) {
            return Ok(String::from_utf8_lossy(&response).into_owned());
        }
    }
}

/// Whether `response` holds its whole head and as many body bytes as `Content-Length` says.
fn complete(response: &[u8]) -> bool {
    let text = String::from_utf8_lossy(response);
    let Some((head, body)) = text.split_once("\r\n\r\n") else {
        return false;
    };
    head.lines()
        .filter_map(|line| line.split_once(':'))
        .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, length)| length.trim().parse::<usize>().ok())
        .is_some_and(|length| body.len() >= length)
}

/// A port nothing listens on right now. Another process could take it before its user binds;
/// that race is accepted for a test.
fn free_port() -> Result<u16, Box<dyn Error>> {
    Ok(TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?
        .local_addr()?
        .port())
}

/// A child's output, appended to `path`.
fn log_sink(path: &Path) -> Result<Stdio, Box<dyn Error>> {
    Ok(Stdio::from(
        File::options().create(true).append(true).open(path)?,
    ))
}

#[expect(
    clippy::print_stderr,
    reason = "a skipped e2e says so in the test output; nothing else reads it"
)]
fn skipped() {
    eprintln!("skipped: set {BROWSER_ENV} to a BrowserOS binary to run this test");
}
