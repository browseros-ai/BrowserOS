//! A click's mechanism under failure, against the fake browser in `support/fake_browser.rs`. No
//! browser needed.
//!
//! The layers split three ways: `click.rs`'s `FakeTab` tests own the policy (both loops and their
//! order, under paused time); `click_e2e.rs` owns behaviour on a real page. This file owns what
//! `FakeTab` replaces and a real browser cannot hold still: what `PageSession` actually writes on
//! the wire, in which order, and what it leaves behind when an acknowledgement never comes or the
//! run ends mid-press. Nothing here asserts log text or attempt counts: real time over loopback
//! makes attempt counts timing-bound.

#[path = "support/fake_browser.rs"]
mod fake_browser;

use std::time::{Duration, Instant};

use fake_browser::{CommitDuring, Exchange, FakeBrowser, GLUE, PAGE_URL, Replies, Reply};
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

/// The trail's input steps only: interceptor setups and stops, and mouse events.
fn input_steps(exchanges: &[Exchange]) -> Vec<String> {
    trail(exchanges)
        .into_iter()
        .filter(|step| {
            step.starts_with("mouse ")
                || step == "glue setupHitTargetInterceptor"
                || step == "glue stopHitTargetInterceptor"
        })
        .collect()
}

/// One click's whole input phase on the wire: its interceptor's setup, its mouse events, and
/// the stop.
const ONE_INPUT_PHASE: [&str; 5] = [
    "glue setupHitTargetInterceptor",
    "mouse mouseMoved",
    "mouse mousePressed",
    "mouse mouseReleased",
    "glue stopHitTargetInterceptor",
];

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_a_setup_spanning_a_commit_is_stopped_before_retrying() {
    let browser = FakeBrowser::start(Replies {
        commit_during: Some(CommitDuring::Arm),
        arm: Reply {
            delay: Duration::from_millis(50),
            ..Reply::default()
        },
        ..Replies::default()
    })
    .await;
    let outcome = browser
        .runtime()
        .await
        .run(fake_browser::run_request(
            "await page.locator('#go').click(); return 'clicked'",
            Duration::from_secs(10),
            CancellationToken::new(),
        ))
        .await;
    assert_eq!(outcome.status, RunStatus::Ok, "{outcome:?}");
    assert_eq!(outcome.value, Some(json!("clicked")));
    let exchanges = settle(&browser, detached).await;
    let first_stop = exchanges
        .iter()
        .position(|exchange| {
            glue_call(exchange) == Some("stopHitTargetInterceptor")
                && exchange.command["params"]["arguments"][2]["objectId"] == "interceptor-1"
        })
        .expect("the first setup's answer must be owned and stopped");
    let second_setup = exchanges
        .iter()
        .enumerate()
        .filter(|(_, exchange)| glue_call(exchange) == Some("setupHitTargetInterceptor"))
        .nth(1)
        .map(|(index, _)| index)
        .expect("setup retries in the new document");
    assert!(
        first_stop < second_setup,
        "the old interceptor must stop before another input turn"
    );
    assert_eq!(browser.armed_interceptor(), None);
    assert_eq!(browser.page_clicks(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_a_late_setup_spanning_a_commit_is_stopped_after_timeout() {
    // The click's timeout leaves ample time to reach its setup on a slow machine; the setup's
    // answer lands well after that timeout.
    let browser = FakeBrowser::start(Replies {
        commit_during: Some(CommitDuring::Arm),
        arm: Reply {
            delay: Duration::from_secs(2),
            ..Reply::default()
        },
        ..Replies::default()
    })
    .await;
    let outcome = browser
        .runtime()
        .await
        .run(fake_browser::run_request(
            "try { await page.locator('#go').click({timeout:1000}); } catch(e) { return e.name; }",
            Duration::from_secs(10),
            CancellationToken::new(),
        ))
        .await;
    assert_eq!(outcome.status, RunStatus::Ok, "{outcome:?}");
    assert_eq!(outcome.value, Some(json!("TimeoutError")));
    let exchanges = settle(&browser, detached).await;
    assert!(
        armed(&exchanges),
        "the click must reach its setup before its timeout: {exchanges:#?}"
    );
    assert!(
        exchanges.iter().any(|exchange| {
            glue_call(exchange) == Some("stopHitTargetInterceptor")
                && exchange.command["params"]["arguments"][2]["objectId"] == "interceptor-1"
        }),
        "the late answer must still be stopped"
    );
    assert_eq!(browser.armed_interceptor(), None);
    assert_eq!(browser.page_clicks(), 0);
}

/// A forced click waiting out another run's trial on the wire: the trial's setup and stop, then
/// the forced click's mouse events. The trial sends no mouse event.
const FORCED_AFTER_A_TRIAL: [&str; 5] = [
    "glue setupHitTargetInterceptor",
    "glue stopHitTargetInterceptor",
    "mouse mouseMoved",
    "mouse mousePressed",
    "mouse mouseReleased",
];

/// Whether the fake has received a hit-target interceptor's setup.
fn armed(exchanges: &[Exchange]) -> bool {
    exchanges
        .iter()
        .any(|exchange| glue_call(exchange) == Some("setupHitTargetInterceptor"))
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

/// Delaying both setup replies makes the two clicks overlap while the browser still owns
/// only one hit-target interceptor. A trial presses nothing, so it never adds a click to the
/// real click beside it.
#[tokio::test(flavor = "multi_thread")]
async fn tests_that_concurrent_trial_and_real_click_deliver_only_one_click() {
    let browser = FakeBrowser::start(Replies {
        arm: Reply {
            delay: Duration::from_millis(100),
            ..Reply::default()
        },
        ..Replies::default()
    })
    .await;
    let outcome = browser
        .runtime()
        .await
        .run(fake_browser::run_request(
            "await Promise.all([page.click('#go', { trial: true }), page.click('#go')])",
            Duration::from_secs(10),
            CancellationToken::new(),
        ))
        .await;

    assert_eq!(outcome.status, RunStatus::Ok, "{outcome:?}");
    assert_eq!(outcome.calls.len(), 2, "both calls are recorded");
    let exchanges = browser.exchanges();
    assert_eq!(
        mouse(&exchanges, "mousePressed").len(),
        1,
        "the trial pressed nothing: {:?}",
        trail(&exchanges)
    );
    assert_eq!(
        browser.page_clicks(),
        1,
        "only the real click may reach the page, even when its trial overlaps"
    );
}

/// Independent runs share the tab's input device and hit-target interceptor as well.
#[tokio::test(flavor = "multi_thread")]
async fn tests_that_concurrent_runs_keep_trial_clicks_out_of_the_page() {
    let browser = FakeBrowser::start(Replies {
        arm: Reply {
            delay: Duration::from_millis(100),
            ..Reply::default()
        },
        ..Replies::default()
    })
    .await;
    let runtime = browser.runtime().await;
    let trial = fake_browser::run_request(
        "await page.click('#go', { trial: true })",
        Duration::from_secs(10),
        CancellationToken::new(),
    );
    let real = fake_browser::run_request(
        "await page.click('#go')",
        Duration::from_secs(10),
        CancellationToken::new(),
    );
    let (trial, real) = tokio::join!(runtime.run(trial), runtime.run(real));

    assert_eq!(trial.status, RunStatus::Ok, "{trial:?}");
    assert_eq!(real.status, RunStatus::Ok, "{real:?}");
    let exchanges = browser.exchanges();
    assert_eq!(
        mouse(&exchanges, "mousePressed").len(),
        1,
        "the trial pressed nothing: {:?}",
        trail(&exchanges)
    );
    assert_eq!(
        browser.page_clicks(),
        1,
        "a second run must not replace the first run's trial interceptor"
    );
}

/// Two ordinary clicks in flight on one tab take turns at its input: the second interceptor is
/// set up only once the first click's stop has answered, so neither click's events are judged
/// by the other's interceptor. Each setup answer is held 100 ms, which without turns lets both
/// setups land before either press.
#[tokio::test(flavor = "multi_thread")]
async fn concurrent_clicks_take_turns_from_setup_to_stop() {
    let browser = FakeBrowser::start(Replies {
        arm: Reply {
            delay: Duration::from_millis(100),
            ..Reply::default()
        },
        ..Replies::default()
    })
    .await;
    let outcome = browser
        .runtime()
        .await
        .run(fake_browser::run_request(
            "await Promise.all([page.click('#go'), page.click('#go')])",
            Duration::from_secs(10),
            CancellationToken::new(),
        ))
        .await;

    assert_eq!(outcome.status, RunStatus::Ok, "{outcome:?}");
    let ends: Vec<_> = outcome
        .calls
        .iter()
        .map(|call| (&call.result, call.delivery))
        .collect();
    assert_eq!(ends, [(&CallResult::Ok, Some(Delivery::Done)); 2]);
    let exchanges = settle(&browser, detached).await;
    assert_eq!(
        input_steps(&exchanges),
        ONE_INPUT_PHASE.repeat(2),
        "one whole input phase, then the other: {:?}",
        trail(&exchanges)
    );
    assert_eq!(browser.page_clicks(), 2);
}

/// A forced click arms nothing of its own, so it presses into whatever interceptor is armed.
/// A second run's forced click on the tab waits for the first run's trial to be stopped, then
/// reaches the page. The trial's setup answer is held 500 ms, so without turns the forced
/// click presses while the trial's interceptor is armed.
#[tokio::test(flavor = "multi_thread")]
async fn a_forced_click_in_another_run_waits_for_the_armed_trial() {
    let browser = FakeBrowser::start(Replies {
        arm: Reply {
            delay: Duration::from_millis(500),
            ..Reply::default()
        },
        ..Replies::default()
    })
    .await;
    let runtime = browser.runtime().await;
    let trial = tokio::spawn({
        let runtime = runtime.clone();
        async move {
            runtime
                .run(fake_browser::run_request(
                    "await page.click('#go', { trial: true })",
                    Duration::from_secs(10),
                    CancellationToken::new(),
                ))
                .await
        }
    });
    let arming = settle(&browser, armed).await;
    assert!(
        browser.armed_interceptor().is_some(),
        "{:?}",
        trail(&arming)
    );
    let forced = runtime
        .run(fake_browser::run_request(
            "await page.click('#go', { force: true }); return 'clicked'",
            Duration::from_secs(10),
            CancellationToken::new(),
        ))
        .await;
    let trial = trial.await.expect("the trial's run");

    assert_eq!(trial.status, RunStatus::Ok, "{trial:?}");
    assert_eq!(forced.value, Some(json!("clicked")), "{forced:?}");
    let exchanges = browser.exchanges();
    assert_eq!(
        input_steps(&exchanges),
        FORCED_AFTER_A_TRIAL,
        "{:?}",
        trail(&exchanges)
    );
    assert_eq!(
        browser.page_clicks(),
        1,
        "the forced click reached the page, the trial did not: {:?}",
        trail(&exchanges)
    );
}

/// A run cancelled while its trial's setup answer is held keeps the tab's input until that late
/// answer is read and its interceptor stopped. A forced click started right after the cancel,
/// while that interceptor is still armed, waits for the stop and then reaches the page.
#[tokio::test(flavor = "multi_thread")]
async fn a_cancelled_trial_keeps_the_tab_input_until_its_interceptor_is_stopped() {
    let browser = FakeBrowser::start(Replies {
        arm: Reply {
            delay: Duration::from_millis(500),
            ..Reply::default()
        },
        ..Replies::default()
    })
    .await;
    let runtime = browser.runtime().await;
    let cancel = CancellationToken::new();
    let trial = tokio::spawn({
        let runtime = runtime.clone();
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
    let arming = settle(&browser, armed).await;
    assert!(
        browser.armed_interceptor().is_some(),
        "{:?}",
        trail(&arming)
    );
    cancel.cancel();
    let (trial, forced) = tokio::join!(
        trial,
        runtime.run(fake_browser::run_request(
            "await page.click('#go', { force: true }); return 'clicked'",
            Duration::from_secs(10),
            CancellationToken::new(),
        ))
    );
    let trial = trial.expect("the trial's run");

    assert_eq!(trial.status, RunStatus::Cancelled, "{trial:?}");
    assert_eq!(forced.value, Some(json!("clicked")), "{forced:?}");
    let exchanges = settle(&browser, detached).await;
    assert_eq!(
        input_steps(&exchanges),
        FORCED_AFTER_A_TRIAL,
        "{:?}",
        trail(&exchanges)
    );
    assert_eq!(
        browser.page_clicks(),
        1,
        "the forced click waited for the late interceptor's stop: {:?}",
        trail(&exchanges)
    );
    assert_eq!(browser.armed_interceptor(), None, "{:?}", trail(&exchanges));
}

/// A click whose call times out mid-press leaves its writer to release the button 200 ms later.
/// The run's next click waits for that release and for the first click's interceptor to be
/// stopped before it sets up its own: one whole input phase, then the other. Set up any
/// earlier, it would replace an interceptor whose click had not finished.
#[tokio::test(flavor = "multi_thread")]
async fn tests_that_a_click_cut_short_mid_press_keeps_the_tab_input_until_its_release_lands() {
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
            "const first = page.click('#go', { delay: 5000, timeout: 700 })
  .catch((error) => error.name)
await new Promise((resolve) => setTimeout(resolve, 800))
await page.click('#go')
return await first",
            Duration::from_secs(10),
            CancellationToken::new(),
        ))
        .await;

    assert_eq!(outcome.value, Some(json!("TimeoutError")), "{outcome:?}");
    let exchanges = settle(&browser, detached).await;
    assert_eq!(
        input_steps(&exchanges),
        ONE_INPUT_PHASE.repeat(2),
        "the second click set up only once the first had released and stopped: {:?}",
        trail(&exchanges)
    );
    assert_eq!(browser.armed_interceptor(), None, "{:?}", trail(&exchanges));
}

/// The same cut short by the script's `signal` (slice S): an abort ends the call as its timeout
/// would, so the writer still releases the button, and the next click waits for that release.
#[tokio::test(flavor = "multi_thread")]
async fn tests_that_a_click_aborted_mid_press_keeps_the_tab_input_until_its_release_lands() {
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
            "const controller = new AbortController()
setTimeout(() => controller.abort(), 700)
const first = page.click('#go', { delay: 5000, timeout: 0, signal: controller.signal })
  .catch((error) => error.name)
await new Promise((resolve) => setTimeout(resolve, 800))
await page.click('#go')
return await first",
            Duration::from_secs(10),
            CancellationToken::new(),
        ))
        .await;

    assert_eq!(outcome.value, Some(json!("AbortError")), "{outcome:?}");
    let exchanges = settle(&browser, detached).await;
    assert_eq!(
        input_steps(&exchanges),
        ONE_INPUT_PHASE.repeat(2),
        "the second click set up only once the aborted one had released and stopped: {:?}",
        trail(&exchanges)
    );
    assert_eq!(browser.armed_interceptor(), None, "{:?}", trail(&exchanges));
}

/// An abort after the press, with `mouseReleased`'s answer held: `AbortError`, the delivery
/// `Unknown` as for a timeout there, and no second press.
#[tokio::test(flavor = "multi_thread")]
async fn an_abort_after_the_press_is_unknown_and_never_presses_again() {
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
            "const controller = new AbortController()
setTimeout(() => controller.abort('stop'), 500)
try { await page.click('#go', { timeout: 0, signal: controller.signal }) } catch (error) { return error.name }",
            Duration::from_secs(10),
            CancellationToken::new(),
        ))
        .await;

    assert_eq!(outcome.status, RunStatus::Ok, "{outcome:?}");
    assert_eq!(outcome.value, Some(json!("AbortError")));
    let [call] = outcome.calls.as_slice() else {
        panic!("one record: {outcome:?}");
    };
    assert!(matches!(&call.result, CallResult::Error { name, .. } if name == "AbortError"));
    assert_eq!(call.delivery, Some(Delivery::Unknown));
    let exchanges = browser.exchanges();
    assert_eq!(mouse(&exchanges, "mousePressed").len(), 1, "{exchanges:#?}");
    assert_eq!(
        mouse(&exchanges, "mouseReleased").len(),
        1,
        "{exchanges:#?}"
    );
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
    assert!(
        matches!(&call.result, CallResult::Error { name, .. } if name == "TimeoutError"),
        "{call:?}"
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

/// A press that lands on another element after the hit check passed, cut off by its deadline
/// while the writer sleeps out its `delay` before the release. The interceptor blocked the
/// press and blocks the rest of its click; the writer cuts the delay short and releases, and
/// the interceptor is still armed when the release lands, so the page's handlers see no click.
/// Every mouse event is answered 200 ms late, so a stop sent as soon as the call stopped would
/// land well before the release.
#[tokio::test(flavor = "multi_thread")]
async fn tests_that_a_blocked_press_cut_short_never_reaches_the_page() {
    let browser = FakeBrowser::start(Replies {
        mouse: Reply {
            delay: Duration::from_millis(200),
            ..Reply::default()
        },
        press_lands_elsewhere: true,
        ..Replies::default()
    })
    .await;
    let outcome = browser
        .runtime()
        .await
        .run(fake_browser::run_request(
            "try { await page.click('#go', { delay: 5000, timeout: 700 }) } catch (error) { return error.name }",
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
        "the blocked press reached the page's handlers: {:?}",
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
    let value = outcome.value.clone().unwrap_or_default();
    assert_eq!(value[0], json!("Error"), "{value}");
    assert!(
        value[1].as_str().is_some_and(|message| message
            .starts_with("page.click: Target page, context or browser has been closed")),
        "{value}"
    );
    let [call] = outcome.calls.as_slice() else {
        panic!("one record: {outcome:?}");
    };
    assert!(
        matches!(&call.result, CallResult::Error { name, .. } if name == "TargetClosedError"),
        "{call:?}"
    );
    assert_eq!(call.delivery, Some(Delivery::NotSent), "{call:?}");
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "the close ended the click, not its timeout: {:?}",
        started.elapsed()
    );
}
