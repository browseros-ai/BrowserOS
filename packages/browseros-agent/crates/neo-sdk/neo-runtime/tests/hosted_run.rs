//! Neo: The host door preserves synchronous reads, event shutdown, and native run cleanup.
#![cfg(feature = "hosted-run")]

#[expect(
    dead_code,
    reason = "shared fixture exposes more browser controls than these tests need"
)]
#[path = "support/fake_browser.rs"]
mod fake_browser;

use std::sync::Arc;
use std::thread;
use std::time::Duration;

use fake_browser::{FakeBrowser, Replies};
use neo_runtime::RunStatus;
use serde_json::{Value, json};
use tokio::time;

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_an_external_thread_reads_live_values_and_finish_settles_events() {
    let browser = FakeBrowser::start(Replies::default()).await;
    let runtime = browser.runtime().await;
    let run = Arc::new(
        runtime
            .open_hosted_run(Duration::from_secs(10), None)
            .await
            .unwrap(),
    );
    let reader = Arc::clone(&run);
    let value = thread::spawn(move || {
        let request = json!({"guid": reader.init()["page"], "field": "url"});
        reader.get(&request.to_string()).unwrap()
    })
    .join()
    .unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&value).unwrap(),
        "about:blank"
    );
    assert_eq!(run.get("{}").unwrap_err().name, "Error");
    assert_eq!(run.drain(-1), "[]");
    run.log("host log".to_string());
    let pending = Arc::clone(&run);
    let events = tokio::spawn(async move { pending.next_events().await });
    let outcome = run.finish().await.unwrap();
    assert_eq!(outcome.status, RunStatus::Ok, "{outcome:?}");
    assert_eq!(outcome.logs, ["host log"]);
    assert!(outcome.pages.iter().any(|page| page.created));
    assert!(
        time::timeout(Duration::from_secs(1), events)
            .await
            .unwrap()
            .unwrap()
            .is_err()
    );
    assert_eq!(run.finish().await.unwrap(), outcome);
    assert!(run.get(r#"{"field":"devices"}"#).is_err());
    runtime.drain().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_a_host_deadline_finishes_without_host_cooperation() {
    let browser = FakeBrowser::start(Replies::default()).await;
    let runtime = browser.runtime().await;
    let run = runtime
        .open_hosted_run(Duration::from_millis(200), None)
        .await
        .unwrap();
    assert!(
        time::timeout(Duration::from_secs(2), run.next_events())
            .await
            .unwrap()
            .is_err()
    );
    assert_eq!(run.finish().await.unwrap().status, RunStatus::Timeout);
    runtime.drain().await;
}
