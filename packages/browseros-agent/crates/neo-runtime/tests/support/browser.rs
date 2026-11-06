//! A real BrowserOS neo for end-to-end tests: a headless browser on a throwaway profile, one
//! tab the test drives over its own CDP session, a local fixture server, and Neo runs against
//! that tab. Gated on `NEO_E2E_BROWSER`, the browser binary's path; without it a test passes as
//! a skip.
//!
//! The browser always launches `--headless=new --disable-browseros-server`: without the second
//! flag, a browseros-claw-server sidecar outlives it. Dropping a [`Browser`] ends it and every
//! helper process it started.
//!
//! Test crates include this file with `#[path]`. Cargo builds only top-level `tests/*.rs` files
//! as test crates, so it is never one on its own.

use std::ffi::{OsStr, OsString};
use std::io::Write;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};
use std::{env, fs, io, thread};

use browseros_cdp::{CdpClient, ConnectOptions, ReconnectPolicy, SessionId};
use neo_runtime::{PageId, RunOutcome, RunRequest, Runtime};
use serde_json::{Value, json};
use tempfile::TempDir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;
use tokio::time;
use tokio_util::sync::CancellationToken;

/// The browser binary, or `None` (and a note on stderr) when the test should skip.
pub fn e2e_browser() -> Option<OsString> {
    let binary = env::var_os("NEO_E2E_BROWSER");
    if binary.is_none() {
        let _ = writeln!(
            io::stderr(),
            "skipped: set NEO_E2E_BROWSER to a BrowserOS neo binary"
        );
    }
    binary
}

/// What the fixture server serves. A path in `pages` gets its HTML; any other path gets a small
/// page titled with the path. A path starting with `/slow` answers after `slow_response`, so a
/// navigation to it stays in flight before it commits.
#[derive(Clone, Default)]
pub struct Fixture {
    pub slow_response: Duration,
    pub pages: Vec<(String, String)>,
}

/// A tab of a fresh headless BrowserOS neo. The test drives it over its own `driver` session,
/// separate from the one a run attaches, as an outside actor would.
pub struct Tab {
    browser: Browser,
    pub cdp: CdpClient,
    pub origin: String,
    pub target_id: String,
    pub driver: SessionId,
}

impl Tab {
    /// Opens `<origin><path>` and returns once it has committed.
    pub async fn open(binary: OsString, fixture: Fixture, path: &str) -> Self {
        let origin = serve_fixture(fixture).await;
        let browser = Browser::launch(binary);
        let cdp = CdpClient::connect(ConnectOptions {
            host: Some("127.0.0.1".to_string()),
            // The default exits the test process once the browser is gone at teardown.
            reconnect_policy: ReconnectPolicy::KeepTrying,
            ..ConnectOptions::new(browser.port)
        })
        .await
        .unwrap_or_else(|error| panic!("connect to the browser: {error}"));
        let url = format!("{origin}{path}");
        let created = cdp
            .send("Target.createTarget", json!({ "url": url }), None)
            .await
            .unwrap_or_else(|error| panic!("open a tab: {error}"));
        let target_id = created["targetId"]
            .as_str()
            .unwrap_or_else(|| panic!("Target.createTarget answered {created}"))
            .to_string();
        let attached = cdp
            .send(
                "Target.attachToTarget",
                json!({ "targetId": target_id, "flatten": true }),
                None,
            )
            .await
            .unwrap_or_else(|error| panic!("attach the driver session: {error}"));
        let driver = SessionId::from(
            attached["sessionId"]
                .as_str()
                .unwrap_or_else(|| panic!("Target.attachToTarget answered {attached}")),
        );
        wait_for_url(&cdp, &driver, &url).await;
        Self {
            browser,
            cdp,
            origin,
            target_id,
            driver,
        }
    }

