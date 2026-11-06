//! End to end on a real BrowserOS neo, with the test driving the tab over its own CDP session
//! while a Neo run reads the same tab:
//!
//! - `watch-url.js` polls `page.url()` and `page.isClosed()` through a tab's lifetime. The
//!   expected value is what real Playwright returns for the same script
//!   (crates/neo-sdk/tests/parity checks that).
//! - A script polls `page.url()` back to back through ten cross-document navigations, so reads
//!   land in every commit window, where BrowserOS briefly refuses navigation history.
//!
//! Runs only when `NEO_E2E_BROWSER` names the browser binary, for example
//! `/Applications/BrowserOS neo.app/Contents/MacOS/BrowserOS neo`; otherwise it passes as a skip.

#[path = "support/browser.rs"]
mod browser;

use std::iter;
use std::time::Duration;

use browser::{Fixture, Tab, e2e_browser};
use neo_runtime::RunStatus;
use serde_json::json;
use tokio::time;

const WATCH_URL: &str = include_str!("../../neo-sdk/tests/scripts/watch-url.js");
const STEP: Duration = Duration::from_millis(300);
/// Long enough to sample `page.url()` many times while the navigation is in flight.
const SLOW_RESPONSE: Duration = Duration::from_millis(2500);
/// Short enough for `COMMITS` navigations in about three seconds.
const COMMIT_RESPONSE: Duration = Duration::from_millis(200);
const COMMITS: usize = 10;

#[tokio::test(flavor = "multi_thread")]
async fn page_url_and_is_closed_match_playwright_through_a_tab_lifetime() {
    let Some(binary) = e2e_browser() else {
        return;
    };
    let tab = Tab::open(binary, slow(SLOW_RESPONSE), "/a").await;
    let frame_tree = tab
        .cdp
        .send("Page.getFrameTree", json!({}), Some(&tab.driver))
        .await
        .unwrap();
    assert_eq!(
        frame_tree["frameTree"]["frame"]["id"].as_str(),
        Some(tab.target_id.as_str()),
        "`frame@<targetId>` relies on the main frame sharing its page's target id"
    );

    let run = tab.start_run(WATCH_URL.to_string()).await;
    time::sleep(STEP).await;
    tab.evaluate("location.hash = 'done'").await;
    time::sleep(STEP).await;
    tab.evaluate("history.pushState({}, '', '/pushed')").await;
    time::sleep(STEP).await;
    tab.navigate("/slow").await;
    time::sleep(STEP).await;
    tab.cdp
        .send(
            "Target.closeTarget",
            json!({ "targetId": tab.target_id }),
            None,
        )
        .await
        .unwrap();

    let outcome = run.await.unwrap();
    tab.close().await;

    assert_eq!(outcome.status, RunStatus::Ok, "{outcome:?}");
    assert_eq!(
        outcome.value,
        Some(json!({
            "paths": ["/a", "/a#done", "/pushed", "/slow"],
            "closed": true,
            "lastPath": "/slow",
        })),
        "Playwright keeps `/pushed` while `/slow` is in flight and the last URL after close"
    );
    assert_eq!(
        outcome.logs,
        ["saw 4 URLs before the tab closed"],
        "{outcome:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn page_url_reads_through_every_cross_document_commit() {
    let Some(binary) = e2e_browser() else {
        return;
    };
    let tab = Tab::open(binary, slow(COMMIT_RESPONSE), "/a").await;
    let paths: Vec<String> = iter::once("/a".to_string())
        .chain((0..COMMITS).map(|index| format!("/slow{index}")))
        .collect();
    let last_path = paths.last().unwrap();
    // No sleep between reads: each takes about a millisecond, so some land in every commit
    // window. The time cap only keeps a broken run from spinning until the run's timeout.
    let poll_url = format!(
        r"const seen = [page.url()];
const giveUpAt = Date.now() + 10000;
while (!seen[seen.length - 1].endsWith('{last_path}') && Date.now() < giveUpAt) {{
  const url = page.url();
  if (url !== seen[seen.length - 1]) seen.push(url);
}}
return seen.map((url) => url.slice(url.indexOf('/', 'http://'.length)));"
    );

    let run = tab.start_run(poll_url).await;
    time::sleep(STEP).await;
    for path in &paths[1..] {
        tab.navigate(path).await;
    }
    let outcome = run.await.unwrap();
    tab.close().await;

    assert_eq!(
        outcome.status,
        RunStatus::Ok,
        "a `page.url()` read inside a commit window must not throw: {outcome:?}"
    );
    assert_eq!(
        outcome.value,
        Some(json!(paths)),
        "each committed URL once, in order: {outcome:?}"
    );
}

/// The fixture these tests use: every page a stub, `/slow…` pages answering after `response`.
fn slow(response: Duration) -> Fixture {
    Fixture {
        slow_response: response,
        pages: Vec::new(),
    }
}
