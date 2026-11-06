//! Clicks end to end on a real BrowserOS neo, against Playwright's recorded goldens
//! (`crates/neo-sdk/tests/goldens/click-goldens.json`, captured with playwright-core 1.64.1 on
//! the same browser). Each case serves the golden's own fixture page, runs the golden's own
//! script as a Neo run, and compares what the script saw, the run's record, and what the page's
//! listeners saw.
//!
//! The cases the goldens do not cover (a dialog, a run that ends mid-press, a verified block, a
//! trial, a closed tab, the install's reuse, a frame selector, a background tab) check Neo's
//! own guarantees on the same browser: the run's records and deliveries, and what the page saw.
//! `crates/neo-sdk/tests/parity/click.test.mjs` runs the golden cases, and the dialog, blocked,
//! trial and closed-tab cases on the same pages (`crates/neo-sdk/tests/pages/`), through live
//! Playwright beside Neo.
//!
//! The goldens are read at run time through `CARGO_MANIFEST_DIR`, never `env!`, so a test
//! binary from a shared build dir reads this checkout's files. Runs only when `NEO_E2E_BROWSER`
//! names the browser binary (`support/browser.rs`); otherwise each test passes as a skip.

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
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::broadcast;
use tokio::sync::broadcast::error::TryRecvError;
use tokio::time;

/// One recorded Playwright case.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Golden {
    name: String,
    source: String,
    fixture_html: String,
    #[serde(default)]
    url_relative: Option<String>,
    #[serde(default)]
    error: Option<GoldenError>,
    page: GoldenPage,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GoldenError {
    name: String,
    first_line: String,
    message: String,
}

#[derive(Deserialize)]
struct GoldenPage {
    /// Every pointer and mouse event the page's own recorder saw.
    events: Vec<Value>,
}

#[derive(Deserialize)]
struct Goldens {
    cases: Vec<Golden>,
}

impl Golden {
    fn load(case: &str) -> Golden {
        let crate_dir = PathBuf::from(
            env::var_os("CARGO_MANIFEST_DIR").unwrap_or_else(|| panic!("cargo sets it")),
        );
        let path = crate_dir.join("../neo-sdk/tests/goldens/click-goldens.json");
        let text = fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
        let goldens: Goldens = serde_json::from_str(&text)
            .unwrap_or_else(|error| panic!("click-goldens.json parses: {error}"));
        goldens
            .cases
            .into_iter()
            .find(|golden| golden.name == case)
            .unwrap_or_else(|| panic!("no click golden named {case}"))
    }

    /// The path the golden's page is served at, as it was when it was recorded.
    fn path(&self) -> String {
        format!("/{}.html", self.name)
    }

    /// Opens a fresh browser on the golden's fixture page.
    async fn open(&self) -> Option<Tab> {
        let binary = browser::e2e_browser()?;
        let fixture = Fixture {
            pages: vec![(self.path(), self.fixture_html.clone())],
            ..Fixture::default()
        };
        Some(Tab::open(binary, fixture, &self.path()).await)
    }

    fn expected_error(&self) -> &GoldenError {
        self.error
            .as_ref()
            .unwrap_or_else(|| panic!("the {} golden records an error", self.name))
    }
}

/// A button whose `onclick` opens a `confirm()` and keeps its answer in `window.__answer`.
const DIALOG_PAGE: &str = include_str!("../../neo-sdk/tests/pages/dialog.html");
/// A button whose overlay appears on the click's own `mousemove` and goes after the
/// `pointerdown` it caught, so exactly one press is blocked after the hit check passed. The page
/// counts the button's own clicks in `window.__clicks`.
const BLOCKED_ONCE_PAGE: &str = include_str!("../../neo-sdk/tests/pages/blocked-once.html");

/// Opens a fresh browser on one page that is not a golden's.
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

/// The lines after `Call log:` in a Playwright error message (`formatCallLog`).
fn call_log(message: &str) -> Vec<&str> {
    message
        .split_once("\nCall log:\n")
        .map_or_else(Vec::new, |(_, log)| log.lines().collect())
}

/// A call-log line split into its fold count and its text, without nesting: `    2 × waiting
/// for …` is `(2, "waiting for …")` and `  - waiting for …` is `(1, "waiting for …")`. The text
/// is timing-free; how often and how deep a retry line repeats depends on how many attempts fit
/// in the timeout.
fn unfold(line: &str) -> (usize, &str) {
    let line = line.trim_start();
    if let Some(line) = line.strip_prefix("- ") {
        return (1, line);
    }
    match line.split_once(" × ") {
        Some((count, text)) => (count.parse().unwrap_or(0), text),
        None => (1, line),
    }
}

