//! A request a run's route left paused, on a real BrowserOS neo (`src/browser_representations/routes.rs`): a handler
//! that never answers holds its request while the run lasts; the run's end lets it go on, so
//! the page gets its response; and the next run on the tab finds no interception left, its
//! loads and fetches reaching the network untouched. An awaited `context.close()` lets go of
//! the same request before it is acknowledged, while the run lives on, and nothing pauses the
//! page's requests after it (`Tabs::release_run_scoped`).

#[expect(
    dead_code,
    reason = "the shared fixture also provides navigation helpers"
)]
#[path = "support/browser.rs"]
mod browser;

use std::time::Duration;

use browser::{Fixture, Tab};
use neo_runtime::{RunOutcome, RunStatus};
use serde_json::{Value, json};
use tokio::task::JoinHandle;
use tokio::time::{self, Instant};
use tokio_util::sync::CancellationToken;

const PAGE: &str = "<!doctype html><title>routes</title><p>routes</p>";

async fn open() -> Option<Tab> {
    let binary = browser::e2e_browser()?;
    Some(
        Tab::open(
            binary,
            Fixture {
                pages: vec![("/routes.html".into(), PAGE.into())],
                ..Fixture::default()
            },
            "/routes.html",
        )
        .await,
    )
}

async fn run(tab: &Tab, code: &str) -> RunOutcome {
    tab.start_run_within(code.to_string(), Duration::from_mins(1))
        .await
        .await
        .unwrap_or_else(|error| panic!("run task: {error}"))
}

fn value(outcome: &RunOutcome) -> &Value {
    assert_eq!(outcome.status, RunStatus::Ok, "{outcome:#?}");
    outcome.value.as_ref().unwrap_or(&Value::Null)
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_a_request_a_route_left_paused_goes_on_when_the_run_ends() {
    let Some(tab) = open().await else {
        return;
    };
    let first = run(
        &tab,
        r"let entered
const inside = new Promise((resolve) => (entered = resolve))
// The handler never answers: the request stays paused for the rest of the run.
await page.route('**/held', () => { entered() })
await page.evaluate(() => { window.__held = fetch('/held').then((response) => response.text()) })
await inside
return 'paused'",
    )
    .await;
    assert_eq!(value(&first), &json!("paused"));
    // A page whose request were still paused would hold this read until the run's timeout.
    let second = run(
        &tab,
        r"const held = await page.evaluate(() => window.__held)
const response = await page.goto(page.url().replace(/[^/]*$/, 'next.html'))
const fetched = await page.evaluate(() => fetch('/after').then((response) => response.text()))
return { held, ok: response.ok(), title: await page.title(), fetched }",
    )
    .await;
    tab.close().await;
    assert_eq!(
        value(&second),
        &json!({
            "held": "<title>/held</title><h1>/held</h1>",
            "ok": true,
            "title": "/next.html",
            "fetched": "<title>/after</title><h1>/after</h1>",
        }),
        "the paused request went on at the run's end, and nothing pauses the next run's: {second:#?}"
    );
}

/// Reads `expression` in the page until it is not `null`, while `run` is still live; `None`
/// once the run ended first or after `within`.
async fn read_while_live(
    tab: &Tab,
    run: &JoinHandle<RunOutcome>,
    expression: &str,
    within: Duration,
) -> Option<Value> {
    let deadline = Instant::now() + within;
    while Instant::now() < deadline && !run.is_finished() {
        let value = tab.evaluate(expression).await;
        if !value.is_null() {
            return (!run.is_finished()).then_some(value);
        }
        time::sleep(Duration::from_millis(50)).await;
    }
    None
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_closing_the_context_lets_a_held_request_go_on_while_the_run_lives() {
    let Some(tab) = open().await else {
        return;
    };
    let cancel = CancellationToken::new();
    // The script never ends on its own: only the close can let the request go before the test
    // cancels the run.
    let run = tab
        .start_run_cancellable(
            r"let entered
const inside = new Promise((resolve) => (entered = resolve))
await page.route('**/held', () => {
  entered()
  return new Promise(() => {})
})
await page.evaluate(() => {
  window.__held = fetch('/held').then((response) => response.text()).then((text) => { window.__released = text })
})
await inside
await context.close()
await new Promise(() => {})"
                .to_string(),
            Duration::from_mins(1),
            cancel.clone(),
        )
        .await;
    let released = read_while_live(
        &tab,
        &run,
        "window.__released ?? null",
        Duration::from_secs(20),
    )
    .await;
    // A request the page starts after the close reaches the network untouched.
    let _started = tab
        .evaluate(
            "void fetch('/after').then((response) => response.text()).then((text) => { window.__after = text })",
        )
        .await;
    let after = read_while_live(
        &tab,
        &run,
        "window.__after ?? null",
        Duration::from_secs(20),
    )
    .await;
    cancel.cancel();
    let outcome = run
        .await
        .unwrap_or_else(|error| panic!("run task: {error}"));
    tab.close().await;
    assert_eq!(
        released,
        Some(json!("<title>/held</title><h1>/held</h1>")),
        "the close let the held request go on before the run ended: {outcome:#?}"
    );
    assert_eq!(
        after,
        Some(json!("<title>/after</title><h1>/after</h1>")),
        "nothing paused the page's requests after the close: {outcome:#?}"
    );
    assert_eq!(outcome.status, RunStatus::Cancelled, "{outcome:#?}");
}
