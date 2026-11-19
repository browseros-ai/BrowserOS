//! The host's cursor setting observed from a page receiving real Neo SDK input.

#[path = "support/browseros_rig.rs"]
pub mod browseros_rig;

use std::error::Error;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use browseros_cdp::{CdpClient, ConnectOptions, SessionId};
use browseros_rig::{
    CaptchaKey, Client, HOST_TOKEN, Rig, TestResult, browser_or_skip, get, outcome, run,
};
use neo_takeover::testing::FakeXvnc;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use tokio::time;

type ResultOf<T> = Result<T, Box<dyn Error>>;
const FIXTURE: &str = include_str!("agent_cursor_fixture.mjs");
const CALLS: &[&str] = &[
    "locator",
    "ref",
    "locator.dblclick",
    "mouse.click",
    "mouse.dblclick",
    "locator.options",
    "mouse.clickCount",
];

struct Browser {
    rig: Rig,
    client: Client,
}

impl Browser {
    async fn start(env: &[(&str, &str)]) -> ResultOf<Option<Self>> {
        let Some(bin) = browser_or_skip() else {
            return Ok(None);
        };
        let rig = Rig::start_with(&bin, CaptchaKey::Absent, env)?;
        rig.until_ready().await?;
        let client = rig.client().await?;
        Ok(Some(Self { rig, client }))
    }

    async fn script(&self, code: &str) -> ResultOf<Value> {
        let result = run(
            &self.client,
            &format!("{FIXTURE}\n{code}"),
            &json!({ "taskId":"cursor" }),
        )
        .await?;
        if outcome(&result)?.get("status") != Some(&json!("ok")) {
            return Err(format!("{:?}", result.content).into());
        }
        result
            .structured_content
            .and_then(|content| content.get("value").cloned())
            .ok_or_else(|| "script returned no value".into())
    }

    async fn host(&self, method: &str, path: &str, body: Option<&Value>) -> ResultOf<Value> {
        let response = self.rig.host(method, path, Some(HOST_TOKEN), body).await?;
        if response.status != 200 {
            return Err(format!("{method} {path}: {}", response.text()).into());
        }
        response.json()
    }

    async fn cursor(&self, setting: &str) -> TestResult {
        let state = self
            .host(
                "PATCH",
                "/v1/settings",
                Some(&json!({"input":{"cursor":setting}})),
            )
            .await?;
        if state.pointer("/settings/input/cursor") != Some(&json!(setting))
            || state.get("unknown") != Some(&json!([]))
            || state.get("unknownValues") != Some(&json!([]))
        {
            return Err(format!("PATCH input.cursor={setting} was not applied: {state}").into());
        }
        Ok(())
    }

    // A trace capture would also briefly hide the cursor during the native screenshot control.
    async fn disable_trace_captures(&self) -> TestResult {
        self.host(
            "PATCH",
            "/v1/settings",
            Some(&json!({"trace":{"screenshots":"off"}})),
        )
        .await?;
        Ok(())
    }

    async fn matrix(&self, on: bool) -> TestResult {
        let mode = if on { "on" } else { "off" };
        let mut failures = Vec::new();
        for name in CALLS {
            match self
                .script(&format!("return await matrix({name:?}, {on})"))
                .await
            {
                Ok(seen) => record(&format!("{mode}/{name}"), &seen),
                Err(error) => failures.push(format!("{mode}/{name}: {error}")),
            }
        }
        if failures.is_empty() {
            Ok(())
        } else {
            Err(failures.join("\n").into())
        }
    }
}

#[expect(
    clippy::print_stderr,
    reason = "the live report includes page events and per-call wall time"
)]
fn record(case: &str, value: &Value) {
    eprintln!("CURSOR {case} {value}");
}

#[tokio::test(flavor = "multi_thread")]
async fn default_off() -> TestResult {
    let Some(b) = Browser::start(&[]).await? else {
        return Ok(());
    };
    b.matrix(false).await
}

