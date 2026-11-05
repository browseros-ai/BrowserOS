//! Playwright scripts end to end on a real BrowserOS: an agent's own Playwright calls, sent over
//! MCP `run`, on a page with a same-origin iframe, on a page that logs, and before any op.
//!
//! ```text
//!  MCP run ─▶ browseros-claw-sdk ─ browseros-playwright's prelude ─▶ the door ─▶ the crate's ops
//!    page.goto (the run's lazy page: the door opens the tab) ─ page.ariaSnapshot({ mode: 'ai' })
//!    ─ getByRef / aria-ref= on a printed ref ─ locator reads ─ reads inside the iframe
//!  page.on('console') first: page.frames opens the lazy tab ─ events.listen ─ the event pump
//!  context.pages() first: the run's init lists every tab
//! ```
//!
//! Like `e2e_browseros.rs`, it runs only when `CLAW_SDK_E2E_BROWSER` names a BrowserOS binary.

#[path = "support/browseros_rig.rs"]
pub mod browseros_rig;

use std::error::Error;
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use browseros_rig::{CaptchaKey, Rig, TestResult, browser_or_skip, outcome, run};
use serde_json::{Value, json};
use tokio::time;

/// The whole drive; see `e2e_browseros.rs` for why the budget sits inside the test.
const TEST_BUDGET: Duration = Duration::from_secs(240);

/// What a model trained on Playwright writes: load a page, read its snapshot, act on a ref it
/// printed, then read through locators and inside the frame. The page is built in the script, so
/// the test's expectations sit next to the markup they come from. A `srcdoc` iframe shares its
/// parent's origin, and a `data:` page's frame tree needs no server.
const SCRIPT: &str = r#"
const html = '<title>Neo Playwright</title>'
  + '<h1>Orders</h1>'
  + '<p>Two buttons, one frame.</p>'
  + '<button>Save order</button><button>Cancel</button>'
  + '<iframe title="Details" srcdoc="<p>Inside the frame</p><button>Frame action</button>"></iframe>';
await page.goto('data:text/html,' + encodeURIComponent(html));
const snapshot = await page.ariaSnapshot({ mode: 'ai' });
const ref = /button "Save order" \[ref=(e\d+)\]/.exec(snapshot)?.[1];
const frameRef = /button "Frame action" \[ref=(f\d+e\d+)\]/.exec(snapshot)?.[1];
if (!ref || !frameRef) return { snapshot };
return {
  ref,
  frameRef,
  byRef: await page.getByRef(ref).textContent(),
  byAriaRef: await page.locator(`aria-ref=${ref}`).innerText(),
  frameByRef: await page.getByRef(frameRef).textContent(),
  buttons: await page.getByRole('button').count(),
  heading: await page.getByRole('heading', { level: 1 }).textContent(),
  note: await page.getByText('Two buttons').textContent(),
  inFrame: await page.frameLocator('iframe').getByRole('button').textContent(),
  frames: page.frames().length,
  frameText: await page.frames()[1].locator('p').textContent(),
  title: await page.title(),
};
"#;

/// A fresh task whose first SDK call is a listener (decision 25, PB input 3). The run's page has
/// no tab yet, and event ops name no page for the door to resolve, so the prelude first sends a
/// `page.frames`, which the door resolves by opening the tab; the listener then names that tab.
/// The page logs on a timer, so the listener hears it whenever its listen lands.
const LISTENER_FIRST: &str = r#"
const heard = [];
page.on('console', (message) => heard.push(message.text()));
const html = '<title>Console</title><script>let n = 0; setInterval(() => console.log("tick " + n++), 50);</script>';
await page.goto('data:text/html,' + encodeURIComponent(html));
for (let i = 0; i < 100 && heard.length === 0; i++) {
  await new Promise((resolve) => setTimeout(resolve, 50));
}
return { heard: heard.slice(0, 1), title: await page.title() };
"#;

#[tokio::test(flavor = "multi_thread")]
async fn a_playwright_script_reads_a_page_its_refs_and_its_frame() -> TestResult {
    on_browseros(|rig| Box::pin(drive(rig))).await
}

#[tokio::test(flavor = "multi_thread")]
async fn a_fresh_tasks_first_call_can_be_a_console_listener() -> TestResult {
    on_browseros(|rig| Box::pin(listen_first(rig))).await
}

#[tokio::test(flavor = "multi_thread")]
async fn context_pages_lists_every_tab_in_a_runs_first_statement() -> TestResult {
    on_browseros(|rig| Box::pin(tabs_first(rig))).await
}

