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

use fake_browser::{FakeBrowser, Replies, Reply};
use neo_runtime::{CallResult, RunEnd, RunStatus};
use serde_json::{Value, json};
use tokio::task;
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
    assert_eq!(run.drain(-1).unwrap(), "[]");
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
    let error = time::timeout(Duration::from_secs(2), run.next_events())
        .await
        .unwrap()
        .unwrap_err();
    assert_eq!(error.name, "TimeoutError");
    assert_eq!(error.message, "Timeout 200ms exceeded.");
    assert_eq!(run.get("{}").unwrap_err(), error);
    assert_eq!(run.subscribe("{}").unwrap_err(), error);
    assert_eq!(run.drain(-1).unwrap_err(), error);
    assert_eq!(run.call_with_events("{}").await.unwrap_err(), error);
    assert_eq!(run.finish().await.unwrap().status, RunStatus::Timeout);
    runtime.drain().await;
}

/// Neo: Abandoning an open must cancel startup before its long host deadline.
#[tokio::test(flavor = "multi_thread")]
async fn tests_that_dropping_an_open_closes_its_unkept_tab() {
    let browser = FakeBrowser::start(Replies {
        new_page_empty_history: usize::MAX,
        ..Replies::default()
    })
    .await;
    let runtime = browser.runtime().await;
    let opening_runtime = runtime.clone();
    let opening = tokio::spawn(async move {
        opening_runtime
            .open_hosted_run(Duration::from_mins(1), None)
            .await
    });
    time::timeout(Duration::from_secs(2), async {
        while !browser
            .exchanges()
            .iter()
            .any(|exchange| exchange.method() == "Target.attachToTarget")
        {
            time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    opening.abort();
    assert!(opening.await.is_err_and(|error| error.is_cancelled()));
    time::timeout(Duration::from_secs(3), runtime.drain())
        .await
        .unwrap();
    assert_eq!(browser.page.lock().unwrap().tabs.len(), 1);
}

/// Neo: A failed open reports its original native error and does not retain cleanup work.
#[tokio::test(flavor = "multi_thread")]
async fn tests_that_a_failed_open_can_be_drained() {
    let browser = FakeBrowser::start(Replies::default()).await;
    let runtime = browser.runtime().await;
    browser.page.lock().unwrap().tabs.clear();
    let error = runtime
        .open_hosted_run(Duration::from_secs(10), None)
        .await
        .err()
        .unwrap();
    assert_eq!(error.name, "Error");
    assert_eq!(
        error.message,
        "no browser window is open to start the run's tab in"
    );
    time::timeout(Duration::from_secs(2), runtime.drain())
        .await
        .unwrap();
}

/// Neo: Native getters keep the same target-closed name and wording as the QuickJS door.
#[tokio::test(flavor = "multi_thread")]
async fn tests_that_getters_preserve_target_closed_errors() {
    let browser = FakeBrowser::start(Replies::default()).await;
    let runtime = browser.runtime().await;
    let run = runtime
        .open_hosted_run(Duration::from_secs(10), None)
        .await
        .unwrap();
    run.finish().await.unwrap();
    let error = run
        .get(&json!({"guid": run.init()["page"], "field": "url"}).to_string())
        .unwrap_err();
    assert_eq!(error.name, "TargetClosedError");
    assert_eq!(
        error.message,
        "Target page, context or browser has been closed"
    );
    run.finish().await.unwrap();
    runtime.drain().await;
}

/// Neo: Finishing while a call is pending settles the reply and gives concurrent finishers one report.
#[tokio::test(flavor = "multi_thread")]
async fn tests_that_finish_settles_an_in_flight_call() {
    let browser = FakeBrowser::start(Replies {
        title: Reply {
            answers: 0,
            ..Reply::default()
        },
        ..Replies::default()
    })
    .await;
    let runtime = browser.runtime().await;
    let run = runtime
        .open_hosted_run(Duration::from_mins(1), None)
        .await
        .unwrap();
    let reply = run.call_with_events(&json!({"id": 1, "guid": run.init()["mainFrame"], "method": "title", "params": {}, "metadata": {}}).to_string());
    let (first, second) = tokio::join!(run.finish(), run.finish());
    let first = first.unwrap();
    assert_eq!(first, second.unwrap());
    assert_eq!(
        first.calls[0].result,
        CallResult::Unfinished {
            reason: RunEnd::Settled
        }
    );
    assert!(time::timeout(Duration::from_secs(2), reply).await.is_ok());
    runtime.drain().await;
}

/// Neo: Runtime shutdown owns admission and cancellation, including pending idle event futures.
#[tokio::test(flavor = "multi_thread")]
async fn tests_that_shutdown_cancels_open_runs_and_refuses_more() {
    let browser = FakeBrowser::start(Replies::default()).await;
    let runtime = browser.runtime().await;
    let first = runtime
        .open_hosted_run(Duration::from_mins(1), None)
        .await
        .unwrap();
    let second = runtime
        .open_hosted_run(Duration::from_mins(1), None)
        .await
        .unwrap();
    let pending = first.next_events();
    time::timeout(Duration::from_secs(3), runtime.shutdown_hosted_runs())
        .await
        .unwrap();
    assert_eq!(pending.await.unwrap_err().message, "Run cancelled");
    assert_eq!(first.finish().await.unwrap().status, RunStatus::Cancelled);
    assert_eq!(second.finish().await.unwrap().status, RunStatus::Cancelled);
    assert_eq!(
        runtime
            .open_hosted_run(Duration::from_mins(1), None)
            .await
            .err()
            .unwrap()
            .name,
        "TargetClosedError"
    );
    assert_eq!(browser.page.lock().unwrap().tabs.len(), 1);
    runtime.shutdown_hosted_runs().await;
}

/// Neo: An event future must not own a run whose host has dropped it.
#[tokio::test(flavor = "multi_thread")]
async fn tests_that_dropping_a_run_settles_pending_events_and_closes_its_tab() {
    let browser = FakeBrowser::start(Replies::default()).await;
    let runtime = browser.runtime().await;
    let run = runtime
        .open_hosted_run(Duration::from_mins(1), None)
        .await
        .unwrap();
    let pending = run.next_events();
    drop(run);
    assert_eq!(
        time::timeout(Duration::from_secs(2), pending)
            .await
            .unwrap()
            .unwrap_err()
            .message,
        "Run cancelled"
    );
    runtime.shutdown_hosted_runs().await;
    assert_eq!(browser.page.lock().unwrap().tabs.len(), 1);
}

/// Neo: Cancellation before the new tab's acknowledgement still closes the late tab.
#[tokio::test(flavor = "multi_thread")]
async fn tests_that_shutdown_waits_for_a_late_opened_tab_to_close() {
    let browser = FakeBrowser::start(Replies {
        create_tab_delay: Some(Duration::from_millis(100)),
        ..Replies::default()
    })
    .await;
    let runtime = browser.runtime().await;
    let copy = runtime.clone();
    let opening =
        tokio::spawn(async move { copy.open_hosted_run(Duration::from_mins(1), None).await });
    time::timeout(Duration::from_secs(2), async {
        while !browser
            .exchanges()
            .iter()
            .any(|exchange| exchange.method() == "Browser.createTab")
        {
            time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    time::timeout(Duration::from_secs(3), runtime.shutdown_hosted_runs())
        .await
        .unwrap();
    assert!(opening.await.unwrap().is_err());
    assert_eq!(browser.page.lock().unwrap().tabs.len(), 1);
}

/// Neo: A lost browser ends an idle host promptly, without waiting for the worker deadline.
#[tokio::test(flavor = "multi_thread")]
async fn tests_that_browser_loss_ends_idle_events_and_future_opens() {
    let browser = FakeBrowser::start(Replies::default()).await;
    let cdp = browser.cdp().await;
    let runtime = neo_runtime::Runtime::connect(cdp.clone()).await.unwrap();
    let run = runtime
        .open_hosted_run(Duration::from_mins(1), None)
        .await
        .unwrap();
    let events = run.next_events();
    cdp.disconnect().await;
    assert_eq!(
        time::timeout(Duration::from_secs(2), events)
            .await
            .unwrap()
            .unwrap_err()
            .name,
        "TargetClosedError"
    );
    assert_eq!(run.finish().await.unwrap().status, RunStatus::Error);
    assert!(
        runtime
            .open_hosted_run(Duration::from_secs(1), None)
            .await
            .is_err()
    );
    runtime.shutdown_hosted_runs().await;
}

/// Neo: A paused call wakes idle events and the resume reply includes both state changes.
#[tokio::test(flavor = "multi_thread")]
async fn tests_that_events_arrive_before_their_call_continues() {
    let browser = FakeBrowser::start(Replies::default()).await;
    let runtime = browser.runtime().await;
    let run = runtime
        .open_hosted_run(Duration::from_secs(10), None)
        .await
        .unwrap();
    run.subscribe(r#"{"guid":"debugger@default","event":"pausedstatechanged","enabled":true}"#)
        .unwrap();
    let (reply, _) = run.call_with_events(r#"{"id":1,"guid":"debugger@default","method":"requestPause","params":{},"metadata":{}}"#).await.unwrap();
    assert!(
        serde_json::from_str::<Value>(&reply)
            .unwrap()
            .get("error")
            .is_none(),
        "{reply}"
    );
    let pending = run.next_events();
    let paused = run.call_with_events(
        r#"{"id":2,"guid":"context@default","method":"pause","params":{},"metadata":{}}"#,
    );
    time::timeout(Duration::from_secs(2), pending)
        .await
        .unwrap()
        .unwrap();
    let (reply, watermark) = run
        .call_with_events(
            r#"{"id":3,"guid":"debugger@default","method":"resume","params":{},"metadata":{}}"#,
        )
        .await
        .unwrap();
    assert!(
        serde_json::from_str::<Value>(&reply)
            .unwrap()
            .get("error")
            .is_none(),
        "{reply}"
    );
    let events: Vec<Value> =
        serde_json::from_str(&run.drain(i64::try_from(watermark).unwrap()).unwrap()).unwrap();
    assert_eq!(events.len(), 2, "{events:?}");
    assert!(
        events
            .iter()
            .all(|event| event["method"] == "pausedStateChanged"),
        "{events:?}"
    );
    paused.await.unwrap();
    assert_eq!(run.drain(-1).unwrap(), "[]");
    run.finish().await.unwrap();
    runtime.shutdown_hosted_runs().await;
}

/// Neo: Hundreds of test cases share one run, but call records retain the existing cap.
#[tokio::test(flavor = "multi_thread")]
async fn tests_that_a_worker_run_keeps_bounded_call_records() {
    let browser = FakeBrowser::start(Replies::default()).await;
    let runtime = browser.runtime().await;
    let run = runtime
        .open_hosted_run(Duration::from_mins(1), None)
        .await
        .unwrap();
    for id in 0..3000 {
        let message = json!({"id": id, "guid": run.init()["mainFrame"], "method": "title", "params": {}, "metadata": {}});
        let (reply, _) = run.call_with_events(&message.to_string()).await.unwrap();
        assert!(
            serde_json::from_str::<Value>(&reply)
                .unwrap()
                .get("error")
                .is_none(),
            "{reply}"
        );
    }
    let result = run.finish().await.unwrap();
    assert_eq!(result.status, RunStatus::Ok);
    assert_eq!(result.calls.len(), 1000);
    assert!(result.calls_dropped.is_some());
    runtime.shutdown_hosted_runs().await;
}

/// Neo: A synchronous getter already waiting on CDP must wake when the host finishes.
#[tokio::test(flavor = "multi_thread")]
async fn tests_that_finish_interrupts_a_synchronous_getter() {
    let browser = FakeBrowser::start(Replies {
        history: Reply {
            answers: 1,
            ..Reply::default()
        },
        ..Replies::default()
    })
    .await;
    let runtime = browser.runtime().await;
    let run = Arc::new(
        runtime
            .open_hosted_run(Duration::from_mins(1), None)
            .await
            .unwrap(),
    );
    let reader = Arc::clone(&run);
    let reading = task::spawn_blocking(move || {
        reader.get(&json!({"guid":reader.init()["page"],"field":"url"}).to_string())
    });
    time::timeout(Duration::from_secs(2), async {
        while browser
            .exchanges()
            .iter()
            .filter(|exchange| exchange.method() == "Page.getNavigationHistory")
            .count()
            < 2
        {
            time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(run.finish().await.unwrap().status, RunStatus::Ok);
    let error = time::timeout(Duration::from_secs(2), reading)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    assert_eq!(error.name, "TargetClosedError");
    runtime.shutdown_hosted_runs().await;
}

/// Neo: Getter cleanup cannot overwrite the deadline's attribution in an unfinished call record.
#[tokio::test(flavor = "multi_thread")]
async fn tests_that_deadline_records_keep_the_timeout_reason() {
    let browser = FakeBrowser::start(Replies {
        title: Reply {
            answers: 0,
            ..Reply::default()
        },
        ..Replies::default()
    })
    .await;
    let runtime = browser.runtime().await;
    let run = runtime
        .open_hosted_run(Duration::from_millis(200), None)
        .await
        .unwrap();
    let reply = run.call_with_events(
        &json!({"id":1,"guid":run.init()["mainFrame"],"method":"title","params":{},"metadata":{}})
            .to_string(),
    );
    let error = reply.await.unwrap_err();
    assert_eq!(error.name, "TimeoutError");
    let outcome = run.finish().await.unwrap();
    assert_eq!(outcome.status, RunStatus::Timeout);
    assert_eq!(
        outcome.calls[0].result,
        CallResult::Unfinished {
            reason: RunEnd::Timeout
        }
    );
    runtime.shutdown_hosted_runs().await;
}
