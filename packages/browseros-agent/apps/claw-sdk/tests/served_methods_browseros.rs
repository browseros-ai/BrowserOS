//! What the script's realm receives on a real BrowserOS, through MCP `run`: no value the run holds
//! reaches it raw, whichever way Neo hands the script a text (decision 35b). The Playwright methods
//! the held round serves get their cases here as each one is served.
//!
//! ```text
//!  page.fill(password) ─ the seal's journal ─ held: no secret until its field is quiet for 1 s
//!  click ─ the page alerts the field ─ events.next ─ the door: masked with what the run holds
//!    ─▶ page.on('dialog') in the realm reads [private], and the run's records keep none of it
//! ```
//!
//! Like `e2e_browseros.rs`, it runs only when `CLAW_SDK_E2E_BROWSER` names a BrowserOS binary.

#[path = "support/browseros_rig.rs"]
pub mod browseros_rig;

use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use browseros_rig::{CaptchaKey, HOST_TOKEN, Rig, TestResult};
use serde_json::{Value, json};
use tokio::time;

/// The whole drive; see `e2e_browseros.rs` for why the budget sits inside the test.
const TEST_BUDGET: Duration = Duration::from_secs(240);

/// claw-sdk's quiet period (`Defaults::TYPED_QUIET`): a typed value becomes a secret of the
/// process's set only this long after its field's last key.
const TYPED_QUIET: Duration = Duration::from_secs(1);

/// The agent types `SECRET` into a password field, and the page then shows the field's value in
/// an alert. The script reads the dialog as Playwright's `page.on('dialog')` does, and reports
/// whether the realm saw the value raw: its reply is masked again on the way out, so the reply
/// alone could not tell. It also times the dialog from the fill, since a dialog later than the
/// quiet period meets the process's set and would show nothing about the run's held values.
const DIALOG_OF_A_TYPED_PASSWORD: &str = r#"
const html = '<title>Sign in</title><input type="password" id="password">'
  + '<button id="show" onclick="alert(\'typed: \' + document.getElementById(\'password\').value)">Show</button>';
await page.goto('data:text/html,' + encodeURIComponent(html));
const shown = new Promise((resolve) => {
  page.once('dialog', async (dialog) => {
    const message = dialog.message();
    await dialog.accept();
    resolve(message);
  });
});
const typedAt = Date.now();
await page.locator('#password').fill(SECRET);
await page.locator('#show').click();
const message = await shown;
return { message, sawTheValue: message.includes(SECRET), afterMs: Date.now() - typedAt };
"#;

#[tokio::test(flavor = "multi_thread")]
async fn a_dialog_that_shows_a_typed_password_reaches_the_script_masked() -> TestResult {
    on_browseros(|rig| Box::pin(dialog_of_a_typed_password(rig))).await
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

async fn dialog_of_a_typed_password(rig: &Rig) -> TestResult {
    let secret = "hunter2-dialog-7Qx";
    rig.until_ready().await?;
    let client = rig.client().await?;
    let code = format!(
        "const SECRET = {};\n{DIALOG_OF_A_TYPED_PASSWORD}",
        json!(secret)
    );
    let result = browseros_rig::run(&client, &code, &json!({ "taskId": "dialog" })).await?;
    client.cancel().await?;
    let meta = browseros_rig::outcome(&result)?;
    assert_eq!(meta.get("status"), Some(&json!("ok")), "{result:?}");
    let value = result
        .structured_content
        .as_ref()
        .and_then(|content| content.get("value"))
        .ok_or_else(|| format!("the script returns what it read: {result:?}"))?;
    let after = value
        .get("afterMs")
        .and_then(Value::as_u64)
        .map(Duration::from_millis)
        .ok_or_else(|| format!("the script times the dialog: {value}"))?;
    assert!(
        after < TYPED_QUIET,
        "the dialog came {after:?} after the fill, past the quiet period, so the process's set may \
         have masked it and this run proves nothing about the held values: {value}"
    );
    assert_eq!(
        (value.get("sawTheValue"), value.get("message")),
        (Some(&json!(false)), Some(&json!("typed: [private]"))),
        "the dialog reached the script with the typed password masked: {value}"
    );

    let id = meta
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("the run has an id: {meta}"))?;
    let read = rig
        .host(
            "GET",
            &format!("/v1/runs/{id}?wait=30"),
            Some(HOST_TOKEN),
            None,
        )
        .await?;
    assert_eq!(read.status, 200, "{}", read.text());
    let recorded = String::from_utf8_lossy(&read.body);
    assert!(
        recorded.contains("events.") && !recorded.contains(secret),
        "the run's records, the dialog's event op among them, keep none of the password: {recorded}"
    );
    Ok(())
}
