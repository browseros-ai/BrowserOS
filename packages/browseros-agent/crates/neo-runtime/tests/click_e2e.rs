//! Clicks end to end on a real BrowserOS neo. Each case serves a page from
//! `crates/neo-sdk/tests/pages/`, runs a Neo script against it, and checks what the script saw,
//! the run's record (its result and delivery) and what the page's own listeners saw. A failure is
//! checked by its error kind and by the reason its log gives, never by Playwright's exact wording,
//! call-log layout or retry counts.
//!
//! The cases: an ordinary click, which reaches its target and not the button beside it; strict,
//! hidden, covered and disabled buttons; a press an overlay catches after the hit check; a sticky
//! header; a link to a slow page; a dialog; a run that ends mid-press; a trial; a closed tab; the
//! install's reuse; a frame selector; a background tab; and concurrent clicks.
//! `crates/neo-sdk/tests/parity/click.test.mjs` runs the same pages through live Playwright beside
//! Neo.
//!
//! Runs only when `NEO_E2E_BROWSER` names the browser binary (`support/browser.rs`); otherwise
//! each test passes as a skip.

#[path = "support/browser.rs"]
mod browser;

use std::collections::BTreeSet;
use std::env;
use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use browser::{Fixture, Tab};
use browseros_cdp::{CdpEvent, SessionId};
use neo_runtime::{CallRecord, CallResult, Delivery, RunEnd, RunOutcome, RunStatus};
use serde_json::{Value, json};
use tokio::sync::broadcast;
use tokio::sync::broadcast::error::TryRecvError;
use tokio::time;

/// One `Submit` button whose `onclick` sets `#done`, a `Cancel` button beside it whose `onclick`
/// sets `#decoy`, and a recorder of every pointer and mouse event in `window.__events`.
const OK_PAGE: &str = include_str!("../../neo-sdk/tests/pages/ok.html");
/// Two `Submit` buttons.
const STRICT_PAGE: &str = include_str!("../../neo-sdk/tests/pages/strict.html");
/// A `display:none` `Submit` button.
const HIDDEN_PAGE: &str = include_str!("../../neo-sdk/tests/pages/hidden.html");
/// A `Submit` button under a full-viewport overlay, `<div id="overlay">`.
const COVERED_PAGE: &str = include_str!("../../neo-sdk/tests/pages/covered.html");
/// A disabled `Submit` button.
const DISABLED_PAGE: &str = include_str!("../../neo-sdk/tests/pages/disabled.html");
/// A button under a sticky header that covers the top 55% of the viewport.
const STICKY_PAGE: &str = include_str!("../../neo-sdk/tests/pages/sticky.html");
/// A link to `/slow`, which the fixture answers late.
const LINK_PAGE: &str = include_str!("../../neo-sdk/tests/pages/link.html");
/// A button whose `onclick` opens a `confirm()` and keeps its answer in `window.__answer`.
const DIALOG_PAGE: &str = include_str!("../../neo-sdk/tests/pages/dialog.html");
/// A button whose overlay appears on the click's own `mousemove` and goes after the
/// `pointerdown` it caught, so exactly one press is blocked after the hit check passed. The page
/// counts the button's own clicks in `window.__clicks`.
const BLOCKED_ONCE_PAGE: &str = include_str!("../../neo-sdk/tests/pages/blocked-once.html");

/// The ordinary click: Playwright's `getByRole` locator on `OK_PAGE`, then the URL its handler
/// moved to.
const SUBMIT_THEN_URL: &str = "await page.getByRole('button', { name: 'Submit' }).click()
return page.url()";

/// Opens a fresh browser on one page.
async fn open_page(path: &str, html: &str) -> Option<Tab> {
    let binary = browser::e2e_browser()?;
    let fixture = Fixture {
        pages: vec![(path.to_string(), html.to_string())],
        ..Fixture::default()
    };
    Some(Tab::open(binary, fixture, path).await)
}

async fn run(tab: &Tab, code: &str) -> RunOutcome {
    tab.start_run(code.to_string())
        .await
        .await
        .unwrap_or_else(|error| panic!("the run's task: {error}"))
}

