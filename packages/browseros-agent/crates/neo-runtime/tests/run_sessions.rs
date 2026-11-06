//! A run attaches one CDP session to its tab and must detach it however the run ends. The
//! `Runtime`'s connection outlives every run, so a session a run leaves behind stays attached
//! until the browser exits. Against the fake browser in `support/fake_browser.rs`, which
//! records each session it hands out and each detach. No browser needed.

#[path = "support/fake_browser.rs"]
mod fake_browser;

use std::time::{Duration, Instant};

use fake_browser::{Exchange, FakeBrowser, PAGE_URL, Replies, Reply, run_request};
use futures_util::future;
use neo_runtime::RunStatus;
use serde_json::json;
use tokio::time;
use tokio_util::sync::CancellationToken;

/// How long a test waits for detaches, which go out in the background after a run returns.
const SETTLE: Duration = Duration::from_secs(5);

/// The session ids the fake's attach answers handed out.
fn handed_out(exchanges: &[Exchange]) -> Vec<String> {
    let mut sessions: Vec<String> = exchanges
        .iter()
        .filter(|exchange| exchange.method() == "Target.attachToTarget")
        .filter_map(|exchange| exchange.reply.as_ref()?["result"]["sessionId"].as_str())
        .map(str::to_string)
        .collect();
    sessions.sort();
    sessions
}

/// The session ids the fake was asked to detach, once per request.
fn detached(exchanges: &[Exchange]) -> Vec<String> {
    let mut sessions: Vec<String> = exchanges
        .iter()
        .filter(|exchange| exchange.method() == "Target.detachFromTarget")
        .filter_map(|exchange| exchange.command["params"]["sessionId"].as_str())
        .map(str::to_string)
        .collect();
    sessions.sort();
    sessions
}

/// Waits until the fake has handed out `count` sessions and been asked to detach each of them,
/// or [`SETTLE`] passes, and returns both lists as they then stand.
async fn settle(browser: &FakeBrowser, count: usize) -> (Vec<String>, Vec<String>) {
    let give_up_at = Instant::now() + SETTLE;
    loop {
        let exchanges = browser.exchanges();
        let sessions = (handed_out(&exchanges), detached(&exchanges));
        if (sessions.0.len() == count && sessions.0 == sessions.1) || Instant::now() >= give_up_at {
            return sessions;
        }
        time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_finished_run_detaches_its_session() {
    let browser = FakeBrowser::start(Replies::default()).await;
    let outcome = browser
        .runtime()
        .await
        .run(run_request(
            "return page.url()",
            Duration::from_secs(5),
            CancellationToken::new(),
        ))
        .await;

    assert_eq!(outcome.status, RunStatus::Ok, "{outcome:?}");
    assert_eq!(outcome.value, Some(json!(PAGE_URL)), "{outcome:?}");
    let (attached, detached) = settle(&browser, 1).await;
    assert_eq!(attached, ["session-1"], "{:?}", browser.exchanges());
    assert_eq!(detached, attached, "{:?}", browser.exchanges());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_run_cancelled_while_reading_its_first_url_detaches_its_session() {
    // The attach is answered; the history read that follows it at run start never is.
    let browser = FakeBrowser::start(Replies {
        history: Reply {
            answers: 0,
            ..Reply::default()
        },
        ..Replies::default()
    })
    .await;
    let runtime = browser.runtime().await;
    let cancel = CancellationToken::new();
    let request = run_request("return 1", Duration::from_secs(30), cancel.clone());
    let canceller = tokio::spawn(async move {
        time::sleep(Duration::from_millis(300)).await;
        cancel.cancel();
    });
    let outcome = runtime.run(request).await;
    canceller.await.unwrap();

    assert_eq!(outcome.status, RunStatus::Cancelled, "{outcome:?}");
    let (attached, detached) = settle(&browser, 1).await;
    assert_eq!(attached, ["session-1"], "{:?}", browser.exchanges());
    assert_eq!(
        detached,
        attached,
        "the session attached before the stop is detached: {:?}",
        browser.exchanges()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn attaches_answered_after_their_runs_timed_out_are_detached() {
    const RUNS: usize = 3;
    let browser = FakeBrowser::start(Replies {
        attach: Reply {
            delay: Duration::from_secs(1),
            ..Reply::default()
        },
        ..Replies::default()
    })
    .await;
    let runtime = browser.runtime().await;

    let outcomes = future::join_all((0..RUNS).map(|_| {
        runtime.run(run_request(
            "return 1",
            Duration::from_millis(200),
            CancellationToken::new(),
        ))
    }))
    .await;

    for outcome in &outcomes {
        assert_eq!(outcome.status, RunStatus::Timeout, "{outcome:?}");
    }
    assert!(
        handed_out(&browser.exchanges()).is_empty(),
        "every run stopped before the browser answered its attach: {:?}",
        browser.exchanges()
    );
    let (attached, detached) = settle(&browser, RUNS).await;
    assert_eq!(attached.len(), RUNS, "{:?}", browser.exchanges());
    assert_eq!(
        detached,
        attached,
        "each session the late answers handed out is detached: {:?}",
        browser.exchanges()
    );
}
