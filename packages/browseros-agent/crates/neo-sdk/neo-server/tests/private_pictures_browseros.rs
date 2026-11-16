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

/// Ruling R58 on a real BrowserOS, rows 1, 3, 5 and 6 of its spec, on one tab that took the
/// host's fill. Row 1, the default: a script's `page.screenshot()` and `page.pdf()` work, and
/// Neo's camera stores a click's picture. Row 3, the switch at once: under `never` the click is
/// withheld and the script's pictures refused, under `allowed` pictured again, and under `never`
/// refused again (row 5: a fill made under `allowed` is refused in this run once the host says
/// never). Row 6, R45 under `allowed`: a card number the host typed privately, painted in clear,
/// withholds Neo's picture until the box is emptied. Row 5 after a restart of Neo, still under
/// `never`: the tab is pictured, for no record was written under `allowed`. Fails with the
/// setting left out of the entry's answer (`withholds_picture`: row 1 refused), with the
/// settings watcher left out (row 3 pictured under `never`), or with the record written whatever
/// the setting (row 5 withheld after the restart).
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

    // Row 1: the default. A password box paints bullets, so R45 keeps out of the way.
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

    // Row 3, the switch at once, and back; row 5 in this run: the fill came under `allowed`.
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

    // Row 6: R45 under `allowed`.
    set_pictures(rig, "allowed").await?;
    host_fill(rig, tab, &card, CARD).await?;
    assert_eq!(
        picture(rig, &click(&client, tab).await?).await?,
        Picture::Withheld,
        "row 6: a card number typed privately, painted in clear"
    );
    let empty = on_tab(
        tab,
        "await p.fill('#card', ''); await p.click('#go'); return true;",
    );
    let emptied = run(&client, &empty, &json!({})).await?;
    assert_eq!(
        picture(rig, &emptied).await?,
        Picture::Stored,
        "row 6: emptied"
    );

    // Row 5 after a restart: no record was written under `allowed`, so the tab is pictured.
    set_pictures(rig, "never").await?;
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

/// Ruling R45's named limit, on the whole stack (measured by the design's author): a value the
/// page's own script writes into a box inside a closed shadow root fires no event, so until a
/// snapshot of the document has walked that root, or typing has reached it or its box held the
/// focus (the camera's walk on demand, ruling R65), Neo does not know the box, and a picture
/// taken then can show what it holds. A snapshot that walks the root closes it for that
/// document; a closed root attached after that walk is unknown again until the next one.
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
