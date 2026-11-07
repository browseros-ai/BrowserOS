//! The run's `Browser` on a real BrowserOS neo (`src/browser_calls.rs`, `src/emulation.rs`):
//! contexts a run creates go with it, also when it times out; its emulation lasts for the run
//! and no longer; a created context's pages are tabs of their own, routed apart from the run's.

#[path = "support/browser.rs"]
#[expect(
    dead_code,
    reason = "shared browser harness also supports navigation and click tests"
)]
mod browser;

use std::time::Duration;

use browser::{Fixture, Tab, e2e_browser};
use neo_runtime::{RunOutcome, RunStatus};
use serde_json::{Value, json};

/// Long enough to create a context and a page on a loaded machine, short enough for a test.
const SHORT_RUN: Duration = Duration::from_secs(8);

async fn created_contexts(tab: &Tab) -> Value {
    tab.cdp
        .send("Target.getBrowserContexts", json!({}), None)
        .await
        .unwrap_or_else(|error| panic!("Target.getBrowserContexts: {error}"))["browserContextIds"]
        .clone()
}

async fn finished(tab: &Tab, code: &str, timeout: Duration) -> RunOutcome {
    tab.start_run_within(code.to_string(), timeout)
        .await
        .await
        .unwrap_or_else(|error| panic!("the run task failed: {error}"))
}

#[tokio::test(flavor = "multi_thread")]
async fn a_run_that_times_out_disposes_the_contexts_it_created() {
    let Some(binary) = e2e_browser() else {
        return;
    };
    let tab = Tab::open(binary, Fixture::default(), "/start").await;
    let outcome = finished(
        &tab,
        "const created = await context.browser().newContext()
         const other = await created.newPage()
         await other.goto(page.url())
         await new Promise(() => {})",
        SHORT_RUN,
    )
    .await;
    assert_eq!(outcome.status, RunStatus::Timeout, "{outcome:?}");
    assert_eq!(created_contexts(&tab).await, json!([]));
    let created: Vec<_> = outcome.pages.iter().filter(|page| page.created).collect();
    assert_eq!(created.len(), 1, "{:?}", outcome.pages);
    assert!(created[0].closed, "the created page reads closed by the run");
    tab.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn emulation_lasts_for_the_run() {
    let Some(binary) = e2e_browser() else {
        return;
    };
    let tab = Tab::open(binary, Fixture::default(), "/start").await;
    let width = tab.evaluate("innerWidth").await;
    let outcome = finished(
        &tab,
        "await page.setViewportSize({ width: 333, height: 222 })
         await page.emulateMedia({ media: 'print', colorScheme: 'dark' })
         return page.viewportSize()",
        SHORT_RUN,
    )
    .await;
    assert_eq!(outcome.status, RunStatus::Ok, "{outcome:?}");
    assert_eq!(outcome.value, Some(json!({ "width": 333, "height": 222 })));
    assert_eq!(tab.evaluate("innerWidth").await, width, "the viewport is the tab's own again");
    assert_eq!(tab.evaluate("matchMedia('print').matches").await, json!(false));
    let next = finished(&tab, "return page.viewportSize()", SHORT_RUN).await;
    assert_eq!(next.status, RunStatus::Ok, "{next:?}");
    assert_eq!(next.value, Some(Value::Null), "a later run starts with no viewport");
    tab.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_created_contexts_pages_are_tabs_of_their_own() {
    let Some(binary) = e2e_browser() else {
        return;
    };
    let tab = Tab::open(binary, Fixture::default(), "/start").await;
    let outcome = finished(
        &tab,
        "const created = await context.browser().newContext()
         const other = await created.newPage()
         await other.goto(page.url().replace('/start', '/other'))
         await page.goto(page.url().replace('/start', '/mine'))
         return {
           titles: [await page.title(), await other.title()],
           text: await other.locator('h1').textContent(),
           own: context.pages().includes(other),
           created: created.pages().map((p) => p === other),
         }",
        SHORT_RUN,
    )
    .await;
    assert_eq!(outcome.status, RunStatus::Ok, "{outcome:?}");
    assert_eq!(
        outcome.value,
        Some(json!({
            "titles": ["/mine", "/other"],
            "text": "/other",
            "own": false,
            "created": [true],
        }))
    );
    assert_eq!(created_contexts(&tab).await, json!([]));
    tab.close().await;
}
