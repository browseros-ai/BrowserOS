//! Exposed bindings end to end on a real BrowserOS neo, through neo-quickjs-injected's
//! `page.exposeFunction`, for Neo's one deviation from Playwright here: a binding lives as long as
//! the run that exposed it (`Deviation:` in `src/browser_representations/bindings.rs`). Once the run has ended, the page's
//! call of a binding, through a reference the page kept too, rejects and never hangs; a call still
//! waiting when the run ended rejects; the page keeps none of the run's globals; and a later
//! document has no binding. `crates/neo-quickjs-injected/tests/parity/exposeFunction.test.mjs` and
//! `exposeBinding.test.mjs` compare the in-run behaviour with live Playwright.
//!
//! Runs only when `NEO_E2E_BROWSER` names the browser binary (`support/browser.rs`); otherwise
//! each test passes as a skip.

#[path = "support/browser.rs"]
#[expect(
    dead_code,
    reason = "the shared browser fixture also drives tabs and pages these tests do not"
)]
mod browser;

use browser::{Fixture, Tab};
use neo_runtime::{RunOutcome, RunStatus};
use serde_json::{Value, json};
use tokio::task::JoinHandle;

const PAGE: &str = "<!doctype html><title>Bindings</title><p id=p>text</p>";

/// What a page's promise settled to, or `hung` after two seconds.
const SETTLED: &str = r"(promise) => Promise.race([
  Promise.resolve().then(promise).then((value) => 'resolved: ' + value, (error) => 'rejected: ' + error.message),
  new Promise((resolve) => setTimeout(() => resolve('hung'), 2000)),
])";

/// The globals of the run, of Playwright's bindings and of the two names.
const GLOBALS: &str = "Object.getOwnPropertyNames(globalThis).filter((name) => name.startsWith('__playwright') || name === 'ping' || name === 'slow').sort()";

async fn open() -> Option<Tab> {
    let binary = browser::e2e_browser()?;
    let fixture = Fixture {
        pages: vec![("/bindings".to_string(), PAGE.to_string())],
        ..Fixture::default()
    };
    Some(Tab::open(binary, fixture, "/bindings").await)
}

async fn finish(run: JoinHandle<RunOutcome>) -> RunOutcome {
    run.await
        .unwrap_or_else(|error| panic!("the run's task: {error}"))
}

fn value(outcome: &RunOutcome) -> Value {
    assert_eq!(outcome.status, RunStatus::Ok, "{outcome:?}");
    outcome
        .value
        .clone()
        .unwrap_or_else(|| panic!("the script returns a value: {outcome:?}"))
}

/// Evaluates `expression` in the tab's main world, awaiting a promise, outside Neo.
async fn settle(tab: &Tab, expression: &str) -> Value {
    let reply = tab
        .cdp
        .send(
            "Runtime.evaluate",
            json!({ "expression": expression, "returnByValue": true, "awaitPromise": true }),
            Some(&tab.driver),
        )
        .await
        .unwrap_or_else(|error| panic!("{expression}: {error}"));
    assert!(
        reply.get("exceptionDetails").is_none(),
        "{expression} threw: {reply}"
    );
    reply["result"]["value"].clone()
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_a_runs_bindings_end_with_the_run() {
    let Some(tab) = open().await else { return };

    let during = value(
        &finish(
            tab.start_run(format!(
                r"await page.exposeFunction('ping', (x) => 'pong ' + x)
                  await page.exposeFunction('slow', () => new Promise(() => {{}}))
                  return await page.evaluate(async () => {{
                    window.__saved = window.ping
                    window.__pending = window.slow()
                    return [await window.ping('a'), {GLOBALS}]
                  }})"
            ))
            .await,
        )
        .await,
    );
    assert_eq!(
        during,
        json!([
            "pong a",
            [
                "__playwright__binding__",
                "__playwright__binding__controller__",
                "ping",
                "slow"
            ]
        ]),
        "a binding answers during its run, with Playwright's globals"
    );

    assert_eq!(
        settle(&tab, &format!("({SETTLED})(() => window.__pending)")).await,
        json!("rejected: Target page, context or browser has been closed"),
        "a call waiting at run end rejects"
    );
    assert_eq!(
        settle(&tab, &format!("({SETTLED})(() => window.__saved('b'))")).await,
        json!("rejected: binding \"ping\" has been removed"),
        "a kept reference rejects after the run, never hangs"
    );
    assert_eq!(
        settle(&tab, &format!("({SETTLED})(() => window.ping('c'))")).await,
        json!("rejected: window.ping is not a function"),
        "the name is gone"
    );
    assert_eq!(
        settle(&tab, GLOBALS).await,
        json!([]),
        "the page keeps none of the run's globals"
    );

    tab.navigate("/bindings").await;
    assert_eq!(
        settle(&tab, &format!("[typeof window.ping, {GLOBALS}]")).await,
        json!(["undefined", []]),
        "a later document has no binding"
    );
    tab.close().await;
}
