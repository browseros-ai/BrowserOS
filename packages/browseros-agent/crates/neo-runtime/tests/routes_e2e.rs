//! A request a run's route left paused, on a real BrowserOS neo (`src/routes.rs`): a handler
//! that never answers holds its request while the run lasts; the run's end lets it go on, so
//! the page gets its response; and the next run on the tab finds no interception left, its
//! loads and fetches reaching the network untouched.

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