/// On a real page, overlapping trials cannot add input to the ordinary clicks beside them.
#[tokio::test(flavor = "multi_thread")]
async fn tests_that_concurrent_trials_do_not_click_a_real_page() {
    let html = r#"<!doctype html><html><body>
<button id="go" onclick="window.__clicks++">Go</button>
<script>window.__clicks = 0</script>
</body></html>"#;
    let Some(tab) = open_page("/concurrent-clicks.html", html).await else {
        return;
    };
    let outcome = run(
        &tab,
        "for (let i = 0; i < 10; i++) {
  await Promise.all([
    page.locator('#go').click({ trial: true, delay: 20 }),
    page.locator('#go').click({ delay: 20 }),
  ])
}",
    )
    .await;
    let clicks = tab.evaluate("window.__clicks").await;
    tab.close().await;

    assert_eq!(outcome.status, RunStatus::Ok, "{outcome:?}");
    assert_eq!(
        clicks,
        json!(10),
        "only the ten real clicks may reach the page"
    );
}

/// Two ordinary clicks in flight, each on its own button, both land, and each record says so.
/// Neither click's events may be judged by the other's hit-target interceptor: one that is
/// blocks the click while its own interceptor, which never saw an event, reads `done`.
#[tokio::test(flavor = "multi_thread")]
async fn concurrent_clicks_on_two_buttons_each_reach_their_button() {
    let html = r#"<!doctype html><html><body>
<button id="a" onclick="window.__a++">A</button>
<button id="b" onclick="window.__b++">B</button>
<script>window.__a = 0; window.__b = 0</script>
</body></html>"#;
    let Some(tab) = open_page("/two-buttons.html", html).await else {
        return;
    };
    let outcome = run(
        &tab,
        "for (let i = 0; i < 10; i++) {
  await Promise.all([page.locator('#a').click(), page.locator('#b').click()])
}",
    )
    .await;
    let clicks = tab.evaluate("[window.__a, window.__b]").await;
    tab.close().await;

    assert_eq!(outcome.status, RunStatus::Ok, "{outcome:?}");
    assert_eq!(
        clicks,
        json!([10, 10]),
        "every click reached its own button"
    );
    let ends: BTreeSet<_> = outcome
        .calls
        .iter()
        .map(|call| (format!("{:?}", call.result), call.delivery))
        .collect();
    assert_eq!(
        ends,
        BTreeSet::from([("Ok".to_string(), Some(Delivery::Done))]),
        "{:#?}",
        outcome.calls
    );
    assert_eq!(outcome.calls.len(), 20);
}

fn only_record(outcome: &RunOutcome) -> &CallRecord {
    let [call] = outcome.calls.as_slice() else {
        panic!("one browser call: {outcome:#?}");
    };
    call
}

/// Polls `expression` in the page until `done` holds for its value, for up to five seconds,
/// and returns the last value read.
async fn eventually(tab: &Tab, expression: &str, done: impl Fn(&Value) -> bool) -> Value {
    let give_up_at = Instant::now() + Duration::from_secs(5);
    loop {
        let value = tab.evaluate(expression).await;
        if done(&value) || Instant::now() >= give_up_at {
            return value;
        }
        time::sleep(Duration::from_millis(50)).await;
    }
}

/// The `type` of each event the `OK_PAGE` recorder saw.
fn event_types(events: &Value) -> Vec<&str> {
    events
        .as_array()
        .map(|events| {
            events
                .iter()
                .filter_map(|event| event["type"].as_str())
                .collect()
        })
        .unwrap_or_default()
}

