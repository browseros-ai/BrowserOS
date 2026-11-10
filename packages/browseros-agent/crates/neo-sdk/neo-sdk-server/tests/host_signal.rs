//! Neo: T6 drives the standalone executable and real loopback MCP against fake CDP.
//! SIGTERM must deliver the active reply and close legacy GET/listen streams before exit.
//! This proves host ordering/outcomes, not deterministic W2 discrimination (T3/T3c do that).
//! The POST is modern: its server order is T3c's
//! `transport_cannot_end_after_runs_drain_while_a_production_body_is_unread` together with
//! the source fact that GET ends only through transport; T3c-L covers the legacy POST.
//! The client must receive both complete outcomes.

#![cfg(unix)]
#![expect(
    clippy::unwrap_used,
    reason = "test process setup and malformed replies fail the fixture"
)]

#[path = "../../neo-runtime/tests/support/fake_browser.rs"]
#[expect(
    dead_code,
    reason = "shared CDP fixture supplies more methods than this host test uses"
)]
mod fake_browser;
#[path = "support/wire.rs"]
#[expect(
    dead_code,
    reason = "legacy run helper is covered by the separate protocol suite"
)]
mod wire;

use std::fs;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use fake_browser::{FakeBrowser, Replies};
use reqwest::Client;
use serde_json::json;
use tempfile::TempDir;
use tokio::time::{self, Instant};

/// Owns only the process group started by this test. Failure cleanup never selects by name.
struct Host {
    child: Child,
    log: PathBuf,
}

impl Host {
    fn start(root: &Path, port: u16) -> Self {
        let log = root.join("standalone.log");
        let child = Command::new(env!("CARGO_BIN_EXE_neo-sdk-server"))
            .env_clear()
            .env("RUST_LOG", "info,neo_trace=off")
            .args([
                "--listen",
                "127.0.0.1:0",
                "--cdp-port",
                &port.to_string(),
                "--state-dir",
            ])
            .arg(root.join("state"))
            .process_group(0)
            .stdout(Stdio::null())
            .stderr(fs::File::create(&log).unwrap())
            .spawn()
            .unwrap();
        Self { child, log }
    }

    async fn url(&mut self) -> String {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            assert!(
                self.child.try_wait().unwrap().is_none(),
                "host exited: {}",
                self.logs()
            );
            let log = self.logs();
            if let Some(address) = log.split("127.0.0.1:").nth(1) {
                let port: String = address.chars().take_while(char::is_ascii_digit).collect();
                if !port.is_empty() {
                    return format!("http://127.0.0.1:{port}/mcp");
                }
            }
            assert!(
                Instant::now() < deadline,
                "host never reported its bound address: {log}"
            );
            time::sleep(Duration::from_millis(5)).await;
        }
    }

    fn logs(&self) -> String {
        fs::read_to_string(&self.log).unwrap_or_default()
    }

    fn terminate(&self) {
        assert!(
            Command::new("kill")
                .args(["-TERM", &self.child.id().to_string()])
                .status()
                .unwrap()
                .success(),
            "SIGTERM must be delivered to the owned standalone host"
        );
    }

    #[expect(
        clippy::print_stderr,
        reason = "retain owned process cleanup evidence in the test log"
    )]
    async fn exited(&mut self) {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                assert!(status.success(), "{status}: {}", self.logs());
                eprintln!(
                    "T6 owned standalone pid={} exit=0; signal, run reply, GET and listen completed",
                    self.child.id()
                );
                return;
            }
            assert!(
                Instant::now() < deadline,
                "standalone host did not exit: {}",
                self.logs()
            );
            time::sleep(Duration::from_millis(5)).await;
        }
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = Command::new("kill")
                .args(["-KILL", "--", &format!("-{}", self.child.id())])
                .status();
            let _ = self.child.wait();
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sigterm_delivers_the_run_reply_and_ends_open_legacy_and_listen_streams() {
    let browser = FakeBrowser::start(Replies::default()).await;
    let state = TempDir::new().unwrap();
    let mut host = Host::start(state.path(), browser.port());
    let url = host.url().await;
    let client = Client::new();
    let session = wire::legacy_session(&client, &url).await;
    let get = wire::legacy_get(&client, &url, &session).await;
    let listen = wire::Listen::open(&client, &url, 801).await;
    let request = wire::modern(
        &client,
        &url,
        802,
        "tools/call",
        json!({"name":"run","arguments":{
            "page":fake_browser::PAGE,
            "code":"await page.title(); page.url(); await new Promise(r => setTimeout(r, 60000))",
            "timeout":120_000
        }}),
    );
    let pending =
        tokio::spawn(async move { wire::result(request.send().await.unwrap(), 802).await });
    wire::until(
        || {
            browser
                .exchanges()
                .iter()
                .filter(|e| e.method() == "Page.getNavigationHistory")
                .count()
                >= 2
        },
        "the executable run never reached its live getter",
    )
    .await;
    host.terminate();
    let (result, _observed_order) = wire::reply_and_get_eof(pending, get).await;
    assert_eq!(
        result["structuredContent"]["status"], "cancelled",
        "{result}"
    );
    assert!(
        !result["structuredContent"]["calls"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    listen.complete().await;
    host.exited().await;
}
