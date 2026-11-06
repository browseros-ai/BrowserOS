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

use fake_browser::{Exchange, FakeBrowser, GLUE, PAGE_URL, Replies, Reply};
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

/// Each exchange as its method, or the glue function or mouse event it sent: a failure message
/// that fits on a screen, where a full dump carries the 340 KB install bundle.
fn trail(exchanges: &[Exchange]) -> Vec<String> {
    exchanges
        .iter()
        .map(
            |exchange| match (glue_call(exchange), mouse_event(exchange)) {
                (Some(glue), _) => format!("glue {glue}"),
                (_, Some(event)) => format!("mouse {event}"),
                _ => exchange.method().to_string(),
            },
        )
        .collect()
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
        .run(fake_browser::run_request(
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
        .run(fake_browser::run_request(
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
            .run(fake_browser::run_request(
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

/// A trial whose deadline fires while the writer sleeps out its `delay` between press and
/// release. The writer cuts the delay short and releases, and the trial's interceptor, which
/// blocks every event, is still armed when the release lands, so the page's handlers see no
/// click. Every mouse event is answered 200 ms late, so a stop sent as soon as the call stopped
/// would land well before the release.
#[tokio::test(flavor = "multi_thread")]
async fn a_trial_cut_short_mid_press_still_delivers_no_click() {
    let browser = FakeBrowser::start(Replies {
        mouse: Reply {
            delay: Duration::from_millis(200),
            ..Reply::default()
        },
        ..Replies::default()
    })
    .await;
    let outcome = browser
        .runtime()
        .await
        .run(fake_browser::run_request(
            "try { await page.click('#go', { trial: true, delay: 5000, timeout: 700 }) } catch (error) { return error.name }",
            Duration::from_secs(10),
            CancellationToken::new(),
        ))
        .await;

    assert_eq!(outcome.value, Some(json!("TimeoutError")), "{outcome:?}");
    let exchanges = settle(&browser, detached).await;
    assert_eq!(
        mouse(&exchanges, "mouseReleased").len(),
        1,
        "{:?}",
        trail(&exchanges)
    );
    assert_eq!(
        browser.page_clicks(),
        0,
        "the trial reached the page's handlers: {:?}",
        trail(&exchanges)
    );
    assert_eq!(
        browser.armed_interceptor(),
        None,
        "disarmed once the release landed"
    );
}

/// A run cancelled while its trial click arms the hit-target interceptor. The page arms it
/// before it answers, and the answer comes after the cancel. That late answer is still read and
/// the interceptor stopped, so nothing stays in the document's injected script: a second run's
/// forced click, which arms nothing of its own, as the user's own clicks arm nothing, reaches
/// the page.
#[tokio::test(flavor = "multi_thread")]
async fn a_run_cancelled_while_arming_leaves_no_interceptor_behind() {
    let browser = FakeBrowser::start(Replies {
        arm: Reply {
            delay: Duration::from_millis(500),
            ..Reply::default()
        },
        ..Replies::default()
    })
    .await;
    let cancel = CancellationToken::new();
    let runtime = browser.runtime().await;
    let first = tokio::spawn({
        let cancel = cancel.clone();
        async move {
            runtime
                .run(fake_browser::run_request(
                    "await page.click('#go', { trial: true })",
                    Duration::from_secs(10),
                    cancel,
                ))
                .await
        }
    });
    let arming = settle(&browser, |exchanges| {
        exchanges
            .iter()
            .any(|exchange| glue_call(exchange) == Some("setupHitTargetInterceptor"))
    })
    .await;
    assert!(
        browser.armed_interceptor().is_some(),
        "{:?}",
        trail(&arming)
    );
    cancel.cancel();
    let first = first.await.expect("the first run");
    assert_eq!(first.status, RunStatus::Cancelled, "{first:?}");
    assert_eq!(
        first.calls.first().map(|call| call.delivery),
        Some(Some(Delivery::NotSent))
    );
    let exchanges = settle(&browser, detached).await;
    assert_eq!(
        browser.armed_interceptor(),
        None,
        "the late answer's interceptor is stopped: {:?}",
        trail(&exchanges)
    );

    let second = browser
        .runtime()
        .await
        .run(fake_browser::run_request(
            "await page.click('#go', { force: true }); return 'clicked'",
            Duration::from_secs(10),
            CancellationToken::new(),
        ))
        .await;
    assert_eq!(second.value, Some(json!("clicked")), "{second:?}");
    assert_eq!(
        browser.page_clicks(),
        1,
        "the forced click reached the page"
    );
}

/// A script that only reads `page.url()` sends no `Page.enable`, no focus emulation and no page
/// call: the domains stay lazy, so a getter-only run keeps its single live CDP call per getter.
#[tokio::test(flavor = "multi_thread")]
async fn a_getter_only_run_enables_nothing() {
    let browser = FakeBrowser::start(Replies::default()).await;
    let outcome = browser
        .runtime()
        .await
        .run(fake_browser::run_request(
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

/// The tab closes while the run's first browser call is enabling the page's domains. BrowserOS
/// never answers the `Page.enable` in flight, so only `Target.detachedFromTarget` can tell the
/// click; it ends at once with what Playwright's client throws for a closed page, not with its
/// 20 s timeout.
#[tokio::test(flavor = "multi_thread")]
async fn a_tab_closed_while_its_domains_are_enabled_ends_the_click() {
    let browser = FakeBrowser::start(Replies {
        closes_at_page_enable: true,
        ..Replies::default()
    })
    .await;
    let started = Instant::now();
    let outcome = browser
        .runtime()
        .await
        .run(fake_browser::run_request(
            "try { await page.click('#go', { timeout: 20000 }) } catch (error) { return [error.name, error.message] }",
            Duration::from_secs(30),
            CancellationToken::new(),
        ))
        .await;

    assert_eq!(outcome.status, RunStatus::Ok, "{outcome:?}");
    assert_eq!(
        outcome.value,
        Some(json!([
            "Error",
            "page.click: Target page, context or browser has been closed\nCall log:\n  - waiting for #go\n",
        ]))
    );
    let [call] = outcome.calls.as_slice() else {
        panic!("one record: {outcome:?}");
    };
    assert_eq!(
        (&call.result, call.delivery),
        (
            &CallResult::Error {
                name: "TargetClosedError".to_string(),
                message: "Target page, context or browser has been closed".to_string(),
            },
            Some(Delivery::NotSent)
        )
    );
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "the close ended the click, not its timeout: {:?}",
        started.elapsed()
    );
}
