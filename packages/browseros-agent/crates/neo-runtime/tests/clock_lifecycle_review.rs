//! The run-lifetime Clock deviation must also restore timing APIs on cancel and timeout.
//! Native page behavior is the control; the fake clock is confirmed before ending the run.

#[path = "support/browser.rs"]
#[expect(dead_code, reason = "the shared fixture serves other browser tests")]
mod browser;

use std::time::Duration;

use browser::{Fixture, Tab};
use neo_runtime::{PageId, RunRequest, RunStatus, Runtime};
use serde_json::{Value, json};
use tokio::time;
use tokio_util::sync::CancellationToken;

const TIMING_APIS: &str = r"(async () => {
  const before = new Event('probe').timeStamp;
  const signal = AbortSignal.timeout(10);
  const aborts = await new Promise(resolve => {
    if (signal.aborted) resolve(true);
    else signal.addEventListener('abort', () => resolve(true), {once: true});
    setTimeout(() => resolve(false), 1000);
  });
  await new Promise(resolve => setTimeout(resolve, 25));
  return {realDate: Date.now() > 1600000000000,
    clock: typeof globalThis.__pwClock,
    aborts, eventTimeAdvances: new Event('probe').timeStamp > before};
})()";

async fn timing_apis(tab: &Tab) -> Value {
    let reply = time::timeout(
        Duration::from_secs(5),
        tab.cdp.send(
            "Runtime.evaluate",
            json!({"expression": TIMING_APIS, "returnByValue": true, "awaitPromise": true}),
            Some(&tab.driver),
        ),
    )
    .await
    .unwrap_or_else(|error| panic!("the native fallback timer settles the probe: {error}"))
    .unwrap_or_else(|error| panic!("CDP probe: {error}"));
    assert!(reply.get("exceptionDetails").is_none(), "{reply}");
    reply["result"]["value"].clone()
}

async fn check_cleanup(cancel_run: bool) {
    let Some(binary) = browser::e2e_browser() else {
        return;
    };
    let fixture = Fixture {
        pages: vec![("/clock".into(), "<title>clock lifecycle</title>".into())],
        ..Fixture::default()
    };
    let tab = Tab::open(binary, fixture, "/clock").await;
    let before = timing_apis(&tab).await;
    let runtime = Runtime::connect(tab.cdp.clone())
        .await
        .unwrap_or_else(|error| panic!("runtime: {error}"));
    let cancel = CancellationToken::new();
    let request = RunRequest {
        code: "await page.clock.install({time: 0});
               await page.clock.pauseAt(1000000);
               await page.evaluate(() => {window.clockReady = true});
               await new Promise(() => {})"
            .into(),
        page: PageId::new(tab.target_id.clone()),
        timeout: Duration::from_secs(if cancel_run { 30 } else { 5 }),
        cancel: cancel.clone(),
        artifacts: None,
    };
    let run = tokio::spawn({
        let runtime = runtime.clone();
        async move { runtime.run(request).await }
    });
    time::timeout(Duration::from_secs(15), async {
        while tab.evaluate("window.clockReady === true").await != json!(true) {
            time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap_or_else(|error| panic!("the run installed and paused the clock: {error}"));
    let during = tab.evaluate("Date.now()").await;
    if cancel_run {
        cancel.cancel();
    }
    let outcome = time::timeout(Duration::from_secs(15), run)
        .await
        .unwrap_or_else(|error| panic!("the run returns: {error}"))
        .unwrap_or_else(|error| panic!("run task: {error}"));
    time::timeout(Duration::from_secs(10), runtime.drain())
        .await
        .unwrap_or_else(|error| panic!("cleanup drains: {error}"));
    let after = timing_apis(&tab).await;
    tab.close().await;

    assert_eq!(
        before,
        json!({"realDate": true, "clock": "undefined", "aborts": true, "eventTimeAdvances": true}),
        "native control"
    );
    assert_eq!(during, json!(1_000_000), "the run's clock was active");
    assert_eq!(
        outcome.status,
        if cancel_run {
            RunStatus::Cancelled
        } else {
            RunStatus::Timeout
        },
        "{outcome:?}"
    );
    assert_eq!(
        after, before,
        "the ended run restores every timing API: cancel={cancel_run} outcome={outcome:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn cancellation_restores_the_pages_timing_apis() {
    check_cleanup(true).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn timeout_restores_the_pages_timing_apis() {
    check_cleanup(false).await;
}
