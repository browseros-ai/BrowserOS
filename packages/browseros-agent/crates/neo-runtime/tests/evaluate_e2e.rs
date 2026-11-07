//! Page evaluation and JS handles end to end on a real BrowserOS neo, through neo-sdk's
//! `page.evaluate`, `page.evaluateHandle`, `JSHandle.jsonValue` and `locator.evaluate`: the
//! main world answers, and a run that ends by its deadline while an evaluation is still pending
//! and handles are alive stops on time and leaves the tab to the next run, whose handles are its
//! own. `crates/neo-sdk/tests/parity/evaluate*.test.mjs` and `jsHandle-*.test.mjs` compare the
//! same calls with live Playwright.
//!
//! Runs only when `NEO_E2E_BROWSER` names the browser binary (`support/browser.rs`); otherwise
//! each test passes as a skip.

#[path = "support/browser.rs"]
mod browser;

use std::time::{Duration, Instant};

use browser::{Fixture, Tab};
use neo_runtime::{RunOutcome, RunStatus};
use serde_json::{Value, json};

const PAGE: &str = r#"<!doctype html><html><head><title>Evaluate</title></head><body>
<p id="p">text</p>
<script>window.__probe = 'main'</script>
</body></html>"#;

/// The first run's own deadline: long enough to create its handles, far shorter than the test.
const FIRST_RUN_DEADLINE: Duration = Duration::from_secs(2);

/// How late a run stopped by its deadline may return.
const STOP_SLACK: Duration = Duration::from_secs(4);

async fn open() -> Option<Tab> {
    let binary = browser::e2e_browser()?;
    let fixture = Fixture {
        pages: vec![("/evaluate".to_string(), PAGE.to_string())],
        ..Fixture::default()
    };
    Some(Tab::open(binary, fixture, "/evaluate").await)
}

async fn finish(run: tokio::task::JoinHandle<RunOutcome>) -> RunOutcome {
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

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_a_run_ended_mid_evaluation_leaves_the_tab_to_the_next_run() {
    let Some(tab) = open().await else { return };
    let started = Instant::now();
    let first = finish(
        tab.start_run_within(
            r"const kept = await page.evaluateHandle(() => ({ kept: true }))
              const element = await page.evaluateHandle(() => document.querySelector('#p'))
              console.log(kept._guid)
              await page.evaluate(() => new Promise(() => {}))
              return 'unreachable'"
                .to_string(),
            FIRST_RUN_DEADLINE,
        )
        .await,
    )
    .await;
    assert_eq!(first.status, RunStatus::Timeout, "{first:?}");
    assert!(
        started.elapsed() < FIRST_RUN_DEADLINE + STOP_SLACK,
        "the pending evaluation held the run past its deadline: {:?}",
        started.elapsed()
    );
    let kept = first.logs.first().cloned().unwrap_or_default();
    assert!(kept.starts_with("handle@"), "{first:?}");

    let second = finish(
        tab.start_run(format!(
            r"const mine = await page.evaluateHandle(() => ({{ n: 2 }}))
              const Handle = mine.constructor
              const stale = new Handle(mine._connection, {kept:?}, {{ preview: 'stale' }})
              const settled = (promise) => promise.then(() => 'resolved', (error) => error.message)
              return {{
                probe: await page.evaluate(() => window.__probe),
                json: await mine.jsonValue(),
                text: await page.locator('#p').evaluate((e) => e.textContent),
                fresh: mine._guid !== {kept:?},
                stale: await settled(stale.jsonValue()),
              }}"
        ))
        .await,
    )
    .await;
    let second = value(&second);
    assert_eq!(second["probe"], json!("main"));
    assert_eq!(second["json"], json!({ "n": 2 }));
    assert_eq!(second["text"], json!("text"));
    assert_eq!(second["fresh"], json!(true));
    assert!(
        second["stale"]
            .as_str()
            .is_some_and(|message| message.contains("has been closed")),
        "a guid of the ended run names nothing now: {second}"
    );
    tab.close().await;
}
