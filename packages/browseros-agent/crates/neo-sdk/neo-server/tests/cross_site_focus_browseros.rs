//! Focus in out-of-process frames on a real BrowserOS, through MCP `run`: on an agent tab that is
//! not its window's front tab when the run starts, `focus()` on a box in a cross-site frame
//! leaves the frame's document focused, so `toBeFocused()` and `document.hasFocus()` hold there,
//! and `fill()` and typed keys land.
//!
//! Each renderer process keeps its own focus state. Core sets focus emulation on an agent tab's
//! page session, as upstream Playwright 1.64.1 does (`crPage.ts`, main frame only), and that
//! reaches only the main frame's process. A cross-site frame's process reads the page focus the
//! browser gives the tab, which a background tab does not have: there `hasFocus()` stays false
//! after `focus()`, and Playwright's own check with it (`isFocused` reads
//! `ownerDocument.hasFocus()`). What gives the frame its focus here is the tab coming forward:
//! `focus()` takes the tab's input hold, which first brings the tab forward
//! (`neo-playwright` `input::hold`, neo-server's `AgentFront`, #3522). With that activation off,
//! every cross-site case below fails and the same-origin one still passes (claw-sdk
//! `lead/verify/oopif-focus/NOTES.md`). The activation's TODO says what must replace it first.
//!
//! ```text
//!  the owner's tab (Target.createTarget), outside any script
//!  per case, in a task of its own (a fresh tab that no input has reached):
//!    run: goto a page with the frames ─ add one after load ─ navigate one cross-site
//!    the owner's tab to the front (Browser.activateTab) ─ Browser.getTabs: the agent's is behind
//!    run: focus() ─ the frame's activeElement and hasFocus() ─ toBeFocused() ─ fill(), type
//! ```
//!
//! Like `e2e_browseros.rs`, it runs only when `CLAW_SDK_E2E_BROWSER` names a BrowserOS binary.

#[path = "support/browseros_rig.rs"]
pub mod browseros_rig;

use std::error::Error;
use std::net::{Ipv4Addr, Ipv6Addr};
use std::time::Duration;

use browseros_cdp::{CdpClient, ConnectOptions};
use browseros_rig::{CaptchaKey, Client, Rig, TestResult};
use rmcp::model::CallToolResult;
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio::time;

/// The whole drive; see `e2e_browseros.rs` for why the budget sits inside the test.
const TEST_BUDGET: Duration = Duration::from_secs(240);

/// Run 1, on the task's tab: the page, then a cross-site frame the page adds after load, and a
/// same-origin frame it navigates cross-site, which swaps that frame's renderer and session.
const OPEN: &str = r"
await page.goto(args.page);
await page.evaluate((src) => new Promise((done) => {
  const frame = document.createElement('iframe');
  frame.id = 'late';
  frame.addEventListener('load', () => done(), { once: true });
  frame.src = src;
  document.body.append(frame);
}), args.crossSite);
await page.evaluate((src) => new Promise((done) => {
  const frame = document.getElementById('moved');
  frame.addEventListener('load', () => done(), { once: true });
  frame.src = src;
}), args.crossSite);
return { page };
";

/// One case's run: the box at the end of `args.frames`, focused as a model would, then read the
/// way Playwright's `toBeFocused` reads it.
const FOCUS: &str = r"
const box = args.frames.reduce((scope, id) => scope.frameLocator('#' + id), page).locator('#box');
await box.focus();
const frame = await box.evaluate((e) => ({
  active: e.ownerDocument.activeElement === e,
  hasFocus: e.ownerDocument.hasFocus(),
}));
let toBeFocused = true;
try {
  await expect(box).toBeFocused({ timeout: 2000 });
} catch (error) {
  toBeFocused = error.message;
}
await box.fill('4242');
await page.keyboard.type('77');
return { frame, toBeFocused, value: await box.inputValue() };
";

/// One case: where the box is.
struct FocusCase {
    name: &'static str,
    /// The frame ids from the page down to the box's document.
    frames: &'static [&'static str],
}

const CASES: &[FocusCase] = &[
    FocusCase {
        name: "a cross-site frame",
        frames: &["cross"],
    },
    FocusCase {
        name: "a cross-site frame inside a cross-site frame",
        frames: &["nest", "inner"],
    },
    FocusCase {
        name: "a cross-site frame added after load",
        frames: &["late"],
    },
    FocusCase {
        name: "a frame navigated cross-site after load",
        frames: &["moved"],
    },
    FocusCase {
        name: "a same-origin frame",
        frames: &["same"],
    },
];

