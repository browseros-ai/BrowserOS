//! The host's private typing end to end on a real BrowserOS: the route, the door, `host.fill`
//! and the real entry (protected-fill design, sections 4 to 9).
//!
//! ```text
//!  agent (MCP run) ─ a sign-in form, its boxes' refs from the snapshot
//!  host (host.sock) ─ POST /v1/tabs/{tab}/fill/check ─ each box's facts
//!                   ─ POST /v1/tabs/{tab}/fill ─ the values typed, read back ─ a repeat
//!  agent ─ the page's title: a hash of what its boxes hold, never the values
//!  agent ─ a click on the tab: no picture, its screenshot 404 "withheld"; a script's own
//!          screenshot of the tab refused (rulings R15 and R46)
//!  Neo stopped ─ its state folder (trace.db, -wal, -shm, the fill's record, its logs at trace
//!               level) and its stderr scanned
//! ```
//!
//! The page proves what it received through a hash in its title, so no script ever carries a
//! value and the scan can demand that none is anywhere. Like `e2e_browseros.rs`, it runs only
//! when `CLAW_SDK_E2E_BROWSER` names a BrowserOS binary.

#[path = "support/browseros_rig.rs"]
pub mod browseros_rig;

use std::error::Error;
use std::fs;
use std::path::Path;
use std::time::Duration;

use browseros_rig::{
    CaptchaKey, Client, HOST_TOKEN, Rig, TestResult, browser_or_skip, outcome, run,
};
use rmcp::model::CallToolResult;
use serde_json::{Value, json};
use tokio::time;

/// The drive's whole budget.
const TEST_BUDGET: Duration = Duration::from_secs(240);
/// The host's task: every run of it acts on the tab the first opened.
const TASK: &str = "private-typing";
/// The values the host types. Neither may be in any file or log line of Neo's.
const USER: &str = "CANARY-live-user-4Q";
const PASSWORD: &str = "CANARY-live-pw-7Z9x";

/// The form, built in the script. Its title is a hash of what both boxes hold after each input.
const FORM: &str = r#"
const html = '<title>sign in</title>'
  + '<form><label>User <input id="user" autocomplete="username"></label>'
  + '<label>Password <input id="pw" type="password" autocomplete="current-password"></label></form>'
  + '<script>'
  + 'const hash = (text) => { let x = 5381; for (const c of text) { x = ((x * 33) ^ c.codePointAt(0)) >>> 0; } return x; };'
  + 'const show = () => { document.title = "h:" + hash(user.value + "|" + pw.value); };'
  + 'user.addEventListener("input", show); pw.addEventListener("input", show);'
  + '</script>';
await page.goto('data:text/html,' + encodeURIComponent(html));
const snapshot = await page.ariaSnapshot();
const user = /textbox "User" \[ref=(e\d+)\]/.exec(snapshot)?.[1];
const pw = /textbox "Password" \[ref=(e\d+)\]/.exec(snapshot)?.[1];
return { page, user, pw, snapshot };
"#;

#[tokio::test(flavor = "multi_thread")]
async fn a_host_types_a_sign_in_privately_on_a_real_browseros() -> TestResult {
    let Some(browser) = browser_or_skip() else {
        return Ok(());
    };
    // At trace level, so every log line Neo can write is written and scanned: the design's
    // "Server logs" row (section 8, ruling R37).
    let mut rig = Rig::start_with(&browser, CaptchaKey::Absent, &[("CLAW_LOG", "trace")])?;
    let failure = match time::timeout(TEST_BUDGET, drive(&mut rig)).await {
        Ok(Ok(())) => return Ok(()),
        Ok(Err(error)) => error.to_string(),
        Err(_elapsed) => format!("the drive did not finish within {TEST_BUDGET:?}"),
    };
    Err(format!("{failure}\n{}", rig.logs()).into())
}