/// Whether a call-log line's text is one of `_retryAction`'s waits between attempts
/// (`waiting 20ms`), not a `waiting for …` line.
fn is_retry_wait(text: &str) -> bool {
    text.strip_prefix("waiting ")
        .and_then(|rest| rest.strip_suffix("ms"))
        .is_some_and(|ms| !ms.is_empty() && ms.bytes().all(|byte| byte.is_ascii_digit()))
}

/// The distinct call-log lines other than the retry waits, without nesting or fold counts.
fn key_lines(message: &str) -> BTreeSet<&str> {
    call_log(message)
        .into_iter()
        .map(|line| unfold(line).1)
        .filter(|text| !is_retry_wait(text))
        .collect()
}

/// The retry waits (`waiting 20ms`, `waiting 100ms`, …) in the order the folded log prints them.
fn retry_waits(message: &str) -> Vec<&str> {
    call_log(message)
        .into_iter()
        .map(|line| unfold(line).1)
        .filter(|text| is_retry_wait(text))
        .collect()
}

/// How many times `text` was logged, counting a folded `N × text` as N.
fn occurrences(log: &[String], text: &str) -> usize {
    log.iter()
        .map(|line| unfold(line))
        .filter(|(_, logged)| *logged == text)
        .map(|(count, _)| count)
        .sum()
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

/// The `type` of each event the golden fixtures' recorder saw.
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

/// `ok`: resolves, and `page.url()` read right after the click already ends `#done` (the
/// fragment navigation the click's handler made is visible when `click()` resolves, as under
/// Playwright). One record, title `Click`, delivery `Done`, and the page's listeners saw exactly
/// the events Playwright's click produced, at Playwright's point (36.75, 18.5).
#[tokio::test(flavor = "multi_thread")]
async fn ok_clicks_and_the_hash_is_visible_to_page_url() {
    let golden = Golden::load("ok");
    let Some(tab) = golden.open().await else {
        return;
    };
    let outcome = run(&tab, &golden.source).await;
    let events = tab.evaluate("window.__events").await;
    tab.close().await;

    assert_eq!(outcome.status, RunStatus::Ok, "{outcome:#?}");
    let url_relative = golden.url_relative.as_deref().expect("ok records its url");
    let url = outcome
        .value
        .as_ref()
        .and_then(Value::as_str)
        .unwrap_or_default();
    assert!(
        url.ends_with(url_relative),
        "page.url() right after the click is {url:?}, Playwright's ended {url_relative:?}"
    );
    let call = only_record(&outcome);
    assert_eq!(
        (call.title.as_deref(), &call.result, call.delivery),
        (Some("Click"), &CallResult::Ok, Some(Delivery::Done)),
        "{call:#?}"
    );
    assert_eq!(
        call.log.first().map(String::as_str),
        Some("  - waiting for getByRole('button', { name: 'Submit' })")
    );
    assert_eq!(
        call.log.last().map(String::as_str),
        Some("    - navigations have finished")
    );
    assert_eq!(events, Value::Array(golden.page.events.clone()));
}

/// `strict`: `Error`, the whole message (call log included) byte-identical to Playwright's,
/// well under the timeout, nothing delivered, and a log of exactly the `waiting for` line.
#[tokio::test(flavor = "multi_thread")]
async fn strict_mode_fails_at_once_with_playwrights_message() {
    let golden = Golden::load("strict");
    let Some(tab) = golden.open().await else {
        return;
    };
    let outcome = run(&tab, &golden.source).await;
    let events = tab.evaluate("window.__events").await;
    tab.close().await;

    let expected = golden.error.as_ref().expect("strict records its error");
    assert_eq!(outcome.status, RunStatus::Error, "{outcome:#?}");
    let error = outcome.error.as_ref().expect("a failed run says why");
    assert_eq!(error.name, expected.name);
    assert_eq!(error.message, expected.message);
    assert_eq!(
        error.message.lines().next(),
        Some(expected.first_line.as_str())
    );
    let call = only_record(&outcome);
    assert_eq!(call.delivery, Some(Delivery::NotSent));
    assert_eq!(
        call.log,
        ["  - waiting for getByRole('button', { name: 'Submit' })"]
    );
    assert!(
        call.duration < Duration::from_secs(2),
        "a strict violation fails at once, took {:?}",
        call.duration
    );
    assert_eq!(events, json!([]), "no input reached the page");
}

/// `hidden`: `TimeoutError` after 500 ms, the whole message (call log included) byte-identical
/// to Playwright's. `getByRole` never matches a `display:none` button, so the log is only the
/// `waiting for` line, with no "not visible" line, as in Playwright's.
#[tokio::test(flavor = "multi_thread")]
async fn hidden_button_times_out_with_only_the_waiting_line() {
    let golden = Golden::load("hidden");
    let Some(tab) = golden.open().await else {
        return;
    };
    let outcome = run(&tab, &golden.source).await;
    let events = tab.evaluate("window.__events").await;
    tab.close().await;

    let expected = golden.expected_error();
    assert_eq!(outcome.status, RunStatus::Error, "{outcome:#?}");
    let error = outcome.error.as_ref().expect("a failed run says why");
    assert_eq!(
        (error.name.as_str(), error.message.as_str()),
        (expected.name.as_str(), expected.message.as_str())
    );
    let call = only_record(&outcome);
    assert_eq!(
        (&call.result, call.delivery),
        (
            &CallResult::Error {
                name: "TimeoutError".to_string(),
                message: "Timeout 500ms exceeded.".to_string(),
            },
            Some(Delivery::NotSent)
        ),
        "{call:#?}"
    );
    assert!(
        call.duration >= Duration::from_millis(500) && call.duration < Duration::from_secs(2),
        "the call ran out its 500 ms, took {:?}",
        call.duration
    );
    assert_eq!(events, json!([]), "no input reached the page");
}

/// `covered` and `disabled`: `TimeoutError`, the first line exact, the same distinct call-log
/// lines as Playwright's (the overlay named as the element that intercepts, the `not enabled`
/// state), and retry waits that open Playwright's backoff. How many attempts fit in the timeout
/// depends on the machine's load: a loaded machine fits fewer, and the folded log then stops
/// before `waiting 500ms`. `click.rs`'s `FakeTab` tests pin the idle-browser log byte for byte
/// under paused time. No input reached the page: delivery `NotSent`.
#[tokio::test(flavor = "multi_thread")]
async fn covered_and_disabled_time_out_with_playwrights_key_lines() {
    for case in ["covered", "disabled"] {
        let golden = Golden::load(case);
        let Some(tab) = golden.open().await else {
            return;
        };
        let outcome = run(&tab, &golden.source).await;
        let events = tab.evaluate("window.__events").await;
        tab.close().await;

        let expected = golden.expected_error();
        assert_eq!(outcome.status, RunStatus::Error, "{case}: {outcome:#?}");
        let error = outcome.error.as_ref().expect("a failed run says why");
        assert_eq!(error.name, expected.name, "{case}");
        assert_eq!(
            error.message.lines().next(),
            Some(expected.first_line.as_str()),
            "{case}"
        );
        let neo = key_lines(&error.message);
        let playwright = key_lines(&expected.message);
        assert_eq!(
            neo, playwright,
            "{case}: the distinct call-log lines but the retry waits; Neo's message:\n{}",
            error.message
        );
        let neo_waits = retry_waits(&error.message);
        let playwright_waits = retry_waits(&expected.message);
        assert!(
            !neo_waits.is_empty() && playwright_waits.starts_with(&neo_waits),
            "{case}: Neo's retry waits {neo_waits:?} must open Playwright's backoff \
             {playwright_waits:?}. Only the attempt count may vary with load: `cargo test -p \
             neo-runtime --lib click::` holds the idle-browser log byte for byte under paused \
             time. Neo's message:\n{}",
            error.message
        );
        let call = only_record(&outcome);
        assert_eq!(call.delivery, Some(Delivery::NotSent), "{case}: {call:#?}");
        assert_eq!(events, json!([]), "{case}: no input reached the page");
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
        covered.contains("<div id=\"overlay\"></div> intercepts pointer events")
            && covered.contains("retrying click action"),
        "the overlay held loop B past its first retries:\n{covered}"
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
    let golden = Golden::load("ok");
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
        let Some(tab) = golden.open().await else {
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
/// blocks it and names the overlay, delivery goes back to `NotSent`, and loop B presses again
/// on the same element, exactly where Playwright would. The second press lands: the button's
/// own handler ran once, delivery `Done`.
#[tokio::test(flavor = "multi_thread")]
async fn a_verified_block_after_input_presses_again() {
    let Some(tab) = open_page("/blocked-once.html", BLOCKED_ONCE_PAGE).await else {
        return;
    };
    let outcome = run(
        &tab,
        "await page.getByRole('button', { name: 'Submit' }).click()
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
    let log = &call.log;
    assert_eq!(
        occurrences(log, "performing click action"),
        2,
        "pressed twice, blocked then landed: {log:#?}"
    );
    assert_eq!(
        occurrences(log, "<div id=\"overlay\"></div> intercepts pointer events"),
        1,
        "the verdict named the overlay: {log:#?}"
    );
    assert_eq!(occurrences(log, "retrying click action"), 1, "{log:#?}");
    assert_eq!(
        clicks,
        json!(1),
        "the button's handler never saw the blocked press"
    );
}

/// `trial: true` on the `ok` fixture: Playwright's checks, then a press whose every event the
/// interceptor blocks. The click resolves, the button's own `onclick` never runs (the hash
/// stays empty), and delivery is `NotSent`.
#[tokio::test(flavor = "multi_thread")]
async fn a_trial_click_delivers_nothing() {
    let golden = Golden::load("ok");
    let Some(tab) = golden.open().await else {
        return;
    };
    let outcome = run(
        &tab,
        "await page.getByRole('button', { name: 'Submit' }).click({ trial: true })
return page.url()",
    )
    .await;
    let hash = tab.evaluate("location.hash").await;
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
    for line in [
        "attempting click action (trial run)",
        "trial click action done",
    ] {
        assert_eq!(occurrences(&call.log, line), 1, "{line}: {:#?}", call.log);
    }
    assert_eq!(hash, json!(""), "the button's onclick never ran");
}

/// The test closes the tab while a click polls for a button that never matches. The script
/// catches what Playwright's client throws for it: `e.name` is `Error` (Playwright's
/// `TargetClosedError` sets no name) and the message is `locator.click: Target page, context or
/// browser has been closed` with the call log, long before the click's 20 s timeout. The run's
/// watcher learns of the close from `Target.detachedFromTarget`.
#[tokio::test(flavor = "multi_thread")]
async fn a_tab_closed_mid_click_fails_with_target_closed() {
    let golden = Golden::load("hidden");
    let Some(tab) = golden.open().await else {
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
    assert_eq!(
        outcome.value,
        Some(json!({
            "name": "Error",
            "message": "locator.click: Target page, context or browser has been closed
Call log:
  - waiting for getByRole('button', { name: 'Submit' })
",
        }))
    );
    let call = only_record(&outcome);
    assert_eq!(
        (&call.result, call.delivery),
        (
            &CallResult::Error {
                name: "TargetClosedError".to_string(),
                message: "Target page, context or browser has been closed".to_string(),
            },
            Some(Delivery::NotSent)
        ),
        "{call:#?}"
    );
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
    let golden = Golden::load("ok");
    let Some(binary) = browser::e2e_browser() else {
        return;
    };
    let fixture = Fixture {
        pages: vec![
            (golden.path(), golden.fixture_html.clone()),
            ("/next.html".to_string(), golden.fixture_html.clone()),
        ],
        ..Fixture::default()
    };
    let tab = Tab::open(binary, fixture, &golden.path()).await;
    let mut tap = InstallTap::start(&tab).await;
    let mut installs = Vec::new();
    for step in ["first run", "second run", "after navigating"] {
        if step == "after navigating" {
            tab.navigate("/next.html").await;
        }
        let outcome = run(&tab, &golden.source).await;
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

/// `page.click` with a selector that enters an iframe: refused before any page call with Neo's
/// frames message, no call log, nothing delivered.
#[tokio::test(flavor = "multi_thread")]
async fn a_frame_selector_is_refused_up_front() {
    let golden = Golden::load("ok");
    let Some(tab) = golden.open().await else {
        return;
    };
    let outcome = run(
        &tab,
        "await page.click('iframe >> internal:control=enter-frame >> button')",
    )
    .await;
    tab.close().await;

    let refusal = "Neo does not support frames yet: the selector enters a frame";
    assert_eq!(outcome.status, RunStatus::Error, "{outcome:#?}");
    let error = outcome.error.as_ref().expect("a failed run says why");
    assert_eq!(
        (error.name.as_str(), error.message.as_str()),
        ("Error", format!("page.click: {refusal}").as_str())
    );
    let call = only_record(&outcome);
    assert_eq!(
        (&call.result, call.delivery, call.log.as_slice()),
        (
            &CallResult::Error {
                name: "Error".to_string(),
                message: refusal.to_string(),
            },
            Some(Delivery::NotSent),
            [].as_slice()
        ),
        "{call:#?}"
    );
}

/// The run's tab in the background: another tab took the foreground, so the page reports itself
/// hidden and, per probe P3, gets no animation frames, which `stable` counts. The click still
/// resolves, because the run's session turns on focus emulation, which makes the tab paint.
#[tokio::test(flavor = "multi_thread")]
async fn a_background_tab_still_resolves_stable() {
    let golden = Golden::load("ok");
    let Some(tab) = golden.open().await else {
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
    let outcome = run(&tab, &golden.source).await;
    tab.close().await;

    assert_eq!(outcome.status, RunStatus::Ok, "{outcome:#?}");
    let url_relative = golden.url_relative.as_deref().expect("ok records its url");
    let url = outcome
        .value
        .as_ref()
        .and_then(Value::as_str)
        .unwrap_or_default();
    assert!(url.ends_with(url_relative), "{url:?}");
    let call = only_record(&outcome);
    assert_eq!(
        (&call.result, call.delivery),
        (&CallResult::Ok, Some(Delivery::Done)),
        "{call:#?}"
    );
}
