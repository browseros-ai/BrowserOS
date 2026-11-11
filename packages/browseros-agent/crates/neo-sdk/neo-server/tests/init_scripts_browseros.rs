//! Init scripts on a real BrowserOS, through MCP `run`: `page.addInitScript` and
//! `context.addInitScript` run in every new document of a tab, before the page's own scripts,
//! until the run that registered them ends, and the owner's lock refuses a registration on the
//! tab it holds, as it refuses any other write.
//!
//! ```text
//!  run 1: page script, context script ─ goto, goto ─ dispose the page's ─ goto ─ a new tab
//!  run 2, same tab: goto ─ neither script runs: the run that registered them ended
//!  run 3, same tab, locked: page.addInitScript ─ host / owner_has_control, not_sent
//! ```
//!
//! Like `e2e_browseros.rs`, it runs only when `CLAW_SDK_E2E_BROWSER` names a BrowserOS binary.

#[path = "support/browseros_rig.rs"]
pub mod browseros_rig;

use std::error::Error;
use std::time::Duration;

use browseros_rig::{CaptchaKey, HOST_TOKEN, Rig, TestResult};
use rmcp::model::CallToolResult;
use serde_json::{Value, json};
use tokio::time;

/// The whole drive; see `e2e_browseros.rs` for why the budget sits inside the test.
const TEST_BUDGET: Duration = Duration::from_secs(240);

/// Run 1: what each document saw of the run's scripts.
const REGISTER: &str = r"
const read = () => page.evaluate(() => [window.initTag ?? null, window.pageSaw ?? null, window.fromContext ?? null]);
const disposable = await page.addInitScript((tag) => { window.initTag = tag }, 'from-page');
await context.addInitScript(
  'window.fromContext = typeof window.initTag === `string` ? `after-page` : `alone`');
await page.goto('data:text/html,' + encodeURIComponent('<script>window.pageSaw = window.initTag</script>'));
const first = await read();
await page.goto('data:text/html,<p>second</p>');
const second = await read();
await disposable.dispose();
await disposable.dispose();
await page.goto('data:text/html,<p>third</p>');
const third = await read();
const other = await context.newPage();
await other.goto('data:text/html,<p>another tab</p>');
const otherTab = await other.evaluate(() => [window.initTag ?? null, window.fromContext ?? null]);
await other.close();
// The task's last tab is the one its next run gets: this one, not the closed one.
await page.evaluate(() => 0);
return { first, second, third, otherTab, page };
";

/// Run 2, on run 1's tab: what a new document sees once run 1 ended.
const AFTER: &str = r"
await page.goto('data:text/html,<p>next run</p>');
return { page, saw: await page.evaluate(() => [window.initTag ?? null, window.fromContext ?? null]) };
";

/// Run 3, on the locked tab: what a registration meets.
const LOCKED: &str = r"
try {
  await page.addInitScript('window.locked = true');
  return { page, met: 'registered' };
} catch (error) {
  return { page, met: [error.owner, error.code, error.delivery] };
}
";

#[tokio::test(flavor = "multi_thread")]
async fn init_scripts_run_in_every_new_document_until_the_run_ends() -> TestResult {
    let Some(browser) = browseros_rig::browser_or_skip() else {
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

async fn drive(rig: &Rig) -> TestResult {
    rig.until_ready().await?;
    let client = rig.client().await?;
    let meta = json!({ "taskId": "init-scripts" });

    let registered = returned(&browseros_rig::run(&client, REGISTER, &meta).await?)?;
    let tab = registered
        .pointer("/page/tab")
        .and_then(Value::as_i64)
        .ok_or_else(|| format!("a page leaves a run as its brand: {registered}"))?;
    assert_eq!(
        registered.get("first"),
        Some(&json!(["from-page", "from-page", "after-page"])),
        "both scripts ran before the page's own, the page's first, as registered: {registered}"
    );
    assert_eq!(
        registered.get("second"),
        Some(&json!(["from-page", null, "after-page"])),
        "a later goto in the same run gets both: {registered}"
    );
    assert_eq!(
        registered.get("third"),
        Some(&json!([null, null, "alone"])),
        "the disposed page script is gone, and the context's stays: {registered}"
    );
    assert_eq!(
        registered.get("otherTab"),
        Some(&json!([null, "alone"])),
        "the context's script reaches a tab the run opened, and the page's does not: \
         {registered}"
    );

    let after = returned(&browseros_rig::run(&client, AFTER, &meta).await?)?;
    assert_eq!(
        after,
        json!({ "page": { "$neo": "page", "tab": tab }, "saw": [null, null] }),
        "the same tab no longer runs a script whose run ended"
    );

    let take = json!({ "tabId": tab, "reads": "allow", "ttlSeconds": 600 });
    let taken = rig
        .host("POST", "/v1/locks", Some(HOST_TOKEN), Some(&take))
        .await?;
    if taken.status != 200 {
        return Err(format!("the host locks the tab: {}", taken.text()).into());
    }
    let locked = returned(&browseros_rig::run(&client, LOCKED, &meta).await?)?;
    client.cancel().await?;
    assert_eq!(
        locked,
        json!({ "page": { "$neo": "page", "tab": tab }, "met": ["host", "owner_has_control", "not_sent"] }),
        "the owner's lock refuses a registration on its tab as it refuses any write"
    );
    Ok(())
}

/// What a run returned, once it ended `ok`.
fn returned(result: &CallToolResult) -> Result<Value, Box<dyn Error>> {
    let meta = browseros_rig::outcome(result)?;
    if meta.get("status") != Some(&json!("ok")) {
        return Err(format!("the run ends ok: {result:?}").into());
    }
    result
        .structured_content
        .as_ref()
        .and_then(|content| content.get("value"))
        .cloned()
        .ok_or_else(|| format!("the script returns what it saw: {result:?}").into())
}