/// `ok`: the click resolves to `undefined` and reaches the `Submit` button, not the `Cancel`
/// button beside it, and its handler's `#done` is visible to `page.url()` read right after the
/// click, as under Playwright. The page saw one real click's events, all trusted and all on the
/// target. One record: title `Click`, delivery `Done`.
#[tokio::test(flavor = "multi_thread")]
async fn ok_clicks_and_the_hash_is_visible_to_page_url() {
    let Some(tab) = open_page("/ok.html", OK_PAGE).await else {
        return;
    };
    let outcome = run(
        &tab,
        "const result = await page.getByRole('button', { name: 'Submit' }).click()
return { undefined: result === undefined, url: page.url() }",
    )
    .await;
    let events = tab.evaluate("window.__events").await;
    tab.close().await;

    assert_eq!(outcome.status, RunStatus::Ok, "{outcome:#?}");
    let value = outcome.value.clone().unwrap_or_default();
    assert_eq!(
        value["undefined"],
        json!(true),
        "click() resolves to undefined: {value}"
    );
    let url = value["url"].as_str().unwrap_or_default();
    assert!(
        url.ends_with("/ok.html#done"),
        "the target's handler ran, not the decoy's: {url:?}"
    );
    let call = only_record(&outcome);
    assert_eq!(
        (call.title.as_deref(), &call.result, call.delivery),
        (Some("Click"), &CallResult::Ok, Some(Delivery::Done)),
        "{call:#?}"
    );
    assert_eq!(
        event_types(&events),
        [
            "pointermove",
            "pointerdown",
            "mousedown",
            "pointerup",
            "mouseup",
            "click"
        ],
        "one real click: {events:#}"
    );
    let events = events.as_array().cloned().unwrap_or_default();
    let targets: BTreeSet<&str> = events
        .iter()
        .filter_map(|event| event["target"].as_str())
        .collect();
    assert_eq!(
        targets,
        BTreeSet::from(["button"]),
        "every event on the target: {events:#?}"
    );
    assert!(
        events.iter().all(|event| event["isTrusted"] == json!(true)),
        "real input, not script events: {events:#?}"
    );
}

/// `strict`: two matching buttons end the call at once with a strict mode violation, well under
/// the timeout, with nothing delivered.
#[tokio::test(flavor = "multi_thread")]
async fn strict_mode_fails_at_once_without_input() {
    let Some(tab) = open_page("/strict.html", STRICT_PAGE).await else {
        return;
    };
    let outcome = run(
        &tab,
        "await page.getByRole('button', { name: 'Submit' }).click()",
    )
    .await;
    let events = tab.evaluate("window.__events").await;
    tab.close().await;

    assert_eq!(outcome.status, RunStatus::Error, "{outcome:#?}");
    let error = outcome.error.as_ref().expect("a failed run says why");
    assert_eq!(error.name, "Error", "{error:?}");
    assert!(
        error.message.starts_with("locator.click: ")
            && error.message.contains("strict mode violation"),
        "{}",
        error.message
    );
    let call = only_record(&outcome);
    assert_eq!(call.delivery, Some(Delivery::NotSent), "{call:#?}");
    assert!(
        call.duration < Duration::from_secs(2),
        "a strict violation fails at once, took {:?}",
        call.duration
    );
    assert_eq!(events, json!([]), "no input reached the page");
}

/// `hidden`: `getByRole` never matches a `display:none` button, so the click runs out its 500 ms
/// and ends with `TimeoutError`, nothing delivered.
#[tokio::test(flavor = "multi_thread")]
async fn a_hidden_button_times_out_without_input() {
    let Some(tab) = open_page("/hidden.html", HIDDEN_PAGE).await else {
        return;
    };
    let outcome = run(
        &tab,
        "await page.getByRole('button', { name: 'Submit' }).click({ timeout: 500 })",
    )
    .await;
    let events = tab.evaluate("window.__events").await;
    tab.close().await;

    assert_eq!(outcome.status, RunStatus::Error, "{outcome:#?}");
    let error = outcome.error.as_ref().expect("a failed run says why");
    assert_eq!(error.name, "TimeoutError", "{error:?}");
    let call = only_record(&outcome);
    assert!(
        matches!(&call.result, CallResult::Error { name, .. } if name == "TimeoutError"),
        "{call:#?}"
    );
    assert_eq!(call.delivery, Some(Delivery::NotSent), "{call:#?}");
    assert!(
        call.duration >= Duration::from_millis(500) && call.duration < Duration::from_secs(2),
        "the call ran out its 500 ms, took {:?}",
        call.duration
    );
    assert_eq!(events, json!([]), "no input reached the page");
}

