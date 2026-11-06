//! `Locator.count()` and `Locator.allTextContents()` end to end on a real BrowserOS neo, through
//! neo-sdk, on `neo-sdk/tests/pages/element-lists.html`: one run reads five matches (an empty and
//! a hidden one among them) and a selector that matches nothing, and gets Playwright's exact
//! numbers and texts, with no strict mode violation; a selector that enters a frame is refused.
//! `crates/neo-sdk/tests/parity/count.test.mjs` and `allTextContents.test.mjs` run the same reads
//! through live Playwright beside Neo.
//!
//! Runs only when `NEO_E2E_BROWSER` names the browser binary (`support/browser.rs`); otherwise
//! each test passes as a skip.

#[expect(
    dead_code,
    reason = "shared fixture includes navigation helpers other suites use"
)]
#[path = "support/browser.rs"]
mod browser;

use browser::{Fixture, Tab};
use neo_runtime::{CallResult, RunOutcome, RunStatus};
use serde_json::{Value, json};

const LISTS: &str = include_str!("../../neo-sdk/tests/pages/element-lists.html");

async fn open() -> Option<Tab> {
    let binary = browser::e2e_browser()?;
    let fixture = Fixture {
        pages: vec![("/lists.html".to_string(), LISTS.to_string())],
        ..Fixture::default()
    };
    Some(Tab::open(binary, fixture, "/lists.html").await)
}

async fn run(tab: &Tab, code: &str) -> RunOutcome {
    tab.start_run(code.to_string())
        .await
        .await
        .unwrap_or_else(|error| panic!("the run's task: {error}"))
}

/// The script's return value, after checking the run itself went well.
fn value(outcome: &RunOutcome) -> Value {
    assert_eq!(outcome.status, RunStatus::Ok, "{outcome:?}");
    outcome
        .value
        .clone()
        .unwrap_or_else(|| panic!("the script returns a value: {outcome:?}"))
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_count_and_all_text_contents_read_every_match_and_none() {
    let Some(tab) = open().await else { return };
    let outcome = run(
        &tab,
        "return {
           count: await page.locator('.fruit').count(),
           texts: await page.locator('.fruit').allTextContents(),
           missingCount: await page.locator('.missing').count(),
           missingTexts: await page.locator('.missing').allTextContents(),
         };",
    )
    .await;
    let dom = tab
        .evaluate("[...document.querySelectorAll('.fruit')].map(e => e.textContent)")
        .await;
    tab.close().await;
    let result = value(&outcome);
    assert_eq!(
        result,
        json!({
            "count": 5,
            "texts": ["Apple", "  Banana\n    split  ", "", "Cherry", "Date"],
            "missingCount": 0,
            "missingTexts": [],
        }),
        "every match in document order, untouched, and none is 0 and []"
    );
    assert_eq!(result["texts"], dom, "the DOM's own textContent agrees");
    let methods: Vec<&str> = outcome
        .calls
        .iter()
        .map(|call| call.method.as_str())
        .collect();
    assert_eq!(
        methods,
        [
            "queryCount",
            "evalOnSelectorAll",
            "queryCount",
            "evalOnSelectorAll"
        ],
        "Playwright's own commands"
    );
    assert!(
        outcome
            .calls
            .iter()
            .all(|call| call.result == CallResult::Ok && call.delivery.is_none()),
        "reads that answer and write no input: {:#?}",
        outcome.calls
    );
}

/// The iframe holds a `.fruit`, so a 0 or [] here would be a wrong answer, not a refusal.
#[tokio::test(flavor = "multi_thread")]
async fn tests_that_a_selector_entering_a_frame_is_refused() {
    let Some(tab) = open().await else { return };
    tab.evaluate(
        "document.body.insertAdjacentHTML('beforeend', \
         '<iframe srcdoc=\"<ul><li class=fruit>Inner</li></ul>\"></iframe>')",
    )
    .await;
    let outcome = run(
        &tab,
        "const caught = (e) => ({ name: e.name, frames: /does not support frames/.test(e.message) });
         const inner = page.locator('iframe >> internal:control=enter-frame >> .fruit');
         return {
           count: await inner.count().catch(caught),
           texts: await inner.allTextContents().catch(caught),
         };",
    )
    .await;
    tab.close().await;
    let refused = json!({ "name": "Error", "frames": true });
    assert_eq!(
        value(&outcome),
        json!({ "count": refused, "texts": refused })
    );
}
