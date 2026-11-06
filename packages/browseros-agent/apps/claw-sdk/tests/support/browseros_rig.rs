//! The real-browser rig the BrowserOS tests share: a headless BrowserOS on a fresh profile, the
//! built `browseros-claw-sdk` launched the way Chromium launches it, an MCP client on its agent
//! port, and plain HTTP on its host socket the way zuzu reads it.
//!
//! Each test file includes it as `pub mod`, so the helpers one file does not use are not dead
//! code there.

use std::env;
use std::error::Error;
use std::fs::{self, File};
use std::net::{Ipv4Addr, TcpListener};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

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

/// What every test here returns.
pub type TestResult = Result<(), Box<dyn Error>>;
/// An MCP client on Neo's agent port.
pub type Client = RunningService<RoleClient, ClientInfo>;

const BIN: &str = env!("CARGO_BIN_EXE_browseros-claw-sdk");
/// Names the BrowserOS binary; unset, the tests do nothing.
pub const BROWSER_ENV: &str = "CLAW_SDK_E2E_BROWSER";
/// The `_meta` key a host's ids and the run's outcome ride under.
pub const RUN_META: &str = "com.browseros.neo/run";
/// A cold BrowserOS start plus the link's first connect, on a loaded machine.
pub const PATIENCE: Duration = Duration::from_secs(60);
/// The bearer token the host presents on the host socket; browseros-service wants 32 bytes.
pub const HOST_TOKEN: &str = "e2e-host-token-0123456789-abcdefghij";
/// In the rig's root: the token's file, the CAPTCHA placeholder key's file, and the browser's
/// user-data dir. All three are among Neo's secret files, which no upload hands a page.
const TOKEN_FILE: &str = "host-token";
const CAPTCHA_KEY_FILE: &str = "capsolver-key";
const PROFILE_DIR: &str = "profile";

/// The BrowserOS binary `CLAW_SDK_E2E_BROWSER` names, or `None` after saying the test skipped.
#[must_use]
pub fn browser_or_skip() -> Option<PathBuf> {
    let browser = env::var_os(BROWSER_ENV).map(PathBuf::from);
    if browser.is_none() {
        skipped();
    }
    browser
}

/// Whether Neo starts with a CAPTCHA provider key.
#[derive(Clone, Copy, Debug)]
pub enum CaptchaKey {
    /// No key: `neo.solveCaptcha` is off.
    Absent,
    /// A placeholder key file and a provider URL nothing listens on.
    Placeholder,
}

/// One response from the host socket, its body as bytes, so a screenshot survives the read.
#[derive(Debug)]
pub struct HostResponse {
    /// The status code.
    pub status: u16,
    /// The status line and headers, as sent.
    pub head: String,
    /// The body, untouched.
    pub body: Vec<u8>,
}

impl HostResponse {
    /// The body as JSON.
    pub fn json(&self) -> Result<Value, Box<dyn Error>> {
        Ok(serde_json::from_slice(&self.body)?)
    }

    /// A header's value, by case-insensitive name.
    #[must_use]
    pub fn header(&self, name: &str) -> Option<&str> {
        self.head
            .lines()
            .filter_map(|line| line.split_once(':'))
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.trim())
    }

    /// The whole response as text, for a failure message.
    #[must_use]
    pub fn text(&self) -> String {
        format!(
            "{}\r\n\r\n{}",
            self.head,
            String::from_utf8_lossy(&self.body)
        )
    }
}

/// The two processes and their scratch directory. Fields drop in order: both children are
/// killed (`kill_on_drop`) before the directory they write into is removed, on every path out
/// of the test, a failed assertion included.
#[derive(Debug)]
pub struct Rig {
    neo: Child,
    _browser: Child,
    server_port: u16,
    cdp_port: u16,
    dir: TempDir,
}

impl Rig {
    /// BrowserOS on a fresh profile, then claw-sdk pointed at its CDP port.
    pub fn start(browser: &Path, captcha: CaptchaKey) -> Result<Self, Box<dyn Error>> {
        Self::start_with(browser, captcha, &[])
    }

    /// As [`Rig::start`], with `env` also set on claw-sdk, after the rig's own variables.
    pub fn start_with(
        browser: &Path,
        captcha: CaptchaKey,
        env: &[(&str, &str)],
    ) -> Result<Self, Box<dyn Error>> {
        let dir = tempfile::tempdir()?;
        let cdp_port = free_port()?;
        let server_port = free_port()?;

        // The flags hosted Neo's image uses minus the display: no bundled server, updater or
        // extensions, which would race claw-sdk for the browser.
        let profile = dir.path().join(PROFILE_DIR);
        let browser = Command::new(browser)
            .arg("--headless=new")
            .arg(format!("--user-data-dir={}", profile.display()))
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

        // `execution` is Chromium's `<user-data-dir>/.browseros`, as every real launch writes it:
        // Neo places the browser profile from it, and uploads no file from its disk without it.
        let launch = dir.path().join("server_config.json");
        let launch_json = json!({
            "ports": { "server": server_port, "cdp": cdp_port },
            "directories": { "resources": "r", "execution": profile.join(".browseros") },
        });
        fs::write(&launch, launch_json.to_string())?;
        let token_file = dir.path().join(TOKEN_FILE);
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
            let key_file = dir.path().join(CAPTCHA_KEY_FILE);
            fs::write(&key_file, "placeholder")?;
            neo.env("BROWSEROS_CAPSOLVER_API_KEY_FILE", &key_file)
                .env("BROWSEROS_CAPSOLVER_API_URL", "http://127.0.0.1:9");
        }
        neo.envs(env.iter().copied());
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