/// `covered` and `disabled`: `TimeoutError`, nothing delivered, and the error says why: it names
/// the overlay that covers the button, or the state the button lacks.
#[tokio::test(flavor = "multi_thread")]
async fn covered_and_disabled_buttons_time_out_naming_the_reason() {
    for (path, html, timeout, reason) in [
        ("/covered.html", COVERED_PAGE, 1000, "id=\"overlay\""),
        ("/disabled.html", DISABLED_PAGE, 500, "not enabled"),
    ] {
        let Some(tab) = open_page(path, html).await else {
            return;
        };
        let code = format!(
            "await page.getByRole('button', {{ name: 'Submit' }}).click({{ timeout: {timeout} }})"
        );
        let outcome = run(&tab, &code).await;
        let events = tab.evaluate("window.__events").await;
        tab.close().await;

        assert_eq!(outcome.status, RunStatus::Error, "{path}: {outcome:#?}");
        let error = outcome.error.as_ref().expect("a failed run says why");
        assert_eq!(error.name, "TimeoutError", "{path}: {error:?}");
        assert!(
            error.message.contains(reason),
            "{path}: the error names why ({reason}):\n{}",
            error.message
        );
        let call = only_record(&outcome);
        assert_eq!(call.delivery, Some(Delivery::NotSent), "{path}: {call:#?}");
        assert_eq!(events, json!([]), "{path}: no input reached the page");
    }
}

/// The page the spec test clicks: a button under a full-page overlay that the test removes a
/// moment after the run starts, so loop B's retries 1 to 3 scroll with the `scrollIntoView`
/// glue before the click lands; and a bordered button clicked at a `position`, which reads its
/// box model and `getElementBorderWidth`.
const CALLS_PAGE: &str = r#"<!doctype html><html><head><meta charset="utf-8"><title>calls</title>
<script>window.__clicked = []</script></head><body>
<button id="covered" onclick="__clicked.push('covered')">Covered</button>
<button id="offset" style="border: 3px solid black" onclick="__clicked.push('offset:' + event.offsetX + ',' + event.offsetY)">Offset</button>
<div id="overlay" style="position:fixed;top:0;left:0;width:100vw;height:100vh;z-index:10"></div>
</body></html>"#;

/// "Each glue function returns its spec'd shape", on a real page. Rust's strict decode is the
/// oracle: a result outside the spec ends the click with an `Error` naming the page call
/// (`PageError::Contract`), which fails this test. Between them the two clicks call every
/// click page call: `describeSelector`, `resolve`, `checkElementStates`, `scrollIntoView`
/// (retries 1 to 3), `viewportSize`, `getElementBorderWidth`, `setupHitTargetInterceptor` and
/// `stopHitTargetInterceptor`.
#[tokio::test(flavor = "multi_thread")]
async fn every_click_page_call_decodes_against_the_spec() {
    let Some(binary) = browser::e2e_browser() else {
        return;
    };
    let fixture = Fixture {
        pages: vec![("/calls.html".to_string(), CALLS_PAGE.to_string())],
        ..Fixture::default()
    };
    let tab = Tab::open(binary, fixture, "/calls.html").await;
    tab.evaluate("setTimeout(() => document.getElementById('overlay').remove(), 600)")
        .await;
    let outcome = run(
        &tab,
        "await page.locator('#covered').click({ timeout: 10000 })
await page.locator('#offset').click({ position: { x: 5, y: 6 }, timeout: 10000 })
return 'clicked'",
    )
    .await;
    let clicked = tab.evaluate("window.__clicked").await;
    tab.close().await;

    assert_eq!(outcome.status, RunStatus::Ok, "{outcome:#?}");
    assert_eq!(outcome.calls.len(), 2, "{outcome:#?}");
    for call in &outcome.calls {
        assert_eq!(
            (&call.result, call.delivery),
            (&CallResult::Ok, Some(Delivery::Done)),
            "{call:#?}"
        );
    }
    let covered = outcome.calls[0].log.join("\n");
    assert!(
        covered.contains("id=\"overlay\""),
        "the overlay held the first attempts, so later ones scrolled with the glue:\n{covered}"
    );
    // The offset is relative to the padding box, as Playwright's `_offsetPoint` makes it.
    assert_eq!(clicked, json!(["covered", "offset:5,6"]));
}

