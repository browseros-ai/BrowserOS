//! Neo's default close of a page that asks to stay ("Leave site?"), on a real browser through
//! the built server, as the first host measured it on the published image: a payment page whose
//! beforeunload prevents leaving, its card number set by a script, then closed by the next run
//! the way the host closes it (zuzu's `close_tab.js`). Closed must mean gone from the browser's
//! own tab list, and the run after it must not stall. Runs only when `CLAW_SDK_E2E_BROWSER`
//! names a BrowserOS binary (`browseros_rig`).

#[path = "support/browseros_rig.rs"]
pub mod browseros_rig;

use std::time::Duration;

use browseros_rig::{CaptchaKey, Rig, TestResult, browser_or_skip, get, outcome, run};
use serde_json::json;
use tokio::time;

/// The payment page: a card number box and a beforeunload handler that prevents leaving.
const PAY: &str = "data:text/html,<title>Neo checkout</title><h1>Checkout</h1>\
    <input id=card autocomplete=cc-number><script>addEventListener('beforeunload', \
    (event) => { event.preventDefault(); event.returnValue = ''; })</script>";
const PAY_TITLE: &str = "Neo checkout";
/// What the next run opens, in another task.
const NEXT: &str = "data:text/html,<title>Neo next</title><p>next</p>";
const NEXT_TITLE: &str = "Neo next";
/// After the old close the next run took 60 s on the published image and 12 s on a headless
/// BrowserOS here; after this one, well under a second.
const NO_STALL: Duration = Duration::from_secs(5);

#[tokio::test(flavor = "multi_thread")]
async fn a_page_that_prevents_leaving_closes_and_the_next_run_does_not_stall() -> TestResult {
    let Some(browser) = browser_or_skip() else {
        return Ok(());
    };
    let rig = Rig::start(&browser, CaptchaKey::Absent)?;
    let failure = match time::timeout(Duration::from_secs(240), drive(&rig)).await {
        Ok(Ok(())) => return Ok(()),
        Ok(Err(error)) => error.to_string(),
        Err(_elapsed) => "the drive did not finish within 240 s".to_owned(),
    };
    Err(format!("{failure}\n{}", rig.logs()).into())
}

async fn drive(rig: &Rig) -> TestResult {
    rig.until_ready().await?;
    let client = rig.client().await?;

    let opened = run(
        &client,
        &format!(
            r"
            const p = await context.newPage();
            await p.goto({PAY:?});
            await p.evaluate(() => {{ document.querySelector('#card').value = '4111111111111111'; }});
            await p.click('h1');
            const active = await p.evaluate(() => navigator.userActivation.hasBeenActive);
            return {{ tab: p.toJSON().tab, active }};"
        ),
        &json!({ "requestId": "pay-open", "taskId": "pay" }),
    )
    .await?;
    let value = opened
        .structured_content
        .as_ref()
        .and_then(|content| content.get("value"))
        .ok_or_else(|| format!("the open run returned a value: {opened:?}"))?;
    assert_eq!(
        value.get("active"),
        Some(&json!(true)),
        "the page may ask to stay: {value}"
    );
    let tab = value
        .get("tab")
        .and_then(serde_json::Value::as_i64)
        .ok_or("the page names its tab")?;

    let closed = run(
        &client,
        &format!(
            r"
            const p = context.pages().find((q) => q.toJSON().tab === {tab});
            if (p) await p.close();
            return {{ closed: true, stillOpen: context.pages().some((q) => q.toJSON().tab === {tab}) }};"
        ),
        &json!({ "requestId": "pay-close", "taskId": "pay" }),
    )
    .await?;
    assert_eq!(
        outcome(&closed)?.get("status"),
        Some(&json!("ok")),
        "{closed:?}"
    );
    assert_eq!(
        closed.structured_content,
        Some(json!({ "value": { "closed": true, "stillOpen": false } })),
    );
    let listed = get(rig.cdp_port(), "/json/list").await?;
    assert!(
        !listed.contains(PAY_TITLE),
        "closed means gone from the browser's own tab list: {listed}"
    );

    let next = time::timeout(
        NO_STALL,
        run(
            &client,
            &format!(
                r"
                const p = await context.newPage();
                await p.goto({NEXT:?});
                const t = await p.title();
                await p.close();
                return t;"
            ),
            &json!({ "requestId": "next", "taskId": "other" }),
        ),
    )
    .await
    .map_err(|_elapsed| format!("the next run stalled past {NO_STALL:?}"))??;
    assert_eq!(
        outcome(&next)?.get("status"),
        Some(&json!("ok")),
        "{next:?}"
    );
    assert_eq!(
        next.structured_content,
        Some(json!({ "value": NEXT_TITLE }))
    );
    client.cancel().await?;
    Ok(())
}
