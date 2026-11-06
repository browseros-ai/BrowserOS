//! End to end on a real BrowserOS neo: `watch-url.js` polls `page.url()` and `page.isClosed()`
//! while this test changes the tab's URL over CDP and then closes it. The expected value is
//! what real Playwright returns for the same script (crates/neo-sdk/tests/parity checks that).
//!
//! Runs only when `NEO_E2E_BROWSER` names the browser binary, for example
//! `/Applications/BrowserOS neo.app/Contents/MacOS/BrowserOS neo`; otherwise it passes as a skip.

use std::ffi::{OsStr, OsString};
use std::io::Write;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};
use std::{env, fs, io, thread};

use browseros_cdp::{CdpClient, ConnectOptions, ReconnectPolicy, SessionId};
use neo_runtime::{PageId, RunRequest, RunStatus, Runtime};
use serde_json::{Value, json};
use tempfile::TempDir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::time;
use tokio_util::sync::CancellationToken;

const WATCH_URL: &str = include_str!("../../neo-sdk/tests/scripts/watch-url.js");
const STEP: Duration = Duration::from_millis(300);
const SLOW_RESPONSE: Duration = Duration::from_millis(2500);

#[tokio::test(flavor = "multi_thread")]
async fn page_url_and_is_closed_match_playwright_through_a_tab_lifetime() {
    let Some(binary) = env::var_os("NEO_E2E_BROWSER") else {
        writeln!(
            io::stderr(),
            "skipped: set NEO_E2E_BROWSER to a BrowserOS neo binary"
        )
        .unwrap();
        return;
    };
    let origin = serve_fixture().await;
    let browser = Browser::launch(binary);
    let cdp = CdpClient::connect(ConnectOptions {
        host: Some("127.0.0.1".to_string()),
        // The default exits the test process once the browser is gone at teardown.
        reconnect_policy: ReconnectPolicy::KeepTrying,
        ..ConnectOptions::new(browser.port)
    })
    .await
    .unwrap();

    let created = cdp
        .send(
            "Target.createTarget",
            json!({ "url": format!("{origin}/a") }),
            None,
        )
        .await
        .unwrap();
    let target_id = created["targetId"].as_str().unwrap().to_string();
    let attached = cdp
        .send(
            "Target.attachToTarget",
            json!({ "targetId": target_id, "flatten": true }),
            None,
        )
        .await
        .unwrap();
    let driver = SessionId::from(attached["sessionId"].as_str().unwrap());
    wait_for_url(&cdp, &driver, &format!("{origin}/a")).await;

    let frame_tree = cdp
        .send("Page.getFrameTree", json!({}), Some(&driver))
        .await
        .unwrap();
    assert_eq!(
        frame_tree["frameTree"]["frame"]["id"].as_str(),
        Some(target_id.as_str()),
        "`frame@<targetId>` relies on the main frame sharing its page's target id"
    );

    let runtime = Runtime::connect(cdp.clone()).await.unwrap();
    let run = tokio::spawn({
        let page = PageId::new(target_id.clone());
        async move {
            runtime
                .run(RunRequest {
                    code: WATCH_URL.to_string(),
                    page,
                    timeout: Duration::from_secs(30),
                    cancel: CancellationToken::new(),
                })
                .await
        }
    });

    time::sleep(STEP).await;
    evaluate(&cdp, &driver, "location.hash = 'done'").await;
    time::sleep(STEP).await;
    evaluate(&cdp, &driver, "history.pushState({}, '', '/pushed')").await;
    time::sleep(STEP).await;
    cdp.send(
        "Page.navigate",
        json!({ "url": format!("{origin}/slow") }),
        Some(&driver),
    )
    .await
    .unwrap();
    wait_for_url(&cdp, &driver, &format!("{origin}/slow")).await;
    time::sleep(STEP).await;
    cdp.send("Target.closeTarget", json!({ "targetId": target_id }), None)
        .await
        .unwrap();

    let outcome = run.await.unwrap();
    cdp.disconnect().await;
    drop(browser);

    assert_eq!(outcome.status, RunStatus::Ok, "{outcome:?}");
    assert_eq!(
        outcome.value,
        Some(json!({
            "paths": ["/a", "/a#done", "/pushed", "/slow"],
            "closed": true,
            "lastPath": "/slow",
        })),
        "Playwright keeps `/pushed` while `/slow` is in flight and the last URL after close"
    );
    assert_eq!(
        outcome.logs,
        ["saw 4 URLs before the tab closed"],
        "{outcome:?}"
    );
}

async fn evaluate(cdp: &CdpClient, session: &SessionId, expression: &str) {
    let reply = cdp
        .send(
            "Runtime.evaluate",
            json!({ "expression": expression }),
            Some(session),
        )
        .await
        .unwrap_or_else(|error| panic!("{expression}: {error}"));
    assert!(
        reply.get("exceptionDetails").is_none(),
        "{expression} threw: {reply}"
    );
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

/// Serves an HTML page for any path; `/slow` answers after `SLOW_RESPONSE`, so a navigation
/// to it stays in flight long enough to sample. Returns the origin.
async fn serve_fixture() -> String {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .unwrap_or_else(|error| panic!("bind the fixture server: {error}"));
    let address = listener
        .local_addr()
        .unwrap_or_else(|error| panic!("read the fixture server's port: {error}"));
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
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
                if path == "/slow" {
                    time::sleep(SLOW_RESPONSE).await;
                }
                let body = format!("<title>{path}</title><h1>{path}</h1>");
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
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