async fn drive(rig: &mut Rig) -> TestResult {
    rig.until_ready().await?;
    let client = rig.client().await?;
    let form = run(&client, FORM, &json!({ "taskId": TASK })).await?;
    let form = value(&form)?;
    let tab = form
        .pointer("/page/tab")
        .and_then(Value::as_i64)
        .ok_or_else(|| format!("the form's tab: {form}"))?;
    let (Some(user), Some(pw)) = (at(form, "/user").as_str(), at(form, "/pw").as_str()) else {
        return Err(format!("the snapshot names both boxes: {form}").into());
    };

    // The check: each box's facts, which the host approves and sends back as `expect`.
    let checked = host(
        rig,
        &format!("/v1/tabs/{tab}/fill/check"),
        &json!({ "taskId": TASK, "fields": [{ "ref": user }, { "ref": pw }] }),
    )
    .await?;
    let origin = at(&checked, "/fields/0/facts/origin")
        .as_str()
        .ok_or_else(|| format!("the check reads the box's origin: {checked}"))?
        .to_owned();
    assert_eq!(
        at(&checked, "/fields/1/facts/type"),
        "password",
        "{checked}"
    );
    assert!(at(&checked, "/fields/0/refusal").is_null(), "{checked}");

    // The fill, and a repeat of its requestId.
    let body = json!({
        "requestId": "typing-1",
        "taskId": TASK,
        "fields": [
            { "ref": user, "value": USER, "expect": { "origin": origin } },
            { "ref": pw, "value": PASSWORD, "expect": { "origin": origin, "type": "password" } }
        ]
    });
    let answer = host(rig, &format!("/v1/tabs/{tab}/fill"), &body).await?;
    assert_eq!(at(&answer, "/outcome"), "entered", "{answer}");
    assert_eq!(at(&answer, "/delivery"), "done", "{answer}");
    assert_eq!(
        at(&answer, "/typed"),
        &json!([USER.chars().count(), PASSWORD.chars().count()])
    );
    let repeat = host(rig, &format!("/v1/tabs/{tab}/fill"), &body).await?;
    assert_eq!(repeat, answer, "a repeat gets the stored answer");

    // The page received exactly the values: its title is their hash.
    let read = run(
        &client,
        "return await page.title();",
        &json!({ "taskId": TASK }),
    )
    .await?;
    assert_eq!(
        value(&read)?,
        &json!(format!("h:{}", page_hash(&format!("{USER}|{PASSWORD}")))),
        "the boxes hold the host's values"
    );

    no_picture_after_the_fill(rig, &client).await?;

    // The record: one protected op, the counts, no picture, no page address.
    let run_id = at(&answer, "/runId").as_str().ok_or("a runId")?;
    let record = rig
        .host(
            "GET",
            &format!("/v1/runs/{run_id}?wait=10"),
            Some(HOST_TOKEN),
            None,
        )
        .await?
        .json()?;
    let operation = at(&record, "/operations/0");
    assert_eq!(at(operation, "/name"), "host.fill", "{record}");
    assert_eq!(at(operation, "/kind"), "protected");
    assert_eq!(at(operation, "/hasScreenshot"), false);
    assert!(at(operation, "/page/url").is_null(), "{operation}");

    client.cancel().await?;
    assert_eq!(rig.stop_neo().await?, Some(0), "SIGTERM is a normal stop");
    let state = rig
        .trace_path()
        .parent()
        .and_then(Path::parent)
        .ok_or("the state folder")?
        .to_path_buf();
    let log = rig.neo_log()?;
    let logs = state.join("logs");
    assert!(
        fs::read_dir(&logs)?
            .filter_map(Result::ok)
            .filter_map(|entry| fs::read_to_string(entry.path()).ok())
            .any(|text| text.contains("TRACE")),
        "Neo ran at trace level, so the scan below covers every line it can write"
    );
    for canary in [USER, PASSWORD] {
        let files = files_holding(&state, canary)?;
        assert!(files.is_empty(), "{canary} is in {files:?}");
        assert!(!log.contains(canary), "{canary} is in Neo's log");
        for text in [answer.to_string(), record.to_string(), checked.to_string()] {
            assert!(!text.contains(canary), "{canary} is in {text}");
        }
    }
    Ok(())
}

