//! A click's mechanism under failure, against the fake browser in `support/fake_browser.rs`. No
//! browser needed.
//!
//! The layers split three ways: `click.rs`'s `FakeTab` tests own the policy (both loops, every
//! call-log line, the goldens byte for byte under paused time); `click_e2e.rs` owns fidelity
//! against a real page. This file owns what `FakeTab` replaces and a real browser cannot hold
//! still: what `PageSession` actually writes on the wire, in which order, and what it leaves
//! behind when an acknowledgement never comes or the run ends mid-press. No golden call log is
//! asserted here: real time over loopback makes attempt counts timing-bound.

#[path = "support/fake_browser.rs"]
mod fake_browser;

use std::time::{Duration, Instant};

use fake_browser::{Exchange, FakeBrowser, GLUE, PAGE_URL, Replies, Reply, run_request};
use neo_runtime::{CallResult, Delivery, RunEnd, RunStatus};
use serde_json::json;
use tokio::time;
use tokio_util::sync::CancellationToken;

/// How long a test waits for what goes out in the background after a run.
const SETTLE: Duration = Duration::from_secs(5);

/// A click with a long `delay` that the script does not await: it is mid-press when the script
/// returns 300 ms later.
const PRESS_AND_RETURN: &str = "page.click('#go', { delay: 5000 })
await new Promise((resolve) => setTimeout(resolve, 300))
return 'returned'";

/// Waits until `done` holds for the fake's exchanges, or [`SETTLE`] passes, and returns them.
async fn settle(browser: &FakeBrowser, done: impl Fn(&[Exchange]) -> bool) -> Vec<Exchange> {
    let give_up_at = Instant::now() + SETTLE;
    loop {
        let exchanges = browser.exchanges();
        if done(&exchanges) || Instant::now() >= give_up_at {
            return exchanges;
        }
        time::sleep(Duration::from_millis(20)).await;
    }
}

/// The glue function a `Runtime.callFunctionOn` named, if it called the glue.
fn glue_call(exchange: &Exchange) -> Option<&str> {
    let params = &exchange.command["params"];
    (exchange.method() == "Runtime.callFunctionOn" && params["objectId"] == GLUE)
        .then(|| params["arguments"][0]["value"].as_str())
        .flatten()
}

/// The `type` of an `Input.dispatchMouseEvent`.
fn mouse_event(exchange: &Exchange) -> Option<&str> {
    (exchange.method() == "Input.dispatchMouseEvent")
        .then(|| exchange.command["params"]["type"].as_str())
        .flatten()
}

fn position(exchanges: &[Exchange], matches: impl Fn(&Exchange) -> bool) -> Option<usize> {
    exchanges.iter().position(matches)
}

fn mouse(exchanges: &[Exchange], kind: &str) -> Vec<usize> {
    exchanges
        .iter()
        .enumerate()
        .filter(|(_, exchange)| mouse_event(exchange) == Some(kind))
        .map(|(index, _)| index)
        .collect()
}

fn detached(exchanges: &[Exchange]) -> bool {
    exchanges
        .iter()
        .any(|exchange| exchange.method() == "Target.detachFromTarget")
}

/// The session's release order: the press, then its release, the interceptor's stop and every
/// object-group release, and only then the detach.
fn assert_released_before_detach(exchanges: &[Exchange]) {
    let pressed = mouse(exchanges, "mousePressed");
    let released = mouse(exchanges, "mouseReleased");
    assert_eq!(pressed.len(), 1, "one press: {exchanges:#?}");
    assert_eq!(released.len(), 1, "one release: {exchanges:#?}");
    assert!(pressed[0] < released[0], "the press, then its release");
    let detach = position(exchanges, |exchange| {
        exchange.method() == "Target.detachFromTarget"
    })
    .unwrap_or_else(|| panic!("the session detaches: {exchanges:#?}"));
    let stop = position(exchanges, |exchange| {
        glue_call(exchange) == Some("stopHitTargetInterceptor")
    })
    .unwrap_or_else(|| panic!("the interceptor is disarmed: {exchanges:#?}"));
    let group_releases: Vec<usize> = exchanges
        .iter()
        .enumerate()
        .filter(|(_, exchange)| exchange.method() == "Runtime.releaseObjectGroup")
        .map(|(index, _)| index)
        .collect();
    assert_eq!(
        group_releases.len(),
        2,
        "the element's and the interceptor's groups: {exchanges:#?}"
    );
    for (what, index) in [("the release", released[0]), ("the stop", stop)]
        .into_iter()
        .chain(
            group_releases
                .iter()
                .map(|index| ("a group release", *index)),
        )
    {
        assert!(
            index < detach,
            "{what} (exchange {index}) goes out before the detach (exchange {detach})"
        );
    }
}

