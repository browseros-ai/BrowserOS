//! `page.addLocatorHandler` end to end on a real BrowserOS: a cookie banner the script dismisses
//! from a handler, through MCP `run`, the run's QuickJS prelude and the crate's checkpoint.
//!
//! ```text
//!  MCP run ─▶ browseros-claw-sdk ─ page.addLocatorHandler ─▶ the run's registry
//!    click     the banner covers the button: the click pauses, the pull runs the handler,
//!              its own click dismisses the banner, the paused click lands
//!    expect    Neo's own matcher op (the spec runner's expectLoop polls reads instead): the
//!              banner hides the target until the handler accepts it
//!    timeout   a handler whose locator never hides: the matcher times out waiting for it
//!    close     a handler that closes its page fails the paused action instead of hanging
//! ```
//!
//! Like `e2e_browseros.rs`, it runs only when `CLAW_SDK_E2E_BROWSER` names a BrowserOS binary.

#[path = "support/browseros_rig.rs"]
pub mod browseros_rig;

use std::time::Duration;

use browseros_rig::{CaptchaKey, Rig, TestResult, browser_or_skip, outcome, run};
use serde_json::{Value, json};
use tokio::time;

/// The whole drive; see `e2e_browseros.rs` for why the budget sits inside the test.
const TEST_BUDGET: Duration = Duration::from_secs(240);

/// A page whose banner covers the button and, once `show(true)` ran, hides it too, until the
/// banner's own button accepts it. `#stuck` is always visible.
const SCRIPT: &str = r#"
const html = '<style>#banner{position:fixed;inset:0;background:#fcc;display:none}'
  + '#banner.on{display:block}#buy.gone{display:none}</style>'
  + '<button id="buy" onclick="window.bought=(window.bought||0)+1">Buy</button>'
  + '<div id="banner"><p>We use cookies</p><button id="accept">Accept</button></div>'
  + '<div id="stuck">Always here</div>'
  + '<script>'
  + 'const banner=document.getElementById("banner"), buy=document.getElementById("buy");'
  + 'window.show=(hide)=>{banner.classList.add("on");if(hide)buy.classList.add("gone")};'
  + 'document.getElementById("accept").onclick=()=>{banner.classList.remove("on");buy.classList.remove("gone")};'
  + '</script>';
await page.goto('data:text/html,' + encodeURIComponent(html));
let handled = 0;
await page.addLocatorHandler(page.getByText('We use cookies'), async () => {
  handled++;
  await page.getByRole('button', { name: 'Accept' }).click();
});

await page.evaluate(() => window.show(false));
await page.locator('#buy').click();
const afterClick = { handled, bought: await page.evaluate(() => window.bought) };

await page.evaluate(() => window.show(true));
await expect(page.locator('#buy')).toBeVisible();
const afterExpect = handled;

await page.addLocatorHandler(page.locator('#stuck'), async () => {});
const stuck = await expect(page.locator('#buy')).toHaveText('Buy', { timeout: 1500 })
  .then(() => 'passed', (error) => error.message);
await page.removeLocatorHandler(page.locator('#stuck'));

const other = await context.newPage();
await other.goto('data:text/html,' + encodeURIComponent('<p>Sign up</p><button>Go</button>'));
await other.addLocatorHandler(other.getByText('Sign up'), async () => { await other.close(); });
const closed = await other.getByRole('button').click({ timeout: 10000 })
  .then(() => 'clicked', (error) => error.message);

return { afterClick, afterExpect, stuck, closed };
"#;

#[tokio::test(flavor = "multi_thread")]
async fn a_locator_handler_dismisses_a_banner_before_actions_and_matchers() -> TestResult {
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

async fn drive(rig: &Rig) -> TestResult {
    rig.until_ready().await?;
    let client = rig.client().await?;
    let result = run(&client, SCRIPT, &json!({ "taskId": "banner" })).await?;
    assert_eq!(
        outcome(&result)?.get("status"),
        Some(&json!("ok")),
        "{result:?}"
    );
    client.cancel().await?;
    let reply = result
        .structured_content
        .ok_or("a run's reply is its structured content")?;
    let value = reply.get("value").ok_or("the script returns")?;
    assert_eq!(
        value.get("afterClick"),
        Some(&json!({ "handled": 1, "bought": 1 })),
        "the click waited for the handler, then landed: {reply}"
    );
    assert_eq!(
        value.get("afterExpect"),
        Some(&json!(2)),
        "the matcher ran the handler before it read: {reply}"
    );
    let stuck = value
        .get("stuck")
        .and_then(Value::as_str)
        .unwrap_or_default();
    assert!(
        stuck.contains("locator handler has finished, waiting for locator('#stuck') to be hidden"),
        "a handler's locator that never hides times the matcher out, logged: {reply}"
    );
    let closed = value
        .get("closed")
        .and_then(Value::as_str)
        .unwrap_or_default();
    assert!(
        closed.contains("Target page, context or browser has been closed"),
        "a handler that closes its page fails the action at once: {reply}"
    );
    Ok(())
}