/// After the fill, no picture of its tab (rulings R15 and R46): a script's later click there
/// records none, and its screenshot answers 404 for that reason; a script's own screenshot of
/// the tab is refused. Both ask the entry's real mark of the tab, through the app's wiring.
async fn no_picture_after_the_fill(rig: &Rig, client: &Client) -> TestResult {
    let clicked = run(
        client,
        "await page.click('#user'); return true;",
        &json!({ "taskId": TASK }),
    )
    .await?;
    let click_run = at(outcome(&clicked)?, "/id")
        .as_str()
        .ok_or("the click's run id")?
        .to_owned();
    let withheld = time::timeout(Duration::from_secs(10), async {
        loop {
            let record: Value = rig
                .host(
                    "GET",
                    &format!("/v1/runs/{click_run}?wait=10"),
                    Some(HOST_TOKEN),
                    None,
                )
                .await?
                .json()?;
            let click = record
                .pointer("/operations")
                .and_then(Value::as_array)
                .and_then(|operations| {
                    operations.iter().find(|operation| {
                        at(operation, "/name")
                            .as_str()
                            .is_some_and(|name| name.ends_with("click"))
                    })
                })
                .cloned()
                .ok_or_else(|| format!("the click's operation: {record}"))?;
            assert_eq!(at(&click, "/hasScreenshot"), false, "{click}");
            let id = at(&click, "/id")
                .as_str()
                .ok_or("the click's id")?
                .to_owned();
            let shot = rig
                .host(
                    "GET",
                    &format!("/v1/operations/{id}/screenshot"),
                    Some(HOST_TOKEN),
                    None,
                )
                .await?;
            let status = shot.status;
            let body = shot.json()?;
            if status == 404
                && at(&body, "/message")
                    .as_str()
                    .is_some_and(|message| message.contains("withheld"))
            {
                return Ok::<_, Box<dyn Error>>(());
            }
            time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await;
    withheld.map_err(|_late| "the click's picture was not withheld within 10 s")??;
    let shot = run(
        client,
        "try { await page.screenshot(); return 'pictured'; } catch (error) { return String(error.message); }",
        &json!({ "taskId": TASK }),
    )
    .await?;
    assert!(
        value(&shot)?
            .as_str()
            .is_some_and(|message| message.contains("took a private fill")
                && message.contains("Trying again will not succeed")),
        "a script's own screenshot of the tab is refused: {shot:?}"
    );

    Ok(())
}

/// A sign-in whose boxes each sit under a sibling div that draws the field's name and takes the
/// pointer, which is no `<label>` (ruling R55). Built in the script; its title is a hash of what
/// both boxes hold after each input.
const DRAWN_FORM: &str = r#"
const field = (id, name, type) => '<div style="position:relative;width:260px;height:40px;margin:0 0 16px">'
  + '<input id="' + id + '" type="' + type + '" aria-label="' + name + '" style="width:100%;height:100%;box-sizing:border-box">'
  + '<div style="position:absolute;inset:0;display:flex;align-items:center;padding:0 8px">' + name + '</div></div>';
const html = '<title>sign in</title><form>' + field('user', 'User', 'text') + field('pw', 'Password', 'password') + '</form>'
  + '<script>'
  + 'const hash = (text) => { let x = 5381; for (const c of text) { x = ((x * 33) ^ c.codePointAt(0)) >>> 0; } return x; };'
  + 'const show = () => { document.title = "h:" + hash(user.value + "|" + pw.value); };'
  + 'user.addEventListener("input", show); pw.addEventListener("input", show);'
  + '</script>';
await page.goto('data:text/html,' + encodeURIComponent(html));
const snapshot = await page.ariaSnapshot();
const user = /textbox "User" \[ref=(e\d+)\]/.exec(snapshot)?.[1];
const pw = /textbox "Password" \[ref=(e\d+)\]/.exec(snapshot)?.[1];
return { page, user, pw, snapshot };
"#;

/// Rulings R55 and R56 through the route on a real BrowserOS, with nothing set by the host: the
/// check call passes both boxes under their fields' drawn names, the fill enters both, the page
/// holds exactly the values, and the record notes both fields as entered under their drawing,
/// with no value drawn. Fails with the drawing's branch left out of the look (`box_covered`).
#[tokio::test(flavor = "multi_thread")]
async fn a_host_types_into_boxes_under_drawn_labels() -> TestResult {
    let Some(browser) = browser_or_skip() else {
        return Ok(());
    };
    let mut rig = Rig::start_with(&browser, CaptchaKey::Absent, &[])?;
    let failure = match time::timeout(TEST_BUDGET, drive_drawn(&mut rig)).await {
        Ok(Ok(())) => return Ok(()),
        Ok(Err(error)) => error.to_string(),
        Err(_elapsed) => format!("the drive did not finish within {TEST_BUDGET:?}"),
    };
    Err(format!("{failure}\n{}", rig.logs()).into())
}

async fn drive_drawn(rig: &mut Rig) -> TestResult {
    rig.until_ready().await?;
    let client = rig.client().await?;
    let form = run(&client, DRAWN_FORM, &json!({ "taskId": TASK })).await?;
    let form = value(&form)?;
    let tab = form
        .pointer("/page/tab")
        .and_then(Value::as_i64)
        .ok_or_else(|| format!("the form's tab: {form}"))?;
    let (Some(user), Some(pw)) = (at(form, "/user").as_str(), at(form, "/pw").as_str()) else {
        return Err(format!("the snapshot names both boxes: {form}").into());
    };
    let check = json!({ "taskId": TASK, "fields": [{ "ref": user }, { "ref": pw }] });
    let checked = host(rig, &format!("/v1/tabs/{tab}/fill/check"), &check).await?;
    for at_box in ["/fields/0/refusal", "/fields/1/refusal"] {
        assert!(at(&checked, at_box).is_null(), "{checked}");
    }
    let origin = at(&checked, "/fields/0/facts/origin")
        .as_str()
        .ok_or_else(|| format!("the check reads the box's origin: {checked}"))?
        .to_owned();
    let body = json!({
        "taskId": TASK,
        "fields": [
            { "ref": user, "value": USER, "expect": { "origin": origin } },
            { "ref": pw, "value": PASSWORD, "expect": { "origin": origin, "type": "password" } }
        ]
    });
    let answer = host(rig, &format!("/v1/tabs/{tab}/fill"), &body).await?;
    assert_eq!(at(&answer, "/outcome"), "entered", "{answer}");
    let read = run(
        &client,
        "return await page.title();",
        &json!({ "taskId": TASK }),
    )
    .await?;
    assert_eq!(
        value(&read)?,
        &json!(format!("h:{}", page_hash(&format!("{USER}|{PASSWORD}")))),
        "the boxes hold the host's values"
    );
    let run_id = at(&answer, "/runId").as_str().ok_or("a runId")?;
    let record = rig
        .host(
            "GET",
            &format!("/v1/runs/{run_id}?wait=10"),
            Some(HOST_TOKEN),
            None,
        )
        .await?
        .json()?;
    assert_eq!(
        at(&record, "/operations/0/result/drawings"),
        &json!([{ "field": 0, "drawn": false }, { "field": 1, "drawn": false }]),
        "{record}"
    );
    client.cancel().await?;
    Ok(())
}
/// The value at `pointer`, or null.
fn at<'a>(value: &'a Value, pointer: &str) -> &'a Value {
    value.pointer(pointer).unwrap_or(&Value::Null)
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

/// The page's own hash of what its boxes hold, as its script computes it.
fn page_hash(text: &str) -> u32 {
    text.chars().fold(5381_u32, |hash, character| {
        hash.wrapping_mul(33) ^ u32::from(character)
    })
}

/// Every file under `dir` whose bytes hold `value`, as UTF-8 or UTF-16.
fn files_holding(dir: &Path, value: &str) -> Result<Vec<String>, Box<dyn Error>> {
    let forms = [
        value.as_bytes().to_vec(),
        value
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect::<Vec<u8>>(),
    ];
    let mut found = Vec::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(path) = pending.pop() {
        if path.is_dir() {
            for entry in fs::read_dir(&path)? {
                pending.push(entry?.path());
            }
        } else if path.is_file() {
            let bytes = fs::read(&path)?;
            if forms.iter().any(|form| {
                bytes
                    .windows(form.len())
                    .any(|window| window == form.as_slice())
            }) {
                found.push(path.display().to_string());
            }
        }
    }
    Ok(found)
}

/// The run's returned value.
fn value(result: &CallToolResult) -> Result<&Value, Box<dyn Error>> {
    result
        .structured_content
        .as_ref()
        .and_then(|content| content.get("value"))
        .ok_or_else(|| format!("the run returned a value: {result:?}").into())
}
