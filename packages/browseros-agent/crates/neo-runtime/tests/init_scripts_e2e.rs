//! Init scripts end to end on a real BrowserOS neo, through neo-sdk's `page.addInitScript` and
//! `browserContext.addInitScript`, for Neo's one deviation from Playwright here: a script lives
//! as long as the run that added it (`Deviation:` in `src/init_scripts.rs`). A run's scripts run
//! in every frame of its new documents, the cross-site out-of-process iframe included; once the
//! run ends, by its script's return or by its deadline, no later document runs one, and no iframe
//! is left held at its start. `crates/neo-sdk/tests/parity/addInitScript.test.mjs` compares the
//! in-run behaviour with live Playwright.
//!
//! Runs only when `NEO_E2E_BROWSER` names the browser binary (`support/browser.rs`); otherwise
//! each test passes as a skip.

#[path = "support/browser.rs"]
mod browser;

use std::time::{Duration, Instant};

use browser::{Fixture, Tab};
use neo_runtime::{RunOutcome, RunStatus};
use serde_json::{Value, json};
use tokio::task::JoinHandle;
use tokio::time;

/// Counts its leaves as they load (`__loaded`), the cross-site one in another process.
const TOP: &str = r"<!doctype html><title>Top</title>
<script>window.__loaded = 0; addEventListener('message', (event) => { if (event.data === 'leaf') window.__loaded++ })</script>
<iframe id=same src=/leaf.html></iframe><iframe id=cross></iframe>
<script>document.getElementById('cross').src = 'http://localhost:' + location.port + '/leaf.html'</script>";
const LEAF: &str =
    r"<!doctype html><title>Leaf</title><script>parent.postMessage('leaf', '*')</script>";

/// Each frame's host and what the scripts left in it, top first.
const READ_FRAMES: &str = r"return await Promise.all(page.frames().map((frame) => frame.evaluate(() => [location.hostname, window.__page ?? null, window.__context ?? null])))";

const ADD_SCRIPTS: &str = r"await page.addInitScript(() => { window.__page = (window.__page || 0) + 1 })
await page.context().addInitScript(() => { window.__context = true })
await page.goto(page.url())";

/// The second run's own deadline: long enough to add its scripts and navigate.
const DEADLINE: Duration = Duration::from_secs(8);

/// How long a new document may take to load both leaves.
const LOAD: Duration = Duration::from_secs(15);

async fn open() -> Option<Tab> {
    let binary = browser::e2e_browser()?;
    let fixture = Fixture {
        pages: vec![
            ("/top".to_string(), TOP.to_string()),
            ("/leaf.html".to_string(), LEAF.to_string()),
        ],
        ..Fixture::default()
    };
    let tab = Tab::open(binary, fixture, "/top").await;
    loaded(&tab).await;
    Some(tab)
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

/// Waits until both leaves of the tab's document loaded: an iframe held at its start never does.
async fn loaded(tab: &Tab) {
    let started = Instant::now();
    while tab.evaluate("window.__loaded").await != json!(2) {
        assert!(
            started.elapsed() < LOAD,
            "a leaf never loaded: {}",
            tab.evaluate("[location.href, window.__loaded]").await
        );
        time::sleep(Duration::from_millis(50)).await;
    }
}

/// Navigates the tab outside Neo, to a new document no run touches, and reads its frames in a
/// run that adds no script.
async fn later_document(tab: &Tab) -> Vec<Value> {
    tab.navigate("/top").await;
    loaded(tab).await;
    by_host(value(
        &finish(tab.start_run(READ_FRAMES.to_string()).await).await,
    ))
}

/// The three frames as [`READ_FRAMES`] reads them, in host order: `page.frames()` lists an
/// out-of-process iframe where its attach lands.
fn frames(page: &Value, context: &Value) -> Vec<Value> {
    vec![
        json!(["127.0.0.1", page, context]),
        json!(["127.0.0.1", page, context]),
        json!(["localhost", page, context]),
    ]
}

fn by_host(read: Value) -> Vec<Value> {
    let Value::Array(mut frames) = read else {
        panic!("frames read as a list: {read}");
    };
    frames.sort_by_key(Value::to_string);
    frames
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_a_runs_init_scripts_end_with_the_run() {
    let Some(tab) = open().await else { return };

    let first = finish(tab.start_run(format!("{ADD_SCRIPTS}\n{READ_FRAMES}")).await).await;
    assert_eq!(
        by_host(value(&first)),
        frames(&json!(1), &json!(true)),
        "the run's scripts ran once in every frame of its new document"
    );
    assert_eq!(
        later_document(&tab).await,
        frames(&Value::Null, &Value::Null),
        "a run that returned left no script behind"
    );

    let started = Instant::now();
    let second = finish(
        tab.start_run_within(
            format!("{ADD_SCRIPTS}\nconsole.log('navigated')\nawait new Promise(() => {{}})"),
            DEADLINE,
        )
        .await,
    )
    .await;
    assert_eq!(second.status, RunStatus::Timeout, "{second:?}");
    assert_eq!(second.logs, ["navigated"], "{second:?}");
    assert!(
        started.elapsed() < DEADLINE + Duration::from_secs(4),
        "removing the scripts held the run past its deadline: {:?}",
        started.elapsed()
    );
    assert_eq!(
        later_document(&tab).await,
        frames(&Value::Null, &Value::Null),
        "a run ended by its deadline left no script behind"
    );
    tab.close().await;
}
