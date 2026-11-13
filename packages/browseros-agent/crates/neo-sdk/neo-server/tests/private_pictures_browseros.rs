//! Neo's stored pictures and private boxes, end to end on a real BrowserOS (protected-fill ruling
//! R45; design section 10.3).
//!
//! ```text
//!  a script fills a password box ─ a click ─ pictured (a password box paints bullets)
//!  a script fills a card box the page declares (cc-number) ─ a click ─ withheld: a frame shows
//!    a private box's value painted in clear
//!  the script empties it ─ a click ─ pictured again
//!
//!  the host fills a plain box through the route ─ a click ─ withheld (the tab took a fill)
//!  Neo restarts, the browser and its tab kept ─ a click ─ still withheld (the record beside the
//!    trace) ─ the snapshot still prints the box as bullets (its mark lives in the page)
//! ```
//!
//! Like `e2e_browseros.rs`, it runs only when `CLAW_SDK_E2E_BROWSER` names a BrowserOS binary.

#[path = "support/browseros_rig.rs"]
pub mod browseros_rig;

use std::error::Error;
use std::time::Duration;

use browseros_rig::{CaptchaKey, HOST_TOKEN, Rig, TestResult, browser_or_skip, outcome, run};
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
return { page, note };
"#;

#[tokio::test(flavor = "multi_thread")]
async fn a_script_s_card_number_keeps_neos_pictures_off_while_it_shows() -> TestResult {
    let Some(browser) = browser_or_skip() else {
        return Ok(());
    };
    let rig = Rig::start(&browser, CaptchaKey::Absent)?;
    within(&rig, Box::pin(card_number(&rig))).await
}

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

/// Runs `drive` within the test's budget, with both processes' logs on a failure.
async fn within(rig: &Rig, drive: impl Future<Output = TestResult>) -> TestResult {
    let failure = match time::timeout(TEST_BUDGET, drive).await {
        Ok(Ok(())) => return Ok(()),
        Ok(Err(error)) => error.to_string(),
        Err(_elapsed) => format!("the drive did not finish within {TEST_BUDGET:?}"),
    };
    Err(format!("{failure}\n{}", rig.logs()).into())
}

async fn card_number(rig: &Rig) -> TestResult {
    rig.until_ready().await?;
    let client = rig.client().await?;
    run(&client, PAGE, &json!({ "taskId": TASK })).await?;
    let step = |code: &'static str| {
        let client = &client;
        async move { run(client, code, &json!({ "taskId": TASK })).await }
    };
    let password =
        step("await page.fill('#pw', 'CANARY-pw-1'); await page.click('#go'); return true;")
            .await?;
    assert_eq!(
        picture(rig, &password).await?,
        Picture::Stored,
        "a password box paints bullets: its page is pictured"
    );
    let card =
        step("await page.fill('#card', '4242424242424242'); await page.click('#go'); return true;")
            .await?;
    assert_eq!(
        picture(rig, &card).await?,
        Picture::Withheld,
        "a card number painted in clear withholds the picture"
    );
    let emptied =
        step("await page.fill('#card', ''); await page.click('#go'); return true;").await?;
    assert_eq!(
        picture(rig, &emptied).await?,
        Picture::Stored,
        "an emptied card box lets Neo picture the page again"
    );
    assert!(
        !rig.neo_log()?.contains(CARD),
        "the card number is in no log line of Neo's"
    );
    Ok(())
}

