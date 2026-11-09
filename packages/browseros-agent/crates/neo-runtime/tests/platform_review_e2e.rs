//! Independent review probes for the platform slice's cancellation and dropped-run cleanup.

#[path = "support/browser.rs"]
#[expect(
    dead_code,
    reason = "the shared real-browser harness serves other suites too"
)]
mod browser;

use std::time::Duration;

use browser::{Fixture, Tab, e2e_browser};
use neo_runtime::{PageId, RunRequest, RunStatus, Runtime};
use serde_json::{Value, json};
use tokio::time::{self, Instant};
use tokio_util::sync::CancellationToken;

async fn contexts(tab: &Tab) -> Value {
    tab.cdp
        .send("Target.getBrowserContexts", json!({}), None)
        .await
        .unwrap_or_else(|error| panic!("reading created contexts: {error}"))["browserContextIds"]
        .clone()
}

async fn armed(tab: &Tab) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while tab.evaluate("location.pathname").await != json!("/armed") {
        assert!(
            Instant::now() < deadline,
            "the script did not reach its cleanup checkpoint"
        );
        time::sleep(Duration::from_millis(25)).await;
    }
    assert_eq!(
        contexts(tab).await.as_array().map(Vec::len),
        Some(1),
        "the script must create one context before cancellation"
    );
}

async fn run_cleanup_case(drop_future: bool) {
    let Some(binary) = e2e_browser() else { return };
    let tab = Tab::open(binary, Fixture::default(), "/start").await;
    let runtime = Runtime::connect(tab.cdp.clone())
        .await
        .unwrap_or_else(|error| panic!("connecting the run: {error}"));
    let cancel = CancellationToken::new();
    let request = RunRequest {
        code: "const created = await context.browser().newContext();
const other = await created.newPage();
await other.goto(page.url().replace('/start', '/created'));
await page.goto(page.url().replace('/start', '/armed'));
await new Promise(() => {});"
            .to_string(),
        page: PageId::new(tab.target_id.clone()),
        timeout: Duration::from_secs(30),
        cancel: cancel.clone(),
        artifacts: None,
        hooks: None,
    };
    let worker = runtime.clone();
    let running = tokio::spawn(async move { worker.run(request).await });
    armed(&tab).await;
    if drop_future {
        running.abort();
        let error = match running.await {
            Ok(outcome) => panic!("the dropped run finished instead: {outcome:?}"),
            Err(error) => error,
        };
        assert!(error.is_cancelled(), "dropping the run failed: {error}");
    } else {
        cancel.cancel();
        let outcome = running
            .await
            .unwrap_or_else(|error| panic!("the cancelled run failed: {error}"));
        assert_eq!(outcome.status, RunStatus::Cancelled, "{outcome:?}");
    }
    time::timeout(Duration::from_secs(10), runtime.drain())
        .await
        .unwrap_or_else(|error| panic!("draining the ended run: {error}"));
    let deadline = Instant::now() + Duration::from_secs(10);
    while contexts(&tab).await != json!([]) {
        assert!(
            Instant::now() < deadline,
            "a created context outlived its run"
        );
        time::sleep(Duration::from_millis(25)).await;
    }
    tab.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn cancellation_disposes_created_contexts() {
    run_cleanup_case(false).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn dropping_a_run_disposes_created_contexts() {
    run_cleanup_case(true).await;
}