/// `drive` on a fresh BrowserOS within the test's budget, with both processes' logs on failure.
async fn on_browseros(
    drive: impl for<'a> FnOnce(&'a Rig) -> Pin<Box<dyn Future<Output = TestResult> + 'a>>,
) -> TestResult {
    let Some(browser) = browser_or_skip() else {
        return Ok(());
    };
    let rig = Rig::start(&browser, CaptchaKey::Absent)?;
    let failure = match time::timeout(TEST_BUDGET, drive(&rig)).await {
        Ok(Ok(())) => return Ok(()),
        Ok(Err(error)) => error.to_string(),
        Err(_elapsed) => format!("the drive did not finish within {TEST_BUDGET:?}"),
    };
    Err(format!("{failure}\n{}", rig.logs()).into())
}

/// The reply of one `run`, which must have succeeded.
async fn reply_of(rig: &Rig, code: &str, task: &str) -> Result<Value, Box<dyn Error>> {
    rig.until_ready().await?;
    let client = rig.client().await?;
    let result = run(&client, code, &json!({ "taskId": task })).await?;
    assert_eq!(
        outcome(&result)?.get("status"),
        Some(&json!("ok")),
        "{result:?}"
    );
    client.cancel().await?;
    result
        .structured_content
        .ok_or_else(|| "a run's reply is its structured content".into())
}

async fn listen_first(rig: &Rig) -> TestResult {
    let reply = reply_of(rig, LISTENER_FIRST, "listener-first").await?;
    let value = reply.get("value").ok_or("the script returns")?;
    assert_eq!(value.get("title"), Some(&json!("Console")), "{reply}");
    let heard = value
        .get("heard")
        .and_then(Value::as_array)
        .and_then(|heard| heard.first())
        .and_then(Value::as_str);
    assert!(
        heard.is_some_and(|text| text.starts_with("tick ")),
        "the listener heard the page it was set on: {reply}"
    );
    assert_eq!(
        reply.get("notAwaited"),
        None,
        "the event pump still pending at the end is cut quietly: {reply}"
    );
    Ok(())
}

async fn tabs_first(rig: &Rig) -> TestResult {
    let reply = reply_of(
        rig,
        "return context.pages().map((tab) => tab.url());",
        "tabs-first",
    )
    .await?;
    assert_eq!(
        reply.get("value"),
        Some(&json!(["about:blank"])),
        "the tab the browser started with, listed before any op: {reply}"
    );
    Ok(())
}

async fn drive(rig: &Rig) -> TestResult {
    rig.until_ready().await?;
    let client = rig.client().await?;
    let result = run(&client, SCRIPT, &json!({ "taskId": "playwright" })).await?;
    assert_eq!(
        outcome(&result)?.get("status"),
        Some(&json!("ok")),
        "{result:?}"
    );
    let value = result
        .structured_content
        .as_ref()
        .and_then(|content| content.get("value"))
        .ok_or_else(|| format!("the script returns its reads: {result:?}"))?;
    let snapshot = value.get("snapshot").and_then(Value::as_str);
    assert_eq!(
        snapshot, None,
        "the snapshot printed a ref for both buttons: {value}"
    );
    let text = |name: &str| value.get(name).and_then(Value::as_str);
    assert!(
        text("ref").is_some_and(|printed| printed.starts_with('e')),
        "a main-frame ref is bare (`e5`): {value}"
    );
    assert!(
        text("frameRef").is_some_and(|printed| printed.starts_with('f')),
        "a ref inside the iframe names its document (`f1e2`): {value}"
    );
    let reads: Vec<(&str, Value)> = [
        "byRef",
        "byAriaRef",
        "frameByRef",
        "buttons",
        "heading",
        "note",
        "inFrame",
        "frames",
        "frameText",
        "title",
    ]
    .into_iter()
    .map(|name| (name, value.get(name).cloned().unwrap_or(Value::Null)))
    .collect();
    assert_eq!(
        reads,
        [
            ("byRef", json!("Save order")),
            ("byAriaRef", json!("Save order")),
            ("frameByRef", json!("Frame action")),
            ("buttons", json!(2)),
            ("heading", json!("Orders")),
            ("note", json!("Two buttons, one frame.")),
            ("inFrame", json!("Frame action")),
            ("frames", json!(2)),
            ("frameText", json!("Inside the frame")),
            ("title", json!("Neo Playwright")),
        ],
        "each read answers from the markup the script loaded: {value}"
    );
    client.cancel().await?;
    Ok(())
}
