//! The fake clock end to end on a real BrowserOS neo, through neo-sdk's `page.clock`, for Neo's
//! one deviation from Playwright here: the clock lives as long as the run (`Deviation:` in
//! `neo-runtime-injected/src/clock-call.ts`). Once a run that paused the page's time has ended,
//! the page has its own `Date` and timers back, and a later document has no clock.
//! `crates/neo-sdk/tests/parity/clock.test.mjs` compares the in-run behaviour with live
//! Playwright.
//!
//! Runs only when `NEO_E2E_BROWSER` names the browser binary (`support/browser.rs`); otherwise
//! each test passes as a skip.

#[path = "support/browser.rs"]
#[expect(
    dead_code,
    reason = "the shared browser fixture also drives tabs and pages these tests do not"
)]
mod browser;

use std::time::Duration;

use browser::{Fixture, Tab};
use neo_runtime::{RunOutcome, RunStatus};
use serde_json::{Value, json};
use tokio::task::JoinHandle;
use tokio::time;

const PAGE: &str = "<!doctype html><title>Clock</title><p id=p>text</p>";

/// Whether the page's time is its own (a 2020s epoch), its clock global, and a timer that
/// fires, which a paused fake clock never lets happen.
const OWN_TIME: &str = "(async () => [Date.now() > 1600000000000, typeof globalThis.__pwClock, await new Promise((resolve) => setTimeout(() => resolve('fired'), 10))])()";

/// How long a page's own timer may take.
const TIMER: Duration = Duration::from_secs(3);

async fn open() -> Option<Tab> {
    let binary = browser::e2e_browser()?;
    let fixture = Fixture {
        pages: vec![("/clock".to_string(), PAGE.to_string())],
        ..Fixture::default()
    };
    Some(Tab::open(binary, fixture, "/clock").await)
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

/// Evaluates `expression` in the tab's main world, awaiting a promise, outside Neo: `hung` when
/// it has not settled within [`TIMER`].
async fn settle(tab: &Tab, expression: &str) -> Value {
    let evaluated = tab.cdp.send(
        "Runtime.evaluate",
        json!({ "expression": expression, "returnByValue": true, "awaitPromise": true }),
        Some(&tab.driver),
    );
    let Ok(reply) = time::timeout(TIMER, evaluated).await else {
        return json!("hung");
    };
    let reply = reply.unwrap_or_else(|error| panic!("{expression}: {error}"));
    assert!(
        reply.get("exceptionDetails").is_none(),
        "{expression} threw: {reply}"
    );
    reply["result"]["value"].clone()
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_a_runs_clock_ends_with_the_run() {
    let Some(tab) = open().await else { return };
    let during = value(
        &finish(
            tab.start_run(
                r"await page.clock.install({ time: 0 })
                  await page.clock.pauseAt(1000)
                  return await page.evaluate(() => [Date.now(), typeof globalThis.__pwClock])"
                    .to_string(),
            )
            .await,
        )
        .await,
    );
    assert_eq!(
        during,
        json!([1000, "object"]),
        "the run's clock is paused at 1000"
    );

    assert_eq!(
        settle(&tab, OWN_TIME).await,
        json!([true, "undefined", "fired"]),
        "after the run, the page has its own time and timers"
    );
    tab.navigate("/clock").await;
    assert_eq!(
        settle(&tab, OWN_TIME).await,
        json!([true, "undefined", "fired"]),
        "a later document has no clock"
    );
    tab.close().await;
}