/// A `confirm()` opened by the click's own handler. The open dialog holds the renderer, so the
/// mouse acknowledgement waits for its answer; the watcher dismisses it (Playwright's answer
/// with no dialog listener, so `confirm` returns `false`), and the click resolves with delivery
/// `Done`. `confirm` rather than `alert`, because its return value shows which answer the
/// page got.
#[tokio::test(flavor = "multi_thread")]
async fn a_dialog_opened_by_the_click_is_dismissed_and_the_click_resolves() {
    let Some(tab) = open_page("/dialog.html", DIALOG_PAGE).await else {
        return;
    };
    let outcome = run(
        &tab,
        "await page.getByRole('button', { name: 'Submit' }).click()
return 'clicked'",
    )
    .await;
    let answer = tab.evaluate("window.__answer").await;
    tab.close().await;

    assert_eq!(outcome.status, RunStatus::Ok, "{outcome:#?}");
    assert_eq!(outcome.value, Some(json!("clicked")));
    let call = only_record(&outcome);
    assert_eq!(
        (&call.result, call.delivery),
        (&CallResult::Ok, Some(Delivery::Done)),
        "{call:#?}"
    );
    assert_eq!(answer, json!(false), "the watcher dismissed the confirm");
}

/// G3: a run that ends while its click holds the button down still releases it before the
/// session detaches, so the page sees `mouseup`. Twice: the script returns without awaiting a
/// click with `delay: 2000` (record `Unfinished { Settled }`), and the run's deadline lands
/// inside a `delay: 5000` (status `Timeout`, record `Unfinished { Timeout }`). Either way the
/// delay is cut short and the press and its release were acknowledged: delivery `Done`.
#[tokio::test(flavor = "multi_thread")]
async fn a_click_cut_off_by_the_run_still_releases_the_button() {
    let cases = [
        (
            "the script returns",
            "page.getByRole('button', { name: 'Submit' }).click({ delay: 2000 })
await new Promise((resolve) => setTimeout(resolve, 500))
return 'returned'",
            Duration::from_secs(30),
            RunStatus::Ok,
            RunEnd::Settled,
        ),
        (
            "the run's deadline",
            "await page.getByRole('button', { name: 'Submit' }).click({ delay: 5000 })",
            Duration::from_millis(1500),
            RunStatus::Timeout,
            RunEnd::Timeout,
        ),
    ];
    for (end, code, timeout, status, reason) in cases {
        let Some(tab) = open_page("/ok.html", OK_PAGE).await else {
            return;
        };
        let started = Instant::now();
        let outcome = tab
            .start_run_within(code.to_string(), timeout)
            .await
            .await
            .unwrap_or_else(|error| panic!("the run's task: {error}"));
        let took = started.elapsed();
        let events = eventually(&tab, "window.__events", |events| {
            event_types(events).contains(&"mouseup")
        })
        .await;
        tab.close().await;

        assert_eq!(outcome.status, status, "{end}: {outcome:#?}");
        let call = only_record(&outcome);
        assert_eq!(
            (&call.result, call.delivery),
            (&CallResult::Unfinished { reason }, Some(Delivery::Done)),
            "{end}: press and release acknowledged: {call:#?}"
        );
        assert!(
            took < Duration::from_secs(3),
            "{end}: the delay was cut short, the run took {took:?}"
        );
        let types = event_types(&events);
        let pressed = types.iter().position(|kind| *kind == "mousedown");
        let released = types.iter().position(|kind| *kind == "mouseup");
        assert!(
            pressed.is_some() && released > pressed,
            "{end}: the page saw the press, then its release: {types:?}"
        );
    }
}