async fn restart(rig: &mut Rig) -> TestResult {
    rig.until_ready().await?;
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
    let checked = host(
        rig,
        &format!("/v1/tabs/{tab}/fill/check"),
        &json!({ "fields": [{ "ref": note }] }),
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
        &json!({ "fields": [{ "ref": note, "value": MARKED, "expect": { "origin": origin } }] }),
    )
    .await?;
    assert_eq!(
        filled.pointer("/outcome"),
        Some(&json!("entered")),
        "{filled}"
    );
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
    let shot = run(
        &client,
        &format!(
            "const p = context.pages().find(p => p.tabId === {tab}); \
             try {{ await p.screenshot(); return 'pictured'; }} \
             catch (error) {{ return error.code + ': ' + error.message; }}"
        ),
        &json!({}),
    )
    .await?;
    let refused = value(&shot)?.as_str().unwrap_or_default().to_owned();
    assert!(
        refused.starts_with("private_tab:"),
        "a script's own screenshot is refused after the restart: {refused}"
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

/// A card box the page declares, inside a CLOSED shadow root that the page's own script fills.
const CLOSED_PAGE: &str = r#"
const html = '<title>closed</title>'
  + '<div id="host"></div>'
  + '<button id="fill">Fill</button><button id="empty">Empty</button>'
  + '<button id="later">Later</button><button id="go">Go</button>'
  + '<script>'
  + 'const box = (label) => \'<input autocomplete="cc-number" aria-label="\' + label + \'">\';'
  + 'const root = document.getElementById("host").attachShadow({ mode: "closed" });'
  + 'root.innerHTML = box("Card number");'
  + 'document.getElementById("fill").onclick = () => { root.querySelector("input").value = "4242424242424242"; };'
  + 'document.getElementById("empty").onclick = () => { root.querySelector("input").value = ""; };'
  + 'document.getElementById("later").onclick = () => { const h = document.createElement("div"); document.body.appendChild(h); const r = h.attachShadow({ mode: "closed" }); r.innerHTML = box("Second card"); r.querySelector("input").value = "5555555555554444"; };'
  + '</scr' + 'ipt>';
await page.goto('data:text/html,' + encodeURIComponent(html));
return { page };
"#;

/// Ruling R45's named limit, on the whole stack (measured by the design's author): until a
/// snapshot of the document has walked a closed shadow root, Neo does not know the boxes inside
/// it, so a picture taken then can show what one holds, whoever typed it. A snapshot that walks
/// the root closes it for that document; a closed root attached after that walk is unknown
/// again until the next one.
#[tokio::test(flavor = "multi_thread")]
async fn a_box_in_a_closed_root_is_known_once_a_snapshot_walked_it() -> TestResult {
    let Some(browser) = browser_or_skip() else {
        return Ok(());
    };
    let rig = Rig::start(&browser, CaptchaKey::Absent)?;
    within(&rig, Box::pin(closed_root(&rig))).await
}

async fn closed_root(rig: &Rig) -> TestResult {
    rig.until_ready().await?;
    let client = rig.client().await?;
    run(&client, CLOSED_PAGE, &json!({ "taskId": TASK })).await?;
    let steps: [(&str, &str, Picture); 6] = [
        (
            "the box empty, no snapshot yet",
            "await page.click('#go'); return true;",
            Picture::Stored,
        ),
        (
            "the page puts a card number in, no snapshot yet: the limit",
            "await page.click('#fill'); await page.click('#go'); return true;",
            Picture::Stored,
        ),
        (
            "after the script's own snapshot walked the root",
            "await page.ariaSnapshot(); await page.click('#go'); return true;",
            Picture::Withheld,
        ),
        (
            "the page empties the box",
            "await page.click('#empty'); await page.click('#go'); return true;",
            Picture::Stored,
        ),
        (
            "a second closed root attached after that snapshot, its box filled: the limit again",
            "await page.click('#later'); await page.click('#go'); return true;",
            Picture::Stored,
        ),
        (
            "after the next snapshot",
            "await page.ariaSnapshot(); await page.click('#go'); return true;",
            Picture::Withheld,
        ),
    ];
    for (what, code, expected) in steps {
        let result = run(&client, code, &json!({ "taskId": TASK })).await?;
        assert_eq!(picture(rig, &result).await?, expected, "{what}");
    }
    Ok(())
}