    /// The browser's remote-debugging port, for `/json/list` and `/json/close`.
    #[must_use]
    pub fn cdp_port(&self) -> u16 {
        self.cdp_port
    }

    /// One GET on the host socket, as zuzu sends it, with the bearer token when given.
    pub async fn host_get(
        &self,
        path: &str,
        token: Option<&str>,
    ) -> Result<String, Box<dyn Error>> {
        self.host_call("GET", path, token, None).await
    }

    /// One request on the host socket, as zuzu sends it, with the bearer token when given.
    pub async fn host_call(
        &self,
        method: &str,
        path: &str,
        token: Option<&str>,
        body: Option<&Value>,
    ) -> Result<String, Box<dyn Error>> {
        Ok(self.host(method, path, token, body).await?.text())
    }

    /// One request on the host socket, its answer parsed into status, head and raw body.
    pub async fn host(
        &self,
        method: &str,
        path: &str,
        token: Option<&str>,
        body: Option<&Value>,
    ) -> Result<HostResponse, Box<dyn Error>> {
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
        let mut response = Vec::new();
        time::timeout(PATIENCE, stream.read_to_end(&mut response)).await??;
        let split = response
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
            .ok_or_else(|| format!("not an HTTP response: {response:?}"))?;
        let head = String::from_utf8(response.get(..split).unwrap_or_default().to_vec())?;
        let status = head
            .split(' ')
            .nth(1)
            .and_then(|code| code.parse().ok())
            .ok_or_else(|| format!("no status line: {head}"))?;
        let body = response.get(split + 4..).unwrap_or_default().to_vec();
        Ok(HostResponse { status, head, body })
    }

    /// The host token's file, which Neo reads at start (`CLAW_HOST_TOKEN_FILE`).
    #[must_use]
    pub fn token_path(&self) -> PathBuf {
        self.dir.path().join(TOKEN_FILE)
    }

    /// The CAPTCHA placeholder key's file, written when the rig starts with
    /// [`CaptchaKey::Placeholder`].
    #[must_use]
    pub fn captcha_key_path(&self) -> PathBuf {
        self.dir.path().join(CAPTCHA_KEY_FILE)
    }

    /// The browser's user-data dir, where Chromium keeps the profile's cookies and `Local State`.
    #[must_use]
    pub fn profile_dir(&self) -> PathBuf {
        self.dir.path().join(PROFILE_DIR)
    }

    /// Where claw-sdk keeps its trace, readable once it stopped.
    #[must_use]
    pub fn trace_path(&self) -> PathBuf {
        self.dir
            .path()
            .join("state")
            .join("claw-sdk")
            .join("trace.db")
    }

    /// `/system/ready` answers 200 once the browser link is up.
    pub async fn until_ready(&self) -> TestResult {
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
    pub async fn client(&self) -> Result<Client, Box<dyn Error>> {
        let mut info = ClientInfo::default();
        info.protocol_version = ProtocolVersion::V_2025_11_25;
        let url = format!("http://127.0.0.1:{}/mcp", self.server_port);
        let transport = StreamableHttpClientTransport::from_config(
            StreamableHttpClientTransportConfig::with_uri(url),
        );
        Ok(info.serve(transport).await?)
    }

    /// SIGTERM, as Chromium stops it, and the exit code; the trace writer commits on this path.
    pub async fn stop_neo(&mut self) -> Result<Option<i32>, Box<dyn Error>> {
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
    #[must_use]
    pub fn logs(&self) -> String {
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

/// `run` with `_meta["com.browseros.neo/run"] = meta`.
pub async fn run(
    client: &Client,
    code: &str,
    meta: &Value,
) -> Result<CallToolResult, Box<dyn Error>> {
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
pub fn outcome(result: &CallToolResult) -> Result<&Value, Box<dyn Error>> {
    result
        .meta
        .as_ref()
        .and_then(|meta| meta.get(RUN_META))
        .ok_or_else(|| "every result carries the run's _meta".into())
}

/// One plain HTTP/1.1 GET on 127.0.0.1: the head and a `Content-Length` body. Chromium's
/// remote-debugging HTTP server ignores `Connection: close` and keeps the socket open, so
/// reading to EOF would never end.
pub async fn get(port: u16, path: &str) -> Result<String, Box<dyn Error>> {
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