/// G2: the first press lands on an overlay that appeared after the hit check. The interceptor
/// blocks it and names the overlay, delivery goes back to `NotSent`, and the click presses again
/// on the same element. The second press lands: the button's own handler ran once, delivery
/// `Done`. The same under `noWaitAfter`, which skips only the navigation wait: Playwright reads
/// no verdict then, and resolves on the blocked press.
#[tokio::test(flavor = "multi_thread")]
async fn tests_that_a_verified_block_presses_again_with_or_without_no_wait_after() {
    for options in ["", "{ noWaitAfter: true }"] {
        let Some(tab) = open_page("/blocked-once.html", BLOCKED_ONCE_PAGE).await else {
            return;
        };
        let outcome = run(
            &tab,
            &format!(
                "await page.getByRole('button', {{ name: 'Submit' }}).click({options})
return 'clicked'"
            ),
        )
        .await;
        let clicks = tab.evaluate("window.__clicks").await;
        tab.close().await;

        assert_eq!(outcome.status, RunStatus::Ok, "{options}: {outcome:#?}");
        let call = only_record(&outcome);
        assert_eq!(
            (&call.result, call.delivery),
            (&CallResult::Ok, Some(Delivery::Done)),
            "{options}: {call:#?}"
        );
        assert!(
            call.log.join("\n").contains("id=\"overlay\""),
            "{options}: the first press was caught by the overlay: {:#?}",
            call.log
        );
        assert_eq!(
            clicks,
            json!(1),
            "{options}: the button's handler ran once, never for the blocked press"
        );
    }
}

/// `trial: true` on the `ok` page: the checks, the hit check included, and no input. The click
/// resolves, the page's listeners see no event at all, neither button's `onclick` runs (the hash
/// stays empty), and delivery is `NotSent`.
#[tokio::test(flavor = "multi_thread")]
async fn tests_that_a_trial_click_sends_no_input() {
    let Some(tab) = open_page("/ok.html", OK_PAGE).await else {
        return;
    };
    let outcome = run(
        &tab,
        "await page.getByRole('button', { name: 'Submit' }).click({ trial: true })
return page.url()",
    )
    .await;
    let hash = tab.evaluate("location.hash").await;
    let events = tab.evaluate("window.__events").await;
    tab.close().await;

    assert_eq!(outcome.status, RunStatus::Ok, "{outcome:#?}");
    let url = outcome
        .value
        .as_ref()
        .and_then(Value::as_str)
        .unwrap_or_default();
    assert!(url.ends_with("/ok.html"), "the url never moved: {url:?}");
    let call = only_record(&outcome);
    assert_eq!(
        (&call.result, call.delivery),
        (&CallResult::Ok, Some(Delivery::NotSent)),
        "{call:#?}"
    );
    assert_eq!(hash, json!(""), "the button's onclick never ran");
    assert_eq!(
        events,
        json!([]),
        "no pointer or mouse event reached the page"
    );
}

/// The test closes the tab while a click polls for a button that never matches. The script
/// catches what Playwright's client throws for it, a plain `Error` (Playwright's
/// `TargetClosedError` sets no name) saying the target closed, long before the click's 20 s
/// timeout. The run's watcher learns of the close from `Target.detachedFromTarget`.
#[tokio::test(flavor = "multi_thread")]
async fn a_tab_closed_mid_click_fails_with_target_closed() {
    let Some(tab) = open_page("/hidden.html", HIDDEN_PAGE).await else {
        return;
    };
    let started = Instant::now();
    let run = tab
        .start_run(
            "try {
  await page.getByRole('button', { name: 'Submit' }).click({ timeout: 20000 })
} catch (error) {
  return { name: error.name, message: error.message }
}"
            .to_string(),
        )
        .await;
    time::sleep(Duration::from_millis(500)).await;
    tab.cdp
        .send(
            "Target.closeTarget",
            json!({ "targetId": tab.target_id }),
            None,
        )
        .await
        .unwrap_or_else(|error| panic!("close the tab: {error}"));
    let outcome = run
        .await
        .unwrap_or_else(|error| panic!("the run's task: {error}"));
    let took = started.elapsed();
    tab.close().await;

    assert_eq!(outcome.status, RunStatus::Ok, "{outcome:#?}");
    let value = outcome.value.clone().unwrap_or_default();
    assert_eq!(value["name"], json!("Error"), "{value}");
    let message = value["message"].as_str().unwrap_or_default();
    assert!(
        message.starts_with("locator.click: Target page, context or browser has been closed"),
        "{message}"
    );
    let call = only_record(&outcome);
    assert!(
        matches!(&call.result, CallResult::Error { name, .. } if name == "TargetClosedError"),
        "{call:#?}"
    );
    assert_eq!(call.delivery, Some(Delivery::NotSent), "{call:#?}");
    assert!(
        took < Duration::from_secs(5),
        "the close ended the click long before its timeout, took {took:?}"
    );
}

