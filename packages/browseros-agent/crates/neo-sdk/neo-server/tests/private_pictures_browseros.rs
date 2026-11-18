//! Stored pictures follow the host's `fill.pictures` setting. Visible card numbers are stored
//! under `allowed`; `never` withholds pictures after private typing. The restart rows pin the
//! filled-tab record and the existing limit that fills under `allowed` write no record.
//!
//! Runs when `CLAW_SDK_E2E_BROWSER` names a BrowserOS binary.

#[path = "support/browseros_rig.rs"]
pub mod browseros_rig;

use std::error::Error;
use std::time::Duration;

use browseros_rig::{
    CaptchaKey, Client, HOST_TOKEN, Rig, TestResult, browser_or_skip, outcome, run,
};
use rmcp::model::CallToolResult;
use serde_json::{Value, json};
use tokio::time;

/// Each test's whole budget.
const TEST_BUDGET: Duration = Duration::from_secs(240);
/// The tasks the scripts act in.
const TASK: &str = "private-pictures";
/// A card number the page declares (`autocomplete="cc-number"`), typed by a script.
const CARD: &str = "4242424242424242";
/// What the host types privately into a box the page does not declare.
const MARKED: &str = "CANARY-restart-77";

/// A page with a card box, a password box, a plain box and a button, built in the script.
const PAGE: &str = r#"
const html = '<title>pay</title>'
  + '<label>Card <input id="card" autocomplete="cc-number"></label>'
  + '<label>Password <input id="pw" type="password"></label>'
  + '<label>Note <input id="note"></label>'
  + '<button id="go">Go</button>';
await page.goto('data:text/html,' + encodeURIComponent(html));
const snapshot = await page.ariaSnapshot();
const note = /textbox "Note" \[ref=(e\d+)\]/.exec(snapshot)?.[1];
const pw = /textbox "Password" \[ref=(e\d+)\]/.exec(snapshot)?.[1];
const card = /textbox "Card" \[ref=(e\d+)\]/.exec(snapshot)?.[1];
return { page, note, pw, card };
"#;

#[tokio::test(flavor = "multi_thread")]
async fn a_neo_restart_keeps_the_tabs_that_took_a_fill() -> TestResult {
    let Some(browser) = browser_or_skip() else {
        return Ok(());
    };
    let mut rig = Rig::start(&browser, CaptchaKey::Absent)?;
    let failure = match time::timeout(TEST_BUDGET, Box::pin(restart(&mut rig))).await {
        Ok(Ok(())) => return Ok(()),
        Ok(Err(error)) => error.to_string(),
        Err(_elapsed) => format!("the drive did not finish within {TEST_BUDGET:?}"),
    };
    Err(format!("{failure}\n{}", rig.logs()).into())
}

/// A visible card number is stored under the default and withheld on the same filled tab
/// under `never`. A restart retains only fills recorded under `never`; an earlier fill under
/// `allowed` becomes unknown and the still-visible card is pictured again.
#[tokio::test(flavor = "multi_thread")]
async fn pictures_follow_the_hosts_fill_pictures_setting() -> TestResult {
    let Some(browser) = browser_or_skip() else {
        return Ok(());
    };
    let mut rig = Rig::start(&browser, CaptchaKey::Absent)?;
    let failure = match time::timeout(TEST_BUDGET, Box::pin(setting(&mut rig))).await {
        Ok(Ok(())) => return Ok(()),
        Ok(Err(error)) => error.to_string(),
        Err(_elapsed) => format!("the drive did not finish within {TEST_BUDGET:?}"),
    };
    Err(format!("{failure}\n{}", rig.logs()).into())
}