#[tokio::test(flavor = "multi_thread")]
async fn enabled_clicks() -> TestResult {
    let Some(b) = Browser::start(&[]).await? else {
        return Ok(());
    };
    let initial = b.host("GET", "/v1/settings", None).await?;
    assert_eq!(
        initial.pointer("/defaults/input/cursor"),
        Some(&json!("off"))
    );
    assert_eq!(
        initial.pointer("/settings/input/cursor"),
        Some(&json!("off"))
    );
    b.cursor("on").await?;
    b.matrix(true).await
}

#[tokio::test(flavor = "multi_thread")]
async fn no_visuals_for_other_actions() -> TestResult {
    let Some(b) = Browser::start(&[]).await? else {
        return Ok(());
    };
    b.cursor("on").await?;
    for name in ["trial", "hover", "fill", "check", "move", "refused"] {
        let seen = b
            .script(&format!("return await withoutCursor({name:?})"))
            .await?;
        record(name, &seen);
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn moving_target() -> TestResult {
    let Some(b) = Browser::start(&[]).await? else {
        return Ok(());
    };
    b.cursor("on").await?;
    record("moving", &b.script("return await movingTarget()").await?);
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn framed_clicks() -> TestResult {
    let Some(b) = Browser::start(&[]).await? else {
        return Ok(());
    };
    b.disable_trace_captures().await?;
    b.cursor("on").await?;
    for transform in ["none", "scale(1.5)"] {
        let hit = b
            .script(&format!(
                "const hit = await framed({transform:?}); return {{tab:page.tabId, hit}}"
            ))
            .await?;
        let pixels = native_target_pixels(&b, hit.get("tab").ok_or("tab id")?).await?;
        record(
            transform,
            &json!({"events": hit.get("hit"), "native": pixels}),
        );
        assert!(
            pixels
                .get("blue")
                .and_then(Value::as_u64)
                .is_some_and(|n| n > 0),
            "top cursor must reach the frame button under {transform}: {pixels}"
        );
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn phone_at_double_scale() -> TestResult {
    // Only the RFB screen is a fixture. The host applies the real browser's mobile viewport,
    // including the 2x preview zoom used when translating input coordinates.
    let screen = FakeXvnc::start("1600x1000".parse()?).await?;
    let Some(b) = Browser::start(&[("CLAW_XVNC_ADDR", &screen.addr().to_string())]).await? else {
        return Ok(());
    };
    b.disable_trace_captures().await?;
    let tab = b.script("await fresh(); return page.tabId").await?;
    let fit = b
        .host(
            "POST",
            &format!("/v1/tabs/{tab}/fit"),
            Some(&json!({"width":366,"height":700,"scale":2})),
        )
        .await?;
    let id = fit.get("id").and_then(Value::as_str).ok_or("fit id")?;
    record("phone-fit", &fit);
    let phone = async {
        for on in [false, true] {
            if on {
                b.cursor("on").await?;
                b.script("await fresh(); return true").await?;
            }
            let seen = b.script(&format!("return await phonePress({on})")).await?;
            let pixels = native_target_pixels(&b, &tab).await?;
            record(
                if on { "phone-on" } else { "phone-off" },
                &json!({"press": seen, "pixels": pixels}),
            );
            assert_eq!(
                pixels
                    .get("blue")
                    .and_then(Value::as_u64)
                    .ok_or("blue pixel count")?
                    > 0,
                on,
                "cursor blue within 8 CSS pixels of the phone press: {pixels}"
            );
        }
        Ok::<_, Box<dyn Error>>(())
    }
    .await;
    b.host("POST", &format!("/v1/fits/{id}/restore"), None)
        .await?;
    assert_eq!(
        fit.get("viewer"),
        Some(&json!({"width":366,"height":700,"scale":2})),
        "{fit}"
    );
    phone?;
    Ok(())
}

const CAPTURE: &str = r"
  await fresh()
  const hit = await landing(() => page.locator('#target').click(), { on })
  const visible = () => page.evaluate(selector => {
    const host = document.querySelector(selector)
    return !!host && host.checkVisibility({visibilityProperty:true, opacityProperty:true})
  }, selector)
  check(await visible() === on, 'cursor visibility before screenshot', hit)
  const png = await page.screenshot({type:'png', clip:{x:55,y:40,width:140,height:105}})
  check(await visible() === on, 'cursor visibility after screenshot', hit)
  return {tab:page.tabId, hit, png:Array.from(png)}
";

// A raw browser capture of the landing point must contain the blue cursor. This distinguishes
// a genuinely clean SDK screenshot from a renderer that only appended an empty host element.
async fn native_target_pixels(b: &Browser, tab: &Value) -> ResultOf<Value> {
    let cdp = CdpClient::connect(ConnectOptions::new(b.rig.cdp_port())).await?;
    let listed = cdp.send("Browser.getTabs", json!({}), None).await?;
    let target = listed
        .get("tabs")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .find(|item| item.get("tabId") == Some(tab))
        .and_then(|item| item.get("targetId"))
        .ok_or("no browser target for the captured tab")?;
    let attached = cdp
        .send(
            "Target.attachToTarget",
            json!({"targetId":target,"flatten":true}),
            None,
        )
        .await?;
    let session = SessionId::from(
        attached
            .get("sessionId")
            .and_then(Value::as_str)
            .ok_or("capture session")?,
    );
    let raw = cdp
        .send(
            "Page.captureScreenshot",
            json!({"format":"png"}),
            Some(&session),
        )
        .await?;
    cdp.disconnect().await;
    let png = raw.get("data").ok_or("native PNG")?;
    b.script(&format!("return await targetPixels({png})")).await
}

#[tokio::test(flavor = "multi_thread")]
async fn screenshot_hides_visible_cursor() -> TestResult {
    let Some(b) = Browser::start(&[]).await? else {
        return Ok(());
    };
    b.disable_trace_captures().await?;
    let off = b.script(&format!("const on = false; {CAPTURE}")).await?;
    b.cursor("on").await?;
    let on = b.script(&format!("const on = true; {CAPTURE}")).await?;
    let pixels = native_target_pixels(&b, on.get("tab").ok_or("tab id")?).await?;
    assert!(
        pixels
            .get("blue")
            .and_then(Value::as_u64)
            .is_some_and(|count| count > 0),
        "the native capture should show cursor blue at the landing point: {pixels}"
    );
    let bytes = |value: &Value| -> ResultOf<Vec<u8>> {
        Ok(serde_json::from_value(
            value.get("png").cloned().ok_or("missing PNG")?,
        )?)
    };
    let (off_png, on_png) = (bytes(&off)?, bytes(&on)?);
    assert!(off_png.starts_with(b"\x89PNG\r\n\x1a\n"));
    assert!(
        off_png == on_png,
        "screenshots differ (off {} bytes, on {})",
        off_png.len(),
        on_png.len()
    );
    record(
        "capture",
        &json!({"native":pixels,"identicalPngBytes":on_png.len(),"off":off.get("hit"),"on":on.get("hit")}),
    );
    Ok(())
}

// The observer changes the title at cursor insertion. Read it on this rig's debugging port
// while the SDK click is pending; a delay from launching an MCP run could race its cold start.
async fn take_during_glide(b: &Browser, tab: &Value) -> ResultOf<(Value, u64)> {
    time::timeout(Duration::from_secs(5), async {
        loop {
            let reply = get(b.rig.cdp_port(), "/json/list").await?;
            let (_, body) = reply
                .split_once("\r\n\r\n")
                .ok_or("DevTools HTTP response")?;
            let tabs: Vec<Value> = serde_json::from_str(body)?;
            if tabs
                .iter()
                .any(|t| t.get("title") == Some(&json!("cursor-in-glide")))
            {
                return Ok::<_, Box<dyn Error>>(());
            }
            time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .map_err(|error| format!("waiting for the cursor-in-glide title: {error}"))??;
    let requested = u64::try_from(SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis())?;
    let taken = b
        .host("POST", "/v1/takeovers", Some(&json!({"tabId":tab})))
        .await?;
    Ok((taken, requested))
}

#[tokio::test(flavor = "multi_thread")]
async fn owner_takes_over_during_glide() -> TestResult {
    let Some(b) = Browser::start(&[]).await? else {
        return Ok(());
    };
    b.cursor("on").await?;
    for (name, action) in [
        ("locator", "page.locator('#target').click()"),
        ("mouse", "page.mouse.click(120,84)"),
        ("already-held", "page.locator('#target').click()"),
    ] {
        let tab = b.script(r"
          await fresh()
          await page.evaluate(() => { window.onCursorAppearance = () => { window.appearedAt = Date.now(); document.title = 'cursor-in-glide' } })
          return page.tabId
        ").await?;
        let program = format!(
            r"
          try {{ await {action}; return 'landed' }}
          catch (e) {{ return {{code:e.code, owner:e.owner, delivery:e.delivery}} }}
        "
        );
        let (click, taken, requested) = if name == "already-held" {
            let taken = b
                .host("POST", "/v1/takeovers", Some(&json!({"tabId": tab})))
                .await?;
            (b.script(&program).await, taken, None)
        } else {
            let (click, taken) = tokio::join!(b.script(&program), take_during_glide(&b, &tab));
            let (taken, requested) = taken?;
            (click, taken, Some(requested))
        };
        let id = taken
            .get("id")
            .and_then(Value::as_str)
            .ok_or("takeover id")?;
        let release = b
            .host("POST", &format!("/v1/takeovers/{id}/release"), None)
            .await?;
        assert_eq!(release.get("state"), Some(&json!("released")));
        let page = b.script("return await page.evaluate(() => ({log:window.cursorEvents, appearedAt:window.appearedAt ?? null}))").await?;
        let log = page.get("log").ok_or("page event log")?;
        let latency = requested
            .zip(page.get("appearedAt").and_then(Value::as_u64))
            .map(|(request, appeared)| request.saturating_sub(appeared));
        let result = click?;
        record(
            &format!("takeover/{name}"),
            &json!({"result":result,"log":log,"requestAfterOverlayMs":latency}),
        );
        assert_eq!(
            result,
            json!({"code":"owner_has_control","owner":"host","delivery":"not_sent"})
        );
        if requested.is_some() {
            assert!(
                latency.is_some_and(|ms| ms <= 150),
                "takeover request after cursor appearance: {latency:?} ms"
            );
        }
        let log = log.as_array().ok_or("event log")?;
        assert_eq!(
            log.len(),
            usize::from(name != "already-held"),
            "refused {name}: {log:?}"
        );
        assert!(
            log.iter().all(|e| e.get("kind") == Some(&json!("overlay"))),
            "{log:?}"
        );
    }
    Ok(())
}

// Navigation rendezvous: the host changes the setting between two clicks in one SDK run.
// Answer only after PATCH returns, so the second click deterministically sees the new value.
async fn turn_off_between_clicks(b: &Browser, listener: &TcpListener) -> TestResult {
    let (mut stream, _) = time::timeout(Duration::from_secs(10), listener.accept()).await??;
    let mut request = String::new();
    BufReader::new(&mut stream).read_line(&mut request).await?;
    if !request.starts_with("GET /switch ") {
        return Err(format!("expected the agent's /switch navigation: {request}").into());
    }
    b.cursor("off").await?;
    stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn live_setting_changes_between_clicks() -> TestResult {
    let Some(b) = Browser::start(&[]).await? else {
        return Ok(());
    };
    b.cursor("on").await?;
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let url = format!("http://{}/switch", listener.local_addr()?);
    let script = format!(
        r"
      const before = await matrix('locator', true)
      await page.goto({url:?})
      const after = await matrix('locator', false)
      return {{before,after}}
    "
    );
    let (value, ()) = tokio::try_join!(b.script(&script), turn_off_between_clicks(&b, &listener))?;
    record("toggle", &value);
    Ok(())
}