/// Counts installs of Neo's injected bundle in a tab, from the test's own session. With the
/// `Debugger` domain on, the browser reports every script it compiles in any world of the page,
/// `Runtime.callFunctionOn` declarations included, and only the install bundle is anywhere near
/// its length (the attach and call entries are a few hundred bytes).
struct InstallTap {
    events: broadcast::Receiver<CdpEvent>,
    driver: SessionId,
    /// `dist/install.js` in UTF-16 units, as `Debugger.scriptParsed` counts `length`.
    install_length: u64,
}

impl InstallTap {
    async fn start(tab: &Tab) -> Self {
        let events = tab.cdp.events();
        tab.cdp
            .send("Debugger.enable", json!({}), Some(&tab.driver))
            .await
            .unwrap_or_else(|error| panic!("Debugger.enable: {error}"));
        let crate_dir = PathBuf::from(
            env::var_os("CARGO_MANIFEST_DIR").unwrap_or_else(|| panic!("cargo sets it")),
        );
        let path = crate_dir.join("../neo-runtime-injected/dist/install.js");
        let install = fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
        Self {
            events,
            driver: tab.driver.clone(),
            install_length: u64::try_from(install.encode_utf16().count()).unwrap_or(u64::MAX),
        }
    }

    /// The installs compiled since the last count.
    fn installs(&mut self) -> usize {
        let mut installs = 0;
        loop {
            match self.events.try_recv() {
                Ok(event)
                    if event.method == "Debugger.scriptParsed"
                        && event.session_id.as_ref() == Some(&self.driver)
                        && event.params["length"].as_u64() >= Some(self.install_length) =>
                {
                    installs += 1;
                }
                Ok(_) => {}
                Err(TryRecvError::Empty) => return installs,
                Err(error) => panic!("the event tap: {error}"),
            }
        }
    }
}

/// Three runs on one tab: the first builds Playwright's injected script in the document, the
/// second only attaches to it (no install bundle is compiled), and after a cross-document
/// navigation the third installs again, because the world went with the old document.
#[tokio::test(flavor = "multi_thread")]
async fn the_injected_install_is_reused_per_document_and_redone_after_navigation() {
    let Some(binary) = browser::e2e_browser() else {
        return;
    };
    let fixture = Fixture {
        pages: vec![
            ("/ok.html".to_string(), OK_PAGE.to_string()),
            ("/next.html".to_string(), OK_PAGE.to_string()),
        ],
        ..Fixture::default()
    };
    let tab = Tab::open(binary, fixture, "/ok.html").await;
    let mut tap = InstallTap::start(&tab).await;
    let mut installs = Vec::new();
    for step in ["first run", "second run", "after navigating"] {
        if step == "after navigating" {
            tab.navigate("/next.html").await;
        }
        let outcome = run(&tab, SUBMIT_THEN_URL).await;
        assert_eq!(outcome.status, RunStatus::Ok, "{step}: {outcome:#?}");
        let call = only_record(&outcome);
        assert_eq!(call.delivery, Some(Delivery::Done), "{step}: {call:#?}");
        installs.push((step, tap.installs()));
    }
    tab.close().await;

    assert_eq!(
        installs,
        [("first run", 1), ("second run", 0), ("after navigating", 1)],
        "install bundles compiled per run"
    );
}

/// A missing iframe uses the action's normal timeout and delivers no input.
#[tokio::test(flavor = "multi_thread")]
async fn a_missing_frame_selector_polls_until_the_call_timeout() {
    let Some(tab) = open_page("/ok.html", OK_PAGE).await else {
        return;
    };
    let outcome = run(
        &tab,
        "await page.click('iframe >> internal:control=enter-frame >> button', {timeout:200})",
    )
    .await;
    tab.close().await;
    assert_eq!(outcome.status, RunStatus::Error, "{outcome:#?}");
    assert_eq!(
        outcome.error.as_ref().expect("timeout").name,
        "TimeoutError"
    );
    assert_eq!(only_record(&outcome).delivery, Some(Delivery::NotSent));
}