/// The fake holds `mouseReleased`'s answer forever and the call's 500 ms timeout fires. The
/// script sees `TimeoutError`; the record says `Unknown`, because the press was written and not
/// every acknowledgement came back; and no second press is ever sent.
#[tokio::test(flavor = "multi_thread")]
async fn a_timeout_after_the_press_is_unknown_and_never_presses_again() {
    let browser = FakeBrowser::start(Replies {
        mouse: Reply {
            answers: 2,
            ..Reply::default()
        },
        ..Replies::default()
    })
    .await;
    let outcome = browser
        .runtime()
        .await
        .run(run_request(
            "try { await page.click('#go', { timeout: 500 }) } catch (error) { return error.name }",
            Duration::from_secs(10),
            CancellationToken::new(),
        ))
        .await;

    assert_eq!(outcome.status, RunStatus::Ok, "{outcome:?}");
    assert_eq!(outcome.value, Some(json!("TimeoutError")));
    let [call] = outcome.calls.as_slice() else {
        panic!("one record: {outcome:?}");
    };
    assert_eq!(
        call.result,
        CallResult::Error {
            name: "TimeoutError".to_string(),
            message: "Timeout 500ms exceeded.".to_string(),
        }
    );
    assert_eq!(call.delivery, Some(Delivery::Unknown));
    let exchanges = browser.exchanges();
    assert_eq!(mouse(&exchanges, "mousePressed").len(), 1, "{exchanges:#?}");
    assert_eq!(
        mouse(&exchanges, "mouseReleased").len(),
        1,
        "{exchanges:#?}"
    );
}

/// The script returns while its click is mid-press. The writer cuts its delay short and sends
/// the release, the interceptor is disarmed and both object groups are released, all before the
/// session detaches, because each of those tasks holds the session. The record reads
/// `Unfinished { Settled }` with the delivery the writer proved.
#[tokio::test(flavor = "multi_thread")]
async fn a_pressed_button_is_released_before_the_session_detaches() {
    let browser = FakeBrowser::start(Replies::default()).await;
    let started = Instant::now();
    let outcome = browser
        .runtime()
        .await
        .run(run_request(
            PRESS_AND_RETURN,
            Duration::from_secs(10),
            CancellationToken::new(),
        ))
        .await;

    assert_eq!(outcome.status, RunStatus::Ok, "{outcome:?}");
    assert_eq!(outcome.value, Some(json!("returned")));
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "the 5 s delay was cut short, took {:?}",
        started.elapsed()
    );
    let [call] = outcome.calls.as_slice() else {
        panic!("one record: {outcome:?}");
    };
    assert_eq!(
        call.result,
        CallResult::Unfinished {
            reason: RunEnd::Settled
        }
    );
    assert_eq!(
        call.delivery,
        Some(Delivery::Done),
        "press and release acknowledged"
    );
    assert_released_before_detach(&settle(&browser, detached).await);
}

/// The same order when the caller drops `run`'s future mid-press instead: the dispatcher drops
/// with it, which ends the call, and ownership does the rest.
#[tokio::test(flavor = "multi_thread")]
async fn a_dropped_run_still_releases_the_button_before_the_session_detaches() {
    let browser = FakeBrowser::start(Replies::default()).await;
    let runtime = browser.runtime().await;
    let run = tokio::spawn(async move {
        runtime
            .run(run_request(
                PRESS_AND_RETURN,
                Duration::from_secs(10),
                CancellationToken::new(),
            ))
            .await
    });
    let pressed = settle(&browser, |exchanges| {
        !mouse(exchanges, "mousePressed").is_empty()
    })
    .await;
    assert_eq!(mouse(&pressed, "mousePressed").len(), 1, "{pressed:#?}");
    run.abort();
    assert!(run.await.is_err_and(|error| error.is_cancelled()));

    assert_released_before_detach(&settle(&browser, detached).await);
}

/// A script that only reads `page.url()` sends no `Page.enable`, no focus emulation and no page
/// call: the domains stay lazy, so a getter-only run keeps its single live CDP call per getter.
#[tokio::test(flavor = "multi_thread")]
async fn a_getter_only_run_enables_nothing() {
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
    assert_eq!(outcome.value, Some(json!(PAGE_URL)));
    let methods: Vec<String> = settle(&browser, detached)
        .await
        .iter()
        .map(|exchange| exchange.method().to_string())
        .collect();
    assert_eq!(
        methods,
        [
            "Browser.getTabInfo",
            "Browser.getTabInfo",
            "Target.attachToTarget",
            "Page.getNavigationHistory",
            "Page.getNavigationHistory",
            "Target.detachFromTarget",
        ],
        "the connect probe, the run's tab check, attach, its URL read, the script's getter, \
         the detach"
    );
}
