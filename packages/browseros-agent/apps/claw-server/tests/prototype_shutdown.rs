//! Drives prototype::serve with its real system routes and rmcp router against fake CDP.
//! T7 covers the controlled host-signal future and /system/shutdown, with a pending run,
//! legacy GET and modern listen. It verifies ordering/outcomes, not W2 in isolation.
//! These POSTs are modern: their server order is T3c's
//! `transport_cannot_end_after_runs_drain_while_a_production_body_is_unread` together with
//! the source fact that GET ends only through transport; T3c-L covers the legacy POST.
//! Client receipt is required for both outcomes.

#![expect(
    clippy::unwrap_used,
    reason = "fixture setup and protocol failures fail the test"
)]

#[path = "../../../crates/neo-sdk/neo-runtime/tests/support/fake_browser.rs"]
#[expect(
    dead_code,
    reason = "the shared browser fixture has broader runtime test coverage"
)]
#[allow(
    unfulfilled_lint_expectations,
    reason = "the shared fixture has runtime-specific CDP lint exceptions absent from claw-server"
)]
mod fake_browser;
#[path = "../../../crates/neo-sdk/neo-sdk-server/tests/support/wire.rs"]
#[expect(
    dead_code,
    reason = "the legacy run variant is exercised by the server protocol suite"
)]
mod wire;

use std::fs;
use std::sync::Arc;
use std::time::Duration;

use claw_server::config::Config;
use claw_server::prototype;
use fake_browser::{FakeBrowser, Replies};
use reqwest::Client;
use serde_json::{Value, json};
use tempfile::TempDir;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use tokio::time;

/// A single owned prototype task; its port and files are private to this test.
struct Host {
    task: Option<JoinHandle<anyhow::Result<()>>>,
    signal: Option<oneshot::Sender<()>>,
    url: String,
    _state: TempDir,
}

impl Host {
    async fn start(port: u16) -> Self {
        let state = TempDir::new().unwrap();
        let dir = state.path().join("state");
        let config = Arc::new(Config {
            server_port: 0,
            cdp_port: port,
            proxy_port: None,
            resources_dir: state.path().join("resources"),
            browserclaw_dir: dir.clone(),
            session_idle: Duration::from_secs(300),
            session_retention: Duration::from_secs(7_200),
            session_sweep_interval: Duration::from_secs(60),
            replay_retention_days: 7,
            dev_mode: true,
            native: Default::default(),
            mode: Default::default(),
            behavior: Default::default(),
            behavior_ignored: Vec::new(),
        });
        let (signal, received) = oneshot::channel();
        let task = tokio::spawn(prototype::serve(config, false, async move {
            received.await.unwrap();
        }));
        let mut url = None;
        wire::until(
            || {
                assert!(
                    !task.is_finished(),
                    "prototype exited before publishing runtime.json"
                );
                url = fs::read_to_string(dir.join("runtime.json"))
                    .ok()
                    .and_then(|text| serde_json::from_str::<Value>(&text).ok())
                    .and_then(|record| record["url"].as_str().map(str::to_string));
                url.is_some()
            },
            "prototype did not publish its ephemeral port",
        )
        .await;
        Self {
            task: Some(task),
            signal: Some(signal),
            url: url.unwrap(),
            _state: state,
        }
    }

    async fn exited(&mut self) {
        time::timeout(Duration::from_secs(20), self.task.take().unwrap())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        if let Some(signal) = self.signal.take() {
            let _ = signal.send(());
        }
        if let Some(task) = &self.task {
            // Failure cleanup selects only this test's task. FakeBrowser owns its socket
            // tasks; dropping the test runtime also releases any remaining child work.
            task.abort();
        }
    }
}

enum Trigger {
    Signal,
    SystemRoute,
}

async fn shutdown_case(trigger: Trigger) {
    let browser = FakeBrowser::start(Replies::default()).await;
    let mut host = Host::start(browser.port()).await;
    let url = format!("{}/mcp", host.url);
    let client = Client::new();
    let session = wire::legacy_session(&client, &url).await;
    let get = wire::legacy_get(&client, &url, &session).await;
    let listen = wire::Listen::open(&client, &url, 901).await;
    let request = wire::modern(
        &client,
        &url,
        902,
        "tools/call",
        json!({"name":"run","arguments":{
            "page":fake_browser::PAGE,
            "code":"await page.title(); page.url(); await new Promise(r => setTimeout(r, 60000))",
            "timeout":120000
        }}),
    );
    let pending =
        tokio::spawn(async move { wire::result(request.send().await.unwrap(), 902).await });
    wire::until(
        || {
            browser
                .exchanges()
                .iter()
                .filter(|e| e.method() == "Page.getNavigationHistory")
                .count()
                >= 2
        },
        "prototype run never reached the live getter",
    )
    .await;
    match trigger {
        Trigger::Signal => host.signal.take().unwrap().send(()).unwrap(),
        Trigger::SystemRoute => {
            let response = client
                .post(format!("{}/system/shutdown", host.url))
                .send()
                .await
                .unwrap();
            assert!(response.status().is_success());
            let _reply: Value = response.json().await.unwrap();
        }
    }
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn prototype_signal_delivers_the_cancelled_reply_and_closes_streams() {
    shutdown_case(Trigger::Signal).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn prototype_system_shutdown_delivers_the_cancelled_reply_and_closes_streams() {
    shutdown_case(Trigger::SystemRoute).await;
}
