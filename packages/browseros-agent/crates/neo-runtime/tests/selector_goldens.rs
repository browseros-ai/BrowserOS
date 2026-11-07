//! Playwright's selectors, built inside QuickJS by the committed `neo-sdk/dist/sdk.js`. Bun
//! cross-checked the base goldens against playwright-core 1.64.1; the frame goldens come from
//! the real pinned 26137face build. This checks the engine that
//! actually runs agent scripts, where `RegExp` flags and `source` escaping could differ from
//! V8's. Building a locator does no I/O, so the fake browser in `support/fake_browser.rs` only
//! has to let the run start, and the test checks that nothing else reached it.

#[path = "support/fake_browser.rs"]
#[expect(
    dead_code,
    reason = "the page's interceptor and click readers serve click_lifecycle.rs"
)]
mod fake_browser;

use std::env;
use std::error::Error;
use std::fs;
use std::path::PathBuf;
use std::time::Duration;

use fake_browser::{Exchange, FakeBrowser, Replies};
use neo_runtime::{RunOutcome, RunStatus};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

/// One recorded chain: the JS an agent writes, starting `page.`, and Playwright's selector.
#[derive(Deserialize)]
struct Golden {
    chain: String,
    selector: String,
    #[serde(default)]
    kind: GoldenKind,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
enum GoldenKind {
    #[default]
    Locator,
    FrameLocator,
}

/// Read at run time through `CARGO_MANIFEST_DIR`, not `env!`, so a test binary from a shared
/// build dir reads this checkout's goldens.
fn goldens() -> Result<Vec<Golden>, Box<dyn Error>> {
    let crate_dir = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").ok_or("cargo sets it")?);
    let directory = crate_dir.join("../neo-sdk/tests/goldens");
    let mut goldens: Vec<Golden> = serde_json::from_str(&fs::read_to_string(
        directory.join("selector-goldens.json"),
    )?)?;
    goldens.extend(serde_json::from_str::<Vec<Golden>>(&fs::read_to_string(
        directory.join("frame-selector-goldens.json"),
    )?)?);
    Ok(goldens)
}

/// The CDP methods a run sends for its own setup and teardown, whatever its script does.
const RUN_SETUP: [&str; 4] = [
    "Browser.getTabInfo",
    "Target.attachToTarget",
    "Page.getNavigationHistory",
    "Target.detachFromTarget",
];

async fn run(code: &str) -> (RunOutcome, Vec<Exchange>) {
    let browser = FakeBrowser::start(Replies::default()).await;
    let outcome = browser
        .runtime()
        .await
        .run(fake_browser::run_request(
            code,
            Duration::from_secs(10),
            CancellationToken::new(),
        ))
        .await;
    (outcome, browser.exchanges())
}

#[tokio::test(flavor = "multi_thread")]
async fn every_selector_golden_matches_inside_quickjs() {
    let goldens = goldens().expect("the selector goldens are committed");
    assert_eq!(
        goldens.len(),
        139,
        "the corpus has 94 base and 45 frame chains"
    );
    // Each chain is built on its own, so one that throws names itself instead of failing the
    // whole run.
    let selectors: Vec<String> = goldens
        .iter()
        .map(|golden| {
            let suffix = match golden.kind {
                GoldenKind::Locator => "._selector",
                GoldenKind::FrameLocator => ".owner()._selector",
            };
            format!(
                "  (() => {{ try {{ return {}{suffix} }} catch (error) {{ return `threw ${{error}}` }} }})(),",
                golden.chain,
            )
        })
        .collect();
    let (outcome, exchanges) = run(&format!("return [\n{}\n]", selectors.join("\n"))).await;

    assert_eq!(outcome.status, RunStatus::Ok, "{:?}", outcome.error);
    assert!(
        exchanges
            .iter()
            .all(|exchange| RUN_SETUP.contains(&exchange.method())),
        "building locators sends nothing to the browser: {exchanges:?}"
    );
    let Some(Value::Array(built)) = outcome.value else {
        panic!("the script returns one selector per chain: {outcome:?}");
    };
    assert_eq!(built.len(), goldens.len(), "{built:?}");
    let mismatches: Vec<String> = goldens
        .iter()
        .zip(&built)
        .filter(|(golden, built)| built.as_str() != Some(golden.selector.as_str()))
        .map(|(golden, built)| {
            format!(
                "{}\n  playwright: {:?}\n  neo:        {built}",
                golden.chain, golden.selector
            )
        })
        .collect();
    assert!(
        mismatches.is_empty(),
        "{} of {} selectors differ:\n{}",
        mismatches.len(),
        goldens.len(),
        mismatches.join("\n")
    );
}

/// `has`, `and`, `or` and `locator(Locator)` work only because `page.mainFrame()` is one object:
/// a locator from any other `Frame` object is refused, as in Playwright.
#[tokio::test(flavor = "multi_thread")]
async fn frame_identity_and_the_test_id_attribute_hold() {
    let code = r"
const frame = page.mainFrame()
const row = page.getByRole('row').filter({ has: page.getByText('Alice') })
const OtherFrame = frame.constructor
const other = new OtherFrame('frame@other', page)
let refused
try {
  page.locator('li', { has: other.locator('b') })
} catch (error) {
  refused = error.message
}
return {
  oneFrame: frame === page.mainFrame() && row._frame === frame && frame.page() === page,
  pageOfLocator: row.page() === page,
  row: row._selector,
  testId: page.getByTestId('save')._selector,
  refused,
}";
    let (outcome, _) = run(code).await;

    assert_eq!(outcome.status, RunStatus::Ok, "{:?}", outcome.error);
    assert_eq!(
        outcome.value,
        Some(json!({
            "oneFrame": true,
            "pageOfLocator": true,
            "row": "internal:role=row >> internal:has=\"internal:text=\\\"Alice\\\"i\"",
            "testId": "internal:testid=[data-testid=\"save\"s]",
            "refused": "Inner \"has\" locator must belong to the same frame.",
        })),
        "{outcome:?}"
    );
}
