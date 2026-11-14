//! A cover in a page above a sign-in frame, through the host's routes on a real BrowserOS
//! (protected-fill ruling R60, row 10): a banner in the top page over a password box in a
//! cross-site frame. Nothing covers the box inside its own document, and before R60 the route
//! typed there; a host reported a password typed under such a banner.
//!
//! ```text
//!  agent (MCP run) ─ the page, the frame's password box's ref from the snapshot
//!  host ─ POST /v1/tabs/{tab}/fill/check ─ refused box_covered, the page above's sentence
//!       ─ POST /v1/tabs/{tab}/fill ─ refused before any key, nothing typed, not sent
//!  agent ─ the box's value: empty
//! ```
//!
//! Like `e2e_browseros.rs`, it runs only when `CLAW_SDK_E2E_BROWSER` names a BrowserOS binary.

#[path = "support/browseros_rig.rs"]
pub mod browseros_rig;

use std::error::Error;
use std::net::{Ipv4Addr, Ipv6Addr};
use std::time::Duration;

use browseros_rig::{CaptchaKey, HOST_TOKEN, Rig, TestResult, browser_or_skip, run};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::time;

/// The drive's whole budget.
const TEST_BUDGET: Duration = Duration::from_secs(120);
/// The host's task: every run of it acts on the tab the first opened.
const TASK: &str = "private-cover";
/// The value the host would type. Nothing of it may reach the box.
const PASSWORD: &str = "CANARY-cover-pw-3K";
/// The refusal's fixed sentence for a cover in a page above the box's frame.
const ABOVE: &str = "Nothing was typed. Something in the page above the frame that holds this box \
                     takes the pointer where the box is, such as a banner, a dialog or a header. \
                     Dismiss it, or scroll until the box is clear of it, and try again.";

/// Ruling R60 through the routes: the check and the fill both answer `box_covered` with the page
/// above's sentence, and the box stays empty. Fails with the pages above left unasked
/// (`look_op.rs`'s `pages_above`): the fill types the password.
#[tokio::test(flavor = "multi_thread")]
async fn a_banner_above_a_sign_in_frame_refuses_the_hosts_fill() -> TestResult {
    let Some(browser) = browser_or_skip() else {
        return Ok(());
    };
    let rig = Rig::start(&browser, CaptchaKey::Absent)?;
    let failure = match time::timeout(TEST_BUDGET, Box::pin(drive(&rig))).await {
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
    let opened = run(
        &client,
        &format!(
            "await page.goto('http://127.0.0.1:{port}/page');\n\
             const snapshot = await page.ariaSnapshot();\n\
             const pw = /textbox \"Password\" \\[ref=(f\\d+e\\d+)\\]/.exec(snapshot)?.[1];\n\
             return {{ page, pw, snapshot }};"
        ),
        &json!({ "taskId": TASK }),
    )
    .await?;
    let opened = opened
        .structured_content
        .as_ref()
        .and_then(|content| content.get("value"))
        .ok_or("the run returned a value")?;
    let tab = opened
        .pointer("/page/tab")
        .and_then(Value::as_i64)
        .ok_or_else(|| format!("the page's tab: {opened}"))?;
    let pw = opened
        .pointer("/pw")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("the snapshot names the frame's password box: {opened}"))?;
    let origin = format!("http://localhost:{port}");

    let checked = host(
        rig,
        &format!("/v1/tabs/{tab}/fill/check"),
        &json!({ "fields": [{ "ref": pw, "expect": { "origin": origin } }] }),
    )
    .await?;
    assert_eq!(
        checked.pointer("/fields/0/refusal/code"),
        Some(&json!("box_covered")),
        "{checked}"
    );
    assert_eq!(
        checked.pointer("/fields/0/refusal/message"),
        Some(&json!(ABOVE)),
        "{checked}"
    );

    let filled = host(
        rig,
        &format!("/v1/tabs/{tab}/fill"),
        &json!({ "fields": [{ "ref": pw, "value": PASSWORD, "expect": { "origin": origin } }] }),
    )
    .await?;
    assert_eq!(
        filled.pointer("/outcome"),
        Some(&json!("refused")),
        "{filled}"
    );
    assert_eq!(
        filled.pointer("/delivery"),
        Some(&json!("not_sent")),
        "{filled}"
    );
    assert_eq!(filled.pointer("/typed"), Some(&json!([0])), "{filled}");
    assert_eq!(
        filled.pointer("/error/code"),
        Some(&json!("box_covered")),
        "{filled}"
    );
    assert_eq!(
        filled.pointer("/error/message"),
        Some(&json!(ABOVE)),
        "{filled}"
    );

    let read = run(
        &client,
        &format!(
            "const p = context.pages().find(p => p.tabId === {tab});\n\
             return await p.frameLocator('iframe').locator('#pw').inputValue();"
        ),
        &json!({ "taskId": TASK }),
    )
    .await?;
    let value = read
        .structured_content
        .as_ref()
        .and_then(|content| content.get("value"));
    assert_eq!(value, Some(&json!("")), "the box stays empty");
    Ok(())
}

/// One host call, answered 200.
async fn host(rig: &Rig, path: &str, body: &Value) -> Result<Value, Box<dyn Error>> {
    let response = rig.host("POST", path, Some(HOST_TOKEN), Some(body)).await?;
    let json = response.json()?;
    if response.status != 200 {
        return Err(format!("{path}: {} {json}", response.status).into());
    }
    Ok(json)
}

/// Two sites on one port, each its own renderer under site isolation: the page on `127.0.0.1`,
/// its sign-in frame on `localhost`.
async fn serve_sites() -> Result<u16, Box<dyn Error>> {
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

/// `/page`: a sign-in frame on the other site under a cookie banner fixed over the top of the
/// page. Anything else: the frame's password box, which nothing covers in its own document.
fn page(path: &str, port: u16) -> String {
    match path {
        "/page" => format!(
            "<title>shop</title><style>body {{ margin: 0 }} \
             iframe {{ margin: 120px 0 0 24px; width: 360px; height: 120px; border: 0 }} \
             .banner {{ position: fixed; left: 0; right: 0; top: 0; height: 280px; z-index: 9; \
             background: #ffe680 }}</style>\
             <iframe title=\"Sign-in frame\" src=\"http://localhost:{port}/box\"></iframe>\
             <div class=banner>We use cookies. Accept?</div>"
        ),
        _ => "<title>box</title><style>body { margin: 0 } input { margin: 10px; width: 260px; \
              height: 34px }</style>\
              <input id=pw type=password autocomplete=current-password aria-label=\"Password\">"
            .to_owned(),
    }
}
