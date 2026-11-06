//! Clicks end to end on a real BrowserOS neo, against Playwright's recorded goldens
//! (`crates/neo-sdk/tests/goldens/click-goldens.json`, captured with playwright-core 1.64.1 on
//! the same browser). Each case serves the golden's own fixture page, runs the golden's own
//! script as a Neo run, and compares what the script saw, the run's record, and what the page's
//! listeners saw.
//!
//! The goldens are read at run time through `CARGO_MANIFEST_DIR`, never `env!`, so a test
//! binary from a shared build dir reads this checkout's files. Runs only when `NEO_E2E_BROWSER`
//! names the browser binary (`support/browser.rs`); otherwise each test passes as a skip.

#[path = "support/browser.rs"]
#[expect(
    dead_code,
    reason = "this file opens tabs at a path and never navigates them"
)]
mod browser;

use std::path::PathBuf;
use std::time::Duration;
use std::{env, fs};

use browser::{Fixture, Tab, e2e_browser};
use neo_runtime::{CallRecord, CallResult, Delivery, RunOutcome, RunStatus};
use serde::Deserialize;
use serde_json::{Value, json};

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
        let binary = e2e_browser()?;
        let fixture = Fixture {
            pages: vec![(self.path(), self.fixture_html.clone())],
            ..Fixture::default()
        };
        Some(Tab::open(binary, fixture, &self.path()).await)
    }
}

async fn run(tab: &Tab, code: &str) -> RunOutcome {
    tab.start_run(code.to_string())
        .await
        .await
        .unwrap_or_else(|error| panic!("the run's task: {error}"))
}

fn only_record(outcome: &RunOutcome) -> &CallRecord {
    let [call] = outcome.calls.as_slice() else {
        panic!("one browser call: {outcome:#?}");
    };
    call
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
    let Some(binary) = e2e_browser() else {
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
