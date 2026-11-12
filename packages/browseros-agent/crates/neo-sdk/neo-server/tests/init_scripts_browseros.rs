//! Init scripts and bindings on a real BrowserOS, through MCP `run`: `page.addInitScript` and
//! `context.addInitScript` run in every new document of a tab, before the page's own scripts,
//! and `exposeBinding` lets the page call the script's functions, each until the run that
//! registered it ends; the owner's lock refuses a registration on the tab it holds, as it refuses
//! any other write.
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
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use browseros_rig::{CaptchaKey, HOST_TOKEN, Rig, TestResult};
use neo_server::test_support;
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

/// Run 1 of the bindings: what the page got back from the script's functions.
const EXPOSE: &str = r"
await page.goto('data:text/html,<title>Bindings</title><p>loaded</p>');
let source = null;
const add = await page.exposeBinding('add', (s, a, b) => {
  source = [s.page === page, s.frame === page.mainFrame(), s.context === context];
  return a + b;
});
const loaded = await page.evaluate(() => window.add(2, 3));
await page.exposeFunction('fail', () => { throw new Error('boom'); });
const failed = await page.evaluate(() => window.fail().then(() => 'resolved', (e) => e.message));
let duplicate;
try {
  await page.exposeFunction('add', () => 0);
  duplicate = 'registered';
} catch (error) {
  duplicate = error.message;
}
await context.exposeFunction('twice', (n) => n * 2);
await page.goto('data:text/html,' + encodeURIComponent('<script>window.early = twice(21)</script>'));
const early = await page.evaluate(() => window.early);
const called = [];
await page.addInitScript(({ cb }) => cb('ran'), { cb: (v) => { called.push(v); } }, { exposeFunctions: true });
await page.reload();
for (let i = 0; i < 100 && called.length === 0; i++) await page.waitForTimeout(50);
const onWindow = await page.evaluate(() => Object.keys(window).filter((key) => key.startsWith('__pw_fn_')));
await add.dispose();
const disposed = await page.evaluate(() => typeof window.add);
// More calls than Neo's event log holds, none awaited but the last: every one reaches the script.
let ticks = 0;
await page.exposeFunction('tick', () => { ticks++; });
await page.evaluate(async () => { for (let i = 0; i < 3000; i++) window.tick(); await window.tick(); });
return { loaded, source, failed, duplicate, early, called, onWindow, disposed, ticks, page };
";

/// Run 2 of the bindings, on run 1's tab: what is left of them once run 1 ended, in the document
/// it left loaded and in a new one.
const EXPOSED_AFTER: &str = r"
const loaded = await page.evaluate(() => [typeof window.twice, typeof window.fail]);
await page.goto('data:text/html,<p>next run</p>');
const fresh = await page.evaluate(() => [typeof window.twice, typeof window.fail]);
return { page, loaded, fresh };
";

/// Run 3 of the bindings, on the locked tab.
const EXPOSE_LOCKED: &str = r"
try {
  await page.exposeFunction('locked', () => 1);
  return { page, met: 'registered' };
} catch (error) {
  return { page, met: [error.owner, error.code, error.delivery] };
}
";

#[tokio::test(flavor = "multi_thread")]
async fn init_scripts_run_in_every_new_document_until_the_run_ends() -> TestResult {
    on_browseros(|rig| Box::pin(drive(rig))).await
}

#[tokio::test(flavor = "multi_thread")]
async fn pages_call_back_into_the_script_until_the_run_ends() -> TestResult {
    on_browseros(|rig| Box::pin(expose(rig))).await
}

/// `drive` on a fresh BrowserOS within the test's budget, with both processes' logs on failure.
async fn on_browseros(
    drive: impl for<'a> FnOnce(&'a Rig) -> Pin<Box<dyn Future<Output = TestResult> + 'a>>,
) -> TestResult {
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

async fn expose(rig: &Rig) -> TestResult {
    rig.until_ready().await?;
    let client = rig.client().await?;
    let meta = json!({ "taskId": "bindings" });

    let exposed = returned(&browseros_rig::run(&client, EXPOSE, &meta).await?)?;
    let tab = exposed
        .pointer("/page/tab")
        .and_then(Value::as_i64)
        .ok_or_else(|| format!("a page leaves a run as its brand: {exposed}"))?;
    let expected = json!({
        "loaded": 5,
        "source": [true, true, true],
        "failed": "boom",
        "duplicate": "page.exposeFunction: Function \"add\" has been already registered",
        "early": 42,
        "called": ["ran"],
        "onWindow": [],
        "disposed": "undefined",
        "ticks": 3001,
        "page": { "$neo": "page", "tab": tab },
    });
    assert_eq!(
        exposed, expected,
        "a binding answers the loaded document at once and every later one, with its source; an \
         error reaches the page; a context binding reaches the run's tab; exposeFunctions passes \
         the script's function off window; dispose takes it from the page; a burst of calls \
         larger than the event log loses none"
    );

    let after = returned(&browseros_rig::run(&client, EXPOSED_AFTER, &meta).await?)?;
    assert_eq!(
        after,
        json!({
            "page": { "$neo": "page", "tab": tab },
            "loaded": ["undefined", "undefined"],
            "fresh": ["undefined", "undefined"],
        }),
        "a run's bindings leave the document it left loaded, and later ones, when it ends"
    );

    let take = json!({ "tabId": tab, "reads": "allow", "ttlSeconds": 600 });
    let taken = rig
        .host("POST", "/v1/locks", Some(HOST_TOKEN), Some(&take))
        .await?;
    if taken.status != 200 {
        return Err(format!("the host locks the tab: {}", taken.text()).into());
    }
    let locked = returned(&browseros_rig::run(&client, EXPOSE_LOCKED, &meta).await?)?;
    client.cancel().await?;
    assert_eq!(
        locked,
        json!({
            "page": { "$neo": "page", "tab": tab },
            "met": ["host", "owner_has_control", "not_sent"],
        }),
        "the owner's lock refuses a binding on its tab as it refuses any write"
    );
    Ok(())
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
    test_support::run_value(result).map_err(Into::into)
}