async fn setting(rig: &mut Rig) -> TestResult {
    rig.until_ready().await?;
    let client = rig.client().await?;
    let opened = run(&client, PAGE, &json!({ "taskId": TASK })).await?;
    let opened = value(&opened)?;
    let tab = opened
        .pointer("/page/tab")
        .and_then(Value::as_i64)
        .ok_or_else(|| format!("the page's tab: {opened}"))?;
    let refs = |name: &str| {
        opened
            .pointer(&format!("/{name}"))
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| format!("the snapshot names the {name} box: {opened}"))
    };
    let (pw, card) = (refs("pw")?, refs("card")?);

    // A fill under the default records no tab on disk.
    host_fill(rig, tab, &pw, "CANARY-pictures-pw-5").await?;
    assert_eq!(
        script_picture(&client, tab, "screenshot").await?,
        "pictured"
    );
    assert_eq!(script_picture(&client, tab, "pdf").await?, "pictured");
    assert_eq!(
        picture(rig, &click(&client, tab).await?).await?,
        Picture::Stored,
        "row 1"
    );

    // The setting applies to a known filled tab at the next picture.
    for (pictures, expected) in [("never", false), ("allowed", true), ("never", false)] {
        set_pictures(rig, pictures).await?;
        let stored = picture(rig, &click(&client, tab).await?).await? == Picture::Stored;
        assert_eq!(stored, expected, "{pictures}: Neo's own picture");
        let shot = script_picture(&client, tab, "screenshot").await?;
        assert_eq!(
            shot == "pictured",
            expected,
            "{pictures}: a script's screenshot: {shot}"
        );
        let pdf = script_picture(&client, tab, "pdf").await?;
        assert_eq!(
            pdf == "pictured",
            expected,
            "{pictures}: a script's PDF: {pdf}"
        );
    }

    // The camera stores visible values unless the filled-tab setting withholds them.
    set_pictures(rig, "allowed").await?;
    host_fill(rig, tab, &card, CARD).await?;
    assert_eq!(
        picture(rig, &click(&client, tab).await?).await?,
        Picture::Stored,
        "a visible card number is stored under allowed"
    );
    // Row 5 after a restart: no record was written under `allowed`, so the tab is pictured.
    set_pictures(rig, "never").await?;
    assert_eq!(
        picture(rig, &click(&client, tab).await?).await?,
        Picture::Withheld,
        "the same visible card number is withheld on a filled tab under never"
    );
    client.cancel().await?;
    assert_eq!(rig.stop_neo().await?, Some(0), "SIGTERM is a normal stop");
    rig.restart_neo()?;
    rig.until_ready().await?;
    let client = rig.client().await?;
    assert_eq!(
        picture(rig, &click(&client, tab).await?).await?,
        Picture::Stored,
        "row 5: after a restart, a tab filled under `allowed` is pictured"
    );
    assert_eq!(
        script_picture(&client, tab, "screenshot").await?,
        "pictured"
    );
    Ok(())
}

/// Sets the host's `fill.pictures` through the settings route.
async fn set_pictures(rig: &Rig, pictures: &str) -> TestResult {
    let response = rig
        .host(
            "PATCH",
            "/v1/settings",
            Some(HOST_TOKEN),
            Some(&json!({ "fill": { "pictures": pictures } })),
        )
        .await?;
    assert_eq!(response.status, 200, "fill.pictures: {pictures}");
    Ok(())
}