    /// Starts `code` as a Neo run against this tab; it reads the tab while the test drives it.
    pub async fn start_run(&self, code: String) -> JoinHandle<RunOutcome> {
        let runtime = Runtime::connect(self.cdp.clone())
            .await
            .unwrap_or_else(|error| panic!("Runtime::connect: {error}"));
        let page = PageId::new(self.target_id.clone());
        tokio::spawn(async move {
            runtime
                .run(RunRequest {
                    code,
                    page,
                    timeout: Duration::from_secs(30),
                    cancel: CancellationToken::new(),
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

    /// Navigates to `path` on the fixture's origin and returns once it has committed.
    pub async fn navigate(&self, path: &str) {
        let url = format!("{}{path}", self.origin);
        self.cdp
            .send("Page.navigate", json!({ "url": url }), Some(&self.driver))
            .await
            .unwrap_or_else(|error| panic!("navigate to {url}: {error}"));
        wait_for_url(&self.cdp, &self.driver, &url).await;
    }

    pub async fn close(self) {
        let Self { browser, cdp, .. } = self;
        cdp.disconnect().await;
        drop(browser);
    }
}

/// Waits until the tab's committed URL is `url`. A read can fail for a few milliseconds while
/// a cross-document navigation commits ("Not attached to an active page"), so failures retry.
async fn wait_for_url(cdp: &CdpClient, session: &SessionId, url: &str) {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let current = match cdp
            .send("Page.getNavigationHistory", json!({}), Some(session))
            .await
        {
            Ok(history) => history["currentIndex"]
                .as_u64()
                .and_then(|index| usize::try_from(index).ok())
                .map_or(Value::Null, |index| {
                    history["entries"][index]["url"].clone()
                }),
            Err(error) => json!(error.to_string()),
        };
        if current == url {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "the tab never committed {url}; it is at {current}"
        );
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
    let fixture = Arc::new(fixture);
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
                let body = fixture
                    .pages
                    .iter()
                    .find(|(page, _)| *page == path)
                    .map_or_else(
                        || format!("<title>{path}</title><h1>{path}</h1>"),
                        |(_, html)| html.clone(),
                    );
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                // The browser may cancel a request it no longer needs.
                let _ = stream.write_all(response.as_bytes()).await;
            });
        }
    });
    format!("http://{address}")
}

/// A headless BrowserOS neo on a throwaway profile. Dropping it ends the browser and every
/// helper process it started.
struct Browser {
    child: Child,
    port: u16,
    profile: TempDir,
}

impl Browser {
    fn launch(binary: OsString) -> Self {
        let profile = tempfile::tempdir()
            .unwrap_or_else(|error| panic!("create a profile directory: {error}"));
        let child = Command::new(binary)
            .arg("--headless=new")
            // Otherwise the browser starts a browseros-claw-server sidecar that outlives it.
            .arg("--disable-browseros-server")
            // Port 0 lets the browser pick a free port and write it to DevToolsActivePort.
            .arg("--remote-debugging-port=0")
            .arg(format!("--user-data-dir={}", profile.path().display()))
            .arg("--no-first-run")
            .arg("about:blank")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap_or_else(|error| panic!("launch the browser: {error}"));
        let port_file = profile.path().join("DevToolsActivePort");
        let deadline = Instant::now() + Duration::from_secs(20);
        let port = loop {
            let port = fs::read_to_string(&port_file)
                .ok()
                .and_then(|text| text.lines().next()?.parse::<u16>().ok());
            if let Some(port) = port {
                break port;
            }
            assert!(
                Instant::now() < deadline,
                "the browser never opened its CDP port"
            );
            thread::sleep(Duration::from_millis(50));
        };
        Self {
            child,
            port,
            profile,
        }
    }
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

impl Drop for Browser {
    fn drop(&mut self) {
        let browser_pid = self.child.id().to_string();
        let mut lingering = pgrep(&["-P".as_ref(), browser_pid.as_ref()]);
        // SIGTERM first, so the browser shuts its helpers down itself.
        let _ = Command::new("kill").arg(&browser_pid).status();
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline && matches!(self.child.try_wait(), Ok(None)) {
            thread::sleep(Duration::from_millis(50));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
        // Helpers notice their parent is gone on their own, usually within a second.
        lingering.extend(pgrep(&["-f".as_ref(), self.profile.path().as_os_str()]));
        let deadline = Instant::now() + Duration::from_secs(5);
        while !lingering.is_empty() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(100));
            lingering.retain(|pid| {
                Command::new("kill")
                    .args(["-0", pid])
                    .stderr(Stdio::null())
                    .status()
                    .is_ok_and(|status| status.success())
            });
        }
        for pid in &lingering {
            let _ = Command::new("kill").args(["-9", pid]).status();
        }
    }
}