/// The run's tab in the background: another tab took the foreground, so the page reports itself
/// hidden and, per probe P3, gets no animation frames, which `stable` counts. The click still
/// resolves, because the run's session turns on focus emulation, which makes the tab paint.
#[tokio::test(flavor = "multi_thread")]
async fn a_background_tab_still_resolves_stable() {
    let Some(tab) = open_page("/ok.html", OK_PAGE).await else {
        return;
    };
    tab.cdp
        .send("Target.createTarget", json!({ "url": "about:blank" }), None)
        .await
        .unwrap_or_else(|error| panic!("open a second tab: {error}"));
    let background = json!(["hidden", false]);
    let seen = eventually(
        &tab,
        "[document.visibilityState, document.hasFocus()]",
        |seen| *seen == background,
    )
    .await;
    assert_eq!(seen, background, "the second tab took the foreground");
    let outcome = run(&tab, SUBMIT_THEN_URL).await;
    tab.close().await;

    assert_eq!(outcome.status, RunStatus::Ok, "{outcome:#?}");
    let url = outcome
        .value
        .as_ref()
        .and_then(Value::as_str)
        .unwrap_or_default();
    assert!(url.ends_with("/ok.html#done"), "{url:?}");
    let call = only_record(&outcome);
    assert_eq!(
        (&call.result, call.delivery),
        (&CallResult::Ok, Some(Delivery::Done)),
        "{call:#?}"
    );
}

/// The page is scrolled below the button, so the first scroll centres it under a sticky header
/// that covers the top 55% of the viewport, and the hit check finds the header. The click
/// retries with another scroll alignment and lands once.
#[tokio::test(flavor = "multi_thread")]
async fn tests_that_a_click_scrolls_out_from_under_a_sticky_header() {
    let Some(tab) = open_page("/sticky.html", STICKY_PAGE).await else {
        return;
    };
    tab.evaluate("window.scrollTo(0, 2000)").await;
    let outcome = run(
        &tab,
        "await page.locator('#target').click({ timeout: 5000 })
return 'clicked'",
    )
    .await;
    let clicks = tab.evaluate("window.__clicks").await;
    tab.close().await;

    assert_eq!(outcome.status, RunStatus::Ok, "{outcome:#?}");
    let call = only_record(&outcome);
    assert_eq!(
        (&call.result, call.delivery),
        (&CallResult::Ok, Some(Delivery::Done)),
        "{call:#?}"
    );
    assert!(
        call.log.join("\n").contains("<header>"),
        "the header caught the first attempt: {:#?}",
        call.log
    );
    assert_eq!(clicks, json!(1), "the button's handler ran once");
}

/// A link to a page that answers after 800 ms. `page.url()` read right after the click is the
/// new page's, because the click waits for the navigation it started to commit. With
/// `noWaitAfter` the click returns first, and the URL is still the old page's.
#[tokio::test(flavor = "multi_thread")]
async fn tests_that_a_link_click_waits_for_the_new_page_unless_no_wait_after() {
    for (options, expected) in [("", "/slow"), ("{ noWaitAfter: true }", "/link.html")] {
        let Some(binary) = browser::e2e_browser() else {
            return;
        };
        let fixture = Fixture {
            slow_response: Duration::from_millis(800),
            pages: vec![("/link.html".to_string(), LINK_PAGE.to_string())],
        };
        let tab = Tab::open(binary, fixture, "/link.html").await;
        let code = format!(
            "await page.getByRole('link', {{ name: 'Next' }}).click({options})
return page.url()"
        );
        let outcome = run(&tab, &code).await;
        tab.close().await;

        assert_eq!(outcome.status, RunStatus::Ok, "{options}: {outcome:#?}");
        let url = outcome
            .value
            .as_ref()
            .and_then(Value::as_str)
            .unwrap_or_default();
        assert!(
            url.ends_with(expected),
            "{options}: page.url() right after the click is {url:?}, expected …{expected}"
        );
        let call = only_record(&outcome);
        assert_eq!(call.delivery, Some(Delivery::Done), "{options}: {call:#?}");
    }
}