/// The host's check and fill of the box `r#ref` names on `tab`, entered.
async fn host_fill(rig: &Rig, tab: i64, r#ref: &str, text: &str) -> TestResult {
    let checked = host(
        rig,
        &format!("/v1/tabs/{tab}/fill/check"),
        &json!({ "fields": [{ "ref": r#ref }] }),
    )
    .await?;
    let origin = checked
        .pointer("/fields/0/facts/origin")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("the box's origin: {checked}"))?
        .to_owned();
    let filled = host(
        rig,
        &format!("/v1/tabs/{tab}/fill"),
        &json!({ "fields": [{ "ref": r#ref, "value": text, "expect": { "origin": origin } }] }),
    )
    .await?;
    assert_eq!(
        filled.pointer("/outcome"),
        Some(&json!("entered")),
        "{filled}"
    );
    Ok(())
}

/// `code` run on `tab`'s page, as `p`.
fn on_tab(tab: i64, code: &str) -> String {
    format!("const p = context.pages().find(p => p.tabId === {tab}); {code}")
}

/// A click on `tab`, whose picture Neo's camera stores or withholds.
async fn click(client: &Client, tab: i64) -> Result<CallToolResult, Box<dyn Error>> {
    run(
        client,
        &on_tab(tab, "await p.click('#go'); return true;"),
        &json!({}),
    )
    .await
}

/// A script's own picture of `tab` (`how`: `screenshot` or `pdf`): `pictured`, or the refusal's
/// code and message.
async fn script_picture(client: &Client, tab: i64, how: &str) -> Result<String, Box<dyn Error>> {
    let code = on_tab(
        tab,
        &format!(
            "try {{ await p.{how}(); return 'pictured'; }} \
             catch (error) {{ return error.code + ': ' + error.message; }}"
        ),
    );
    let answer = run(client, &code, &json!({})).await?;
    Ok(value(&answer)?.as_str().unwrap_or_default().to_owned())
}

/// A fill under `never` remains withheld across a server restart and navigation.
async fn restart(rig: &mut Rig) -> TestResult {
    rig.until_ready().await?;
    // Today's rule is the host's `fill.pictures: never` (ruling R58), set before the fill.
    set_pictures(rig, "never").await?;
    let client = rig.client().await?;
    let opened = run(&client, PAGE, &json!({ "taskId": TASK })).await?;
    let opened = value(&opened)?;
    let tab = opened
        .pointer("/page/tab")
        .and_then(Value::as_i64)
        .ok_or_else(|| format!("the page's tab: {opened}"))?;
    let note = opened
        .pointer("/note")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("the snapshot names the note box: {opened}"))?;
    host_fill(rig, tab, note, MARKED).await?;
    let click = format!(
        "const p = context.pages().find(p => p.tabId === {tab}); await p.click('#go'); return true;"
    );
    let before = run(&client, &click, &json!({})).await?;
    assert_eq!(
        picture(rig, &before).await?,
        Picture::Withheld,
        "the tab took a fill"
    );

    client.cancel().await?;
    assert_eq!(rig.stop_neo().await?, Some(0), "SIGTERM is a normal stop");
    rig.restart_neo()?;
    rig.until_ready().await?;
    let client = rig.client().await?;
    let after = run(&client, &click, &json!({})).await?;
    assert_eq!(
        picture(rig, &after).await?,
        Picture::Withheld,
        "after the restart Neo still knows the tab took a fill"
    );
    let read = run(
        &client,
        &format!(
            "const p = context.pages().find(p => p.tabId === {tab}); return await p.ariaSnapshot();"
        ),
        &json!({}),
    )
    .await?;
    let printed = value(&read)?.to_string();
    assert!(
        !printed.contains(MARKED) && printed.contains(&"•".repeat(MARKED.chars().count())),
        "the box's mark lives in the page and survives the restart: {printed}"
    );

    // What the record is for: the tab moves to another document, so no box of it holds a mark
    // or a value, and only the record says it took a fill. A click is still withheld, and a
    // script's own screenshot is still refused.
    let moved = run(
        &client,
        &format!(
            "const p = context.pages().find(p => p.tabId === {tab}); \
             await p.goto('data:text/html,' + encodeURIComponent('<button id=\"go\">Go</button>')); \
             await p.click('#go'); return true;"
        ),
        &json!({}),
    )
    .await?;
    assert_eq!(
        picture(rig, &moved).await?,
        Picture::Withheld,
        "on another document, the record alone withholds the picture"
    );
    let refused = script_picture(&client, tab, "screenshot").await?;
    assert!(
        refused.starts_with("private_tab:"),
        "a script's own screenshot is refused after the restart: {refused}"
    );
    let printed = script_picture(&client, tab, "pdf").await?;
    assert!(
        printed.starts_with("private_tab:"),
        "a script's own PDF is refused after the restart: {printed}"
    );
    Ok(())
}

/// What became of a run's write's picture.
#[derive(Debug, PartialEq, Eq)]
enum Picture {
    Stored,
    Withheld,
}

/// The picture of the last operation of `result`'s run, once the camera settled it.
async fn picture(rig: &Rig, result: &CallToolResult) -> Result<Picture, Box<dyn Error>> {
    let run_id = outcome(result)?
        .pointer("/id")
        .and_then(Value::as_str)
        .ok_or("the run's id")?
        .to_owned();
    time::timeout(Duration::from_secs(30), async {
        loop {
            let record = rig
                .host(
                    "GET",
                    &format!("/v1/runs/{run_id}?wait=10"),
                    Some(HOST_TOKEN),
                    None,
                )
                .await?
                .json()?;
            let click = record
                .pointer("/operations")
                .and_then(Value::as_array)
                .and_then(|operations| {
                    operations.iter().rev().find(|operation| {
                        operation
                            .pointer("/name")
                            .and_then(Value::as_str)
                            .is_some_and(|name| name.ends_with("click"))
                    })
                })
                .cloned()
                .ok_or_else(|| format!("the run's click: {record}"))?;
            if click.pointer("/hasScreenshot") == Some(&json!(true)) {
                return Ok::<_, Box<dyn Error>>(Picture::Stored);
            }
            let id = click
                .pointer("/id")
                .and_then(Value::as_str)
                .ok_or("the click's id")?;
            let shot = rig
                .host(
                    "GET",
                    &format!("/v1/operations/{id}/screenshot"),
                    Some(HOST_TOKEN),
                    None,
                )
                .await?;
            let body = shot.json().unwrap_or_default();
            if shot.status == 404
                && body
                    .pointer("/message")
                    .and_then(Value::as_str)
                    .is_some_and(|message| message.contains("withheld"))
            {
                return Ok(Picture::Withheld);
            }
            time::sleep(Duration::from_millis(200)).await;
        }
    })
    .await
    .map_err(|_late| "the camera did not settle the click's picture within 30 s")?
}

/// One POST on the host socket with the host token; its JSON answer, which must be a 200.
async fn host(rig: &Rig, path: &str, body: &Value) -> Result<Value, Box<dyn Error>> {
    let response = rig.host("POST", path, Some(HOST_TOKEN), Some(body)).await?;
    let json = response.json()?;
    if response.status != 200 {
        return Err(format!("{path}: {} {json}", response.status).into());
    }
    Ok(json)
}

/// The script's returned value.
fn value(result: &CallToolResult) -> Result<&Value, Box<dyn Error>> {
    result
        .structured_content
        .as_ref()
        .and_then(|content| content.get("value"))
        .ok_or_else(|| format!("the run returned a value: {result:?}").into())
}
