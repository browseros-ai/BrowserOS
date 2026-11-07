//! Every request of a page on a real BrowserOS neo (`src/network.rs`, `src/network_events.rs`,
//! neo-sdk `network.ts`): a busy page's requests each reach the run's listeners exactly once, as
//! one object per request; the network log keeps a bounded number of requests, and an object
//! past that bound answers Neo's error naming it; and a run that listened leaves nothing behind
//! for the next run on the same tab: its log, its events and its Network domain die with its
//! session.

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

const PAGE: &str = "<!doctype html><title>network</title><p>network</p>";

/// A script helper that fetches `count` URLs `/data-<prefix>-<i>` in parallel batches of 100,
/// reading every body (Chromium finishes loading a fetch only once its body is consumed).
const BURST: &str = r"const burst = (prefix, count) => page.evaluate(async ({ prefix, count }) => {
  for (let start = 0; start < count; start += 100) {
    const batch = []
    for (let i = start; i < Math.min(count, start + 100); i++)
      batch.push(fetch('/data-' + prefix + '-' + i).then((response) => response.text()))
    await Promise.all(batch)
  }
}, { prefix, count })
";

async fn open() -> Option<Tab> {
    let binary = browser::e2e_browser()?;
    Some(
        Tab::open(
            binary,
            Fixture {
                pages: vec![("/network.html".into(), PAGE.into())],
                ..Fixture::default()
            },
            "/network.html",
        )
        .await,
    )
}

async fn run(tab: &Tab, code: &str) -> RunOutcome {
    tab.start_run_within(format!("{BURST}{code}"), Duration::from_mins(1))
        .await
        .await
        .unwrap_or_else(|error| panic!("run task: {error}"))
}

fn value(outcome: &RunOutcome) -> &Value {
    assert_eq!(outcome.status, RunStatus::Ok, "{outcome:#?}");
    outcome.value.as_ref().unwrap_or(&Value::Null)
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_a_busy_page_reports_every_request_once() {
    let Some(tab) = open().await else {
        return;
    };
    let outcome = run(
        &tab,
        r"const requests = new Map()
const finished = new Map()
const responses = new Map()
page.on('request', (request) => requests.set(request.url(), (requests.get(request.url()) ?? 0) + 1))
page.on('response', (response) => responses.set(response.url(), response.request()))
page.on('requestfinished', (request) => finished.set(request.url(), (finished.get(request.url()) ?? 0) + 1))
await burst('busy', 400)
await page.waitForLoadState('networkidle')
const counts = (map) => [...map.values()]
const recent = await page.requests()
return {
  requested: requests.size,
  finished: finished.size,
  responded: responses.size,
  once: counts(requests).every((n) => n === 1) && counts(finished).every((n) => n === 1),
  sameObjects: recent.every((request) => responses.get(request.url()) === request),
  recent: recent.length,
}",
    )
    .await;
    let logs = outcome.logs.clone();
    tab.close().await;
    assert_eq!(
        value(&outcome),
        &json!({
            "requested": 400,
            "finished": 400,
            "responded": 400,
            "once": true,
            "sameObjects": true,
            // Playwright's window: at 101 the oldest tenth goes, so 400 starts leave 100.
            "recent": 100,
        }),
        "{logs:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_a_request_past_the_retention_cap_answers_neos_error() {
    let Some(tab) = open().await else {
        return;
    };
    let outcome = run(
        &tab,
        r"const [first] = await Promise.all([page.waitForRequest('**/data-first-0'), burst('first', 1)])
const kept = await first.response()
await burst('more', 1100)
const settle = (promise) => promise.then(() => 'resolved', (error) => [error.name, error.message])
return {
  keptStatus: kept.status(),
  url: first.url().replace(/^.*\//, ''),
  response: await settle(first.response()),
  body: await settle(kept.body()),
}",
    )
    .await;
    tab.close().await;
    let value = value(&outcome);
    assert_eq!(value["keptStatus"], 200);
    assert_eq!(
        value["url"], "data-first-0",
        "a getter on the object still answers"
    );
    for what in ["response", "body"] {
        let answer = &value[what];
        assert_eq!(answer[0], "Error", "{what}: {answer}");
        let message = answer[1].as_str().unwrap_or_default();
        assert!(
            message.contains("its last 1000 other requests"),
            "{what}: {message}"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_a_run_that_listened_leaves_nothing_for_the_next_run() {
    let Some(tab) = open().await else {
        return;
    };
    let first = run(
        &tab,
        r"let seen = 0
page.on('request', () => seen++)
await burst('first', 20)
await page.waitForLoadState('networkidle')
// The page keeps fetching after this run: nobody may hear it.
await page.evaluate(() => { window.__timer = setInterval(() => fetch('/data-late').then((r) => r.text()), 20) })
return { seen, recent: (await page.requests()).length }",
    )
    .await;
    assert_eq!(value(&first), &json!({ "seen": 20, "recent": 20 }));
    let second = run(
        &tab,
        r"const before = await page.requests()
await new Promise((resolve) => setTimeout(resolve, 300))
const during = (await page.requests()).map((request) => request.url().replace(/^.*\//, ''))
await page.evaluate(() => clearInterval(window.__timer))
return { before: before.length, onlyLate: during.length > 0 && during.every((url) => url === 'data-late') }",
    )
    .await;
    tab.close().await;
    // The second run starts with an empty log: the first run's requests and its listener went
    // with its session, and the second sees only what happened after its own first look.
    assert_eq!(
        value(&second),
        &json!({ "before": 0, "onlyLate": true }),
        "{second:#?}"
    );
}