#[tokio::test(flavor = "multi_thread")]
async fn focus_in_a_cross_site_frame_holds_on_a_tab_behind_the_owners() -> TestResult {
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
    let port = serve_sites().await?;
    rig.until_ready().await?;
    let client = rig.client().await?;
    let args = json!({
        "page": format!("http://127.0.0.1:{port}/outer"),
        "crossSite": format!("http://localhost:{port}/inner"),
    });

    // The owner's tab, opened outside any script as the owner would, and put in front before
    // every case, so no case starts with the agent's tab drawn or focused.
    let cdp = CdpClient::connect(ConnectOptions::new(rig.cdp_port())).await?;
    let owner = cdp
        .send("Target.createTarget", json!({ "url": "about:blank" }), None)
        .await?;
    let owner_target = owner
        .get("targetId")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("a new tab has a target: {owner}"))?
        .to_owned();
    let owner_tab = tab_of(&cdp, &owner_target).await?;

    let mut failed = Vec::new();
    for (case, FocusCase { name, frames }) in CASES.iter().enumerate() {
        // A task, and so a tab, of its own per case: once any input reached a tab, the browser
        // keeps that tab's page focus even behind another tab, so a case after one that typed
        // would pass without being brought forward.
        let meta = json!({ "taskId": format!("cross-site-focus-{case}") });
        let opened = returned(&script(&client, OPEN, &args, &meta).await?)?;
        let tab = opened
            .pointer("/page/tab")
            .and_then(Value::as_i64)
            .ok_or_else(|| format!("a page leaves a run as its brand: {opened}"))?;
        cdp.send("Browser.activateTab", json!({ "tabId": owner_tab }), None)
            .await?;
        if active(&cdp, tab).await? {
            return Err(format!("{name}: the agent's tab is still in front").into());
        }
        let seen = returned(&script(&client, FOCUS, &json!({ "frames": frames }), &meta).await?)?;
        let expected = json!({
            "frame": { "active": true, "hasFocus": true },
            "toBeFocused": true,
            "value": "424277",
        });
        if seen != expected {
            failed.push(format!("{name}: {seen}"));
        }
    }
    client.cancel().await?;
    assert!(
        failed.is_empty(),
        "focus() in a frame of a tab behind the owner's leaves the frame's document focused, \
         toBeFocused() passes, and the keys land; these did not:\n{}",
        failed.join("\n")
    );
    Ok(())
}

/// `code` with `const args = …` before it, as a host passes a run's inputs.
async fn script(
    client: &Client,
    code: &str,
    args: &Value,
    meta: &Value,
) -> Result<CallToolResult, Box<dyn Error>> {
    browseros_rig::run(client, &format!("const args = {args};\n{code}"), meta).await
}

/// BrowserOS's tab id for a target.
async fn tab_of(cdp: &CdpClient, target: &str) -> Result<i64, Box<dyn Error>> {
    tabs(cdp)
        .await?
        .iter()
        .find(|tab| tab.get("targetId").and_then(Value::as_str) == Some(target))
        .and_then(|tab| tab.get("tabId").and_then(Value::as_i64))
        .ok_or_else(|| format!("no tab has target {target}").into())
}

/// Whether `tab` is its window's active tab.
async fn active(cdp: &CdpClient, tab: i64) -> Result<bool, Box<dyn Error>> {
    tabs(cdp)
        .await?
        .iter()
        .find(|listed| listed.get("tabId").and_then(Value::as_i64) == Some(tab))
        .and_then(|listed| listed.get("isActive").and_then(Value::as_bool))
        .ok_or_else(|| format!("tab {tab} is listed").into())
}

async fn tabs(cdp: &CdpClient) -> Result<Vec<Value>, Box<dyn Error>> {
    let listed = cdp.send("Browser.getTabs", json!({}), None).await?;
    Ok(listed
        .get("tabs")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default())
}

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

/// Three sites on one port, each its own renderer under site isolation: the page on `127.0.0.1`,
/// its cross-site frames on `localhost`, and the frame nested in one of them on `[::1]`.
async fn serve_sites() -> Result<u16, Box<dyn Error>> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let v4 = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await?;
    let port = v4.local_addr()?.port();
    let v6 = TcpListener::bind((Ipv6Addr::LOCALHOST, port)).await?;
    for listener in [v4, v6] {
        tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                tokio::spawn(async move {
                    let mut request = [0_u8; 4096];
                    let read = stream.read(&mut request).await.unwrap_or(0);
                    let head = String::from_utf8_lossy(request.get(..read).unwrap_or_default());
                    let body = page(head.split(' ').nth(1).unwrap_or_default(), port);
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = stream.write_all(response.as_bytes()).await;
                });
            }
        });
    }
    Ok(port)
}

/// `/outer`: a box of its own and four frames (two cross-site, one same-origin, and one that run
/// 1 navigates cross-site). `/nest`: a cross-site frame holding one from a third site. Anything
/// else: the box.
fn page(path: &str, port: u16) -> String {
    let cross = format!("http://localhost:{port}");
    match path {
        "/outer" => format!(
            "<title>outer</title><input id=top>\
             <iframe id=cross src=\"{cross}/inner\"></iframe>\
             <iframe id=nest src=\"{cross}/nest\" width=440 height=200></iframe>\
             <iframe id=same src=\"/inner\"></iframe>\
             <iframe id=moved src=\"/inner\"></iframe>"
        ),
        "/nest" => format!("<iframe id=inner src=\"http://[::1]:{port}/inner\"></iframe>"),
        _ => "<title>inner</title><input id=box aria-label=\"Card number\">".to_owned(),
    }
}
