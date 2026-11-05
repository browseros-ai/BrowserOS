//! Milestone M1 on a real browser: the built binary, launched the way Chromium launches it,
//! against a headless BrowserOS, driven over MCP by an rmcp client.
//!
//! ```text
//!  BrowserOS --headless --remote-debugging-port=<cdp>  ◀── CDP ──  browseros-claw-sdk
//!                                                                   ▲ POST /mcp: run
//!  this test ── launch JSON {ports: {server, cdp}} ─────────────────┘
//!            ── after SIGTERM: $BROWSERCLAW_DIR/claw-sdk/trace.db
//! ```
//!
//! It needs BrowserOS, not stock Chrome: core lists and opens tabs with BrowserOS-only CDP
//! (`Browser.getTabs`, `Browser.createTab`). It runs only when `CLAW_SDK_E2E_BROWSER` names the
//! browser binary, e.g. `/Applications/BrowserOS.app/Contents/MacOS/BrowserOS`, and passes
//! without doing anything otherwise, so CI without a browser stays green.

use std::env;
use std::error::Error;
use std::fs::{self, File};
use std::net::{Ipv4Addr, TcpListener};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use browseros_trace::{Delivery, Effect, Outcome, RunId, RunStatus, TraceStore};
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
use tokio::net::TcpStream;
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
    fn start(browser: &Path) -> Result<Self, Box<dyn Error>> {
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
        let neo = Command::new(BIN)
            .arg("--config")
            .arg(&launch)
            .env_clear()
            .env("HOME", dir.path().join("home"))
            .env("BROWSERCLAW_DIR", dir.path().join("state"))
            .env("BROWSERCLAW_HOSTED_MODE", "1")
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
    let mut rig = Rig::start(Path::new(&browser))?;
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
    client.cancel().await?;

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
