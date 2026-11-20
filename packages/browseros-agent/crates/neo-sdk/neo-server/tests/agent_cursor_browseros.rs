//! The host's cursor setting observed from a page receiving real Neo SDK input.

#[path = "support/browseros_rig.rs"]
pub mod browseros_rig;

use std::error::Error;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
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
    for name in [
        "trial", "hover", "fill", "check", "move", "missing", "covered",
    ] {
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
async fn take_during_glide(b: &Browser, tab: &Value, title: &str) -> ResultOf<(Value, u64)> {
    time::timeout(Duration::from_secs(5), async {
        loop {
            let reply = get(b.rig.cdp_port(), "/json/list").await?;
            let (_, body) = reply
                .split_once("\r\n\r\n")
                .ok_or("DevTools HTTP response")?;
            let tabs: Vec<Value> = serde_json::from_str(body)?;
            if tabs
                .iter()
                .any(|t| t.get("title").and_then(Value::as_str) == Some(title))
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
        let title = format!("cursor-in-glide-{name}");
        let tab = b.script(&format!(r"
          await fresh()
          await page.evaluate(title => {{
            window.onCursorAppearance = () => {{window.appearedAt = Date.now(); document.title = title}}
          }}, {title:?})
          return page.tabId
        ")).await?;
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
            let (click, taken) =
                tokio::join!(b.script(&program), take_during_glide(&b, &tab, &title));
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

// The fixture's same-origin fetch completes only after the host PATCH. Both clicks use the
// same tab, document, locator and SDK run, with no delay for the old cursor's idle timer.
async fn turn_off_between_clicks(b: &Browser, listener: &TcpListener) -> TestResult {
    loop {
        let (mut stream, _) = time::timeout(Duration::from_secs(10), listener.accept()).await??;
        let mut request = String::new();
        BufReader::new(&mut stream).read_line(&mut request).await?;
        let switch = request.starts_with("GET /switch ");
        if switch {
            b.cursor("off").await?;
        }
        stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await?;
        if switch {
            return Ok(());
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn live_setting_changes_between_clicks() -> TestResult {
    let Some(b) = Browser::start(&[]).await? else {
        return Ok(());
    };
    b.cursor("on").await?;
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let url = format!("http://{}/fixture", listener.local_addr()?);
    let script = format!(
        r"
      await page.goto({url:?})
      await page.setContent(html)
      await observe(false)
      const target = page.locator('#target')
      const before = await landing(() => target.click(), {{on:true}})
      await page.evaluate(async selector => {{
        if (!(await fetch('/switch')).ok) throw new Error('host switch failed')
        // A connected previous cursor can be reused without another insertion. Remove it and
        // move the button so any stale on-setting would have to create a new visible cursor.
        document.querySelector(selector)?.remove()
        document.querySelector('#target').style.left = '230px'
        window.cursorEvents.length = 0
      }}, selector)
      const after = await landing(() => target.click(), {{on:false,x:270}})
      return {{before,after}}
    "
    );
    let (value, ()) = tokio::try_join!(b.script(&script), turn_off_between_clicks(&b, &listener))?;
    record("toggle", &value);
    Ok(())
}

// Install the observer in the fixture markup. An extra SDK call between goto and click would
// give goto's background capture time to finish. mouse.move starts a fresh capture after idle.
const TRACE_CLICKS: &str = r#"
const html = `<!doctype html><style>
  body {background:white} button {position:absolute;top:24px;width:40px;height:32px;
    padding:0;border:0;background:#777;outline:0} #first {left:24px} #second {left:96px}
  </style><button id="first"></button><button id="second"></button>
  <script>
    window.log = []
    new MutationObserver(records => { for (const r of records) {
      for (const n of r.addedNodes) if (n.matches?.('browseros-agent-pointer[data-browseros-agent-pointer]'))
        log.push(['added',performance.now()])
      for (const n of r.removedNodes) if (n.matches?.('browseros-agent-pointer[data-browseros-agent-pointer]'))
        log.push(['removed',performance.now()])
    }}).observe(document,{childList:true,subtree:true})
    addEventListener('click', e => log.push([e.isTrusted ? e.target.id : 'untrusted',performance.now()]))
  </script>`
await page.goto('data:text/html,' + encodeURIComponent(html))
await page.locator('#first').click()
await new Promise(resolve => setTimeout(resolve, 4600))
await page.mouse.move(2,2)
await page.locator('#second').click()
return await page.evaluate(() => window.log)
"#;

// Both gray buttons and the cursor's two glide paths fit inside these first 160 x 90 CSS px.
// JPEG quantization can soften cursor blue, so compare channels rather than exact RGB values.
const TRACE_PIXELS: &str = r"
return await page.evaluate(async shots => {
  const counts = []
  for (const shot of shots) {
    const image = new Image()
    image.src = 'data:image/jpeg;base64,' + shot
    await image.decode()
    const canvas = document.createElement('canvas')
    canvas.width = 160
    canvas.height = 90
    const ctx = canvas.getContext('2d')
    ctx.drawImage(image,0,0,innerWidth,innerHeight)
    const {data} = ctx.getImageData(0,0,160,90)
    let blue = 0, gray = 0
    for (let i=0; i<data.length; i+=4) {
      const r = data[i], g = data[i+1], b = data[i+2]
      if (b-r > 65 && b-g > 45 && b > 120) ++blue
      if (Math.abs(r-119)<12 && Math.abs(g-119)<12 && Math.abs(b-119)<12) ++gray
    }
    counts.push({blue,gray})
  }
  return counts
}, shots)
";

async fn trace_jpegs(b: &Browser, id: &str) -> ResultOf<(Vec<String>, Vec<String>)> {
    let trace = b
        .host(
            "GET",
            &format!("/v1/runs/{id}?wait=30&fields=summary"),
            None,
        )
        .await?;
    assert_eq!(
        trace.get("complete"),
        Some(&json!(true)),
        "pending trace {id}"
    );
    let ops = trace
        .get("operations")
        .and_then(Value::as_array)
        .ok_or("trace operations")?;
    let mut names = Vec::new();
    let mut shots = Vec::new();
    for op in ops {
        if op.get("hasScreenshot") != Some(&json!(true)) {
            continue;
        }
        let op_id = op.get("id").and_then(Value::as_str).ok_or("trace op id")?;
        let jpeg = b
            .rig
            .host(
                "GET",
                &format!("/v1/operations/{op_id}/screenshot"),
                Some(HOST_TOKEN),
                None,
            )
            .await?;
        assert_eq!(
            (jpeg.status, jpeg.header("content-type")),
            (200, Some("image/jpeg")),
            "{op_id}"
        );
        assert!(
            jpeg.body.starts_with(&[0xff, 0xd8]) && jpeg.body.ends_with(&[0xff, 0xd9]),
            "{op_id}"
        );
        shots.push(STANDARD.encode(jpeg.body));
        names.push(
            op.get("name")
                .and_then(Value::as_str)
                .ok_or("trace op name")?
                .to_owned(),
        );
    }
    Ok((names, shots))
}

#[tokio::test(flavor = "multi_thread")]
async fn background_trace_jpegs_hide_new_and_recreated_cursors() -> TestResult {
    let Some(b) = Browser::start(&[]).await? else {
        return Ok(());
    };
    let settings = b.host("GET", "/v1/settings", None).await?;
    assert_eq!(
        settings.pointer("/settings/trace/screenshots"),
        Some(&json!("perOperation"))
    );
    b.cursor("on").await?;
    // Bind the task's tab before measuring. No first-run blank-tab screenshot is in our sample.
    b.script("await page.goto('about:blank'); return page.tabId")
        .await?;
    let mut leaks = Vec::new();
    for iteration in 1..=10 {
        let reply = run(&b.client, TRACE_CLICKS, &json!({"taskId":"cursor"})).await?;
        let meta = outcome(&reply)?;
        assert_eq!(
            meta.get("status"),
            Some(&json!("ok")),
            "{:?}",
            reply.content
        );
        let events = reply
            .structured_content
            .as_ref()
            .and_then(|v| v.get("value"))
            .and_then(Value::as_array)
            .ok_or("fixture event log")?;
        let kinds: Vec<_> = events
            .iter()
            .filter_map(|e| e.pointer("/0").and_then(Value::as_str))
            .collect();
        assert_eq!(
            kinds,
            ["added", "first", "removed", "added", "second"],
            "{events:?}"
        );
        let id = meta
            .get("id")
            .and_then(Value::as_str)
            .ok_or("trace run id")?;
        let (names, shots) = trace_jpegs(&b, id).await?;
        let writes: Vec<_> = names
            .iter()
            .map(String::as_str)
            .filter(|name| matches!(*name, "page.goto" | "locator.click" | "mouse.move"))
            .collect();
        assert_eq!(
            writes,
            ["page.goto", "locator.click", "mouse.move", "locator.click"],
            "missing trace JPEG in run {id}: {names:?}"
        );
        let pixels = b
            .script(&format!("const shots = {};\n{TRACE_PIXELS}", json!(shots)))
            .await?;
        let counts = pixels.as_array().ok_or("decoded JPEGs")?;
        assert_eq!(counts.len(), names.len());
        record(
            "trace-jpegs",
            &json!({"iteration":iteration,"run":id,"ops":names,"pixels":counts,"events":events}),
        );
        for (name, count) in names.iter().zip(counts) {
            assert!(
                count
                    .get("gray")
                    .and_then(Value::as_u64)
                    .is_some_and(|n| n > 1500),
                "fixture buttons missing in {id} {name}: {count}"
            );
            if count
                .get("blue")
                .and_then(Value::as_u64)
                .ok_or("blue pixels")?
                > 0
            {
                leaks.push(format!("iteration {iteration}, run {id}, {name}: {count}"));
            }
        }
    }
    assert!(
        leaks.is_empty(),
        "trace JPEGs contained cursor blue:\n{}",
        leaks.join("\n")
    );
    Ok(())
}

// BEGIN standalone phone diagnostic (the report runs this module against the plain base).
mod phone_repro {
    use std::error::Error;
    use std::sync::Arc;
    use std::time::Duration;

    use browseros_cdp::{CdpClient, ConnectOptions};
    use browseros_core::{BrowserSession, BrowserSessionHooks, TabId};
    use neo_takeover::testing::FakeXvnc;
    use rmcp::model::{CallToolRequestParams, MetaObject, RequestMetaObject};
    use serde_json::{Value, json};
    use tokio::time::{self, Instant};

    use super::browseros_rig::{
        CaptchaKey, Client, HOST_TOKEN, RUN_META, Rig, TestResult, browser_or_skip, outcome, run,
    };

    // Retain every pointer, touch and compatibility mouse event, including a release after
    // the SDK has already returned. All event times use the page's clock from the same mark.
    const PAGE: &str = r#"
      const html = `<!doctype html><meta name="viewport" content="width=device-width,initial-scale=1">
        <style>#target{position:absolute;left:80px;top:64px;width:80px;height:40px}</style>
        <button id="target">Target</button>
        <script>
        window.events=[]
        for (const kind of ['pointerover','pointerenter','pointermove','pointerdown','pointerup','pointercancel','pointerout','pointerleave',
          'touchstart','touchmove','touchend','touchcancel','mouseover','mouseenter','mousemove','mousedown','mouseup','click','dblclick','contextmenu','mouseout','mouseleave']) {
          document.addEventListener(kind,e=>{
            if(window.start===undefined)return
            const point=e.changedTouches?.[0]||e
            events.push({t:Math.round(performance.now()-start),kind,id:e.target.id||e.target.nodeName,
              trusted:e.isTrusted, pointerType:e.pointerType??null, x:point.clientX,y:point.clientY,
              button:e.button??null,buttons:e.buttons??null,touches:e.touches?.length??null})
          },{capture:true,passive:true})
        }
        </script>`
      await page.goto('data:text/html,'+encodeURIComponent(html)); return page.tabId
    "#;
    const META: &str = "phone-repro";

    async fn value(client: &Client, code: &str) -> Result<Value, Box<dyn Error>> {
        let result = run(client, code, &json!({"taskId":META})).await?;
        if outcome(&result)?.get("status") != Some(&json!("ok")) {
            return Err(format!("{:?}", result.content).into());
        }
        result
            .structured_content
            .and_then(|v| v.get("value").cloned())
            .ok_or_else(|| "missing value".into())
    }

    // Raw mouse.click has no per-op timeout. Give the run a real deadline so the SDK itself
    // reports the outcome and drains/releases its input before we read the page in a new run.
    async fn sdk_click(client: &Client, action: &str) -> Result<Value, Box<dyn Error>> {
        let code = format!(
            "try {{ await {action}; return 'resolved' }} catch (e) {{return {{code:e.code, name:e.name, delivery:e.delivery, message:e.message}}}}"
        );
        let Value::Object(args) = json!({"code":code,"timeout":4000}) else {
            return Err("run arguments".into());
        };
        let mut meta = MetaObject::new();
        meta.insert(RUN_META.to_owned(), json!({"taskId":META}));
        let mut params = CallToolRequestParams::new("run").with_arguments(args);
        params.meta = Some(RequestMetaObject(meta));
        let start = Instant::now();
        let result = time::timeout(Duration::from_secs(10), client.call_tool(params)).await??;
        Ok(
            json!({"wallMs":start.elapsed().as_millis(), "status":outcome(&result)?.get("status"),
            "response":result.structured_content,"text":result.content}),
        )
    }

    // Independent CDP control. Submit release on a timer even if the press has no reply yet.
    // The physical point (240,168) corresponds to CSS (120,84) at 2x preview zoom.
    async fn ack(rig: &Rig, tab: i64) -> Result<Value, Box<dyn Error>> {
        let cdp = Arc::new(CdpClient::connect(ConnectOptions::new(rig.cdp_port())).await?);
        let browser = BrowserSession::new(
            Arc::<CdpClient>::clone(&cdp),
            BrowserSessionHooks::default(),
        );
        let pages = browser.pages.resolve_tab_ids(&[TabId(tab)]).await?;
        let page = pages.get(&TabId(tab)).cloned().ok_or("phone tab")?;
        let session = browser.pages.get_session(page).await?.session;
        let start = Instant::now();
        let dispatch = |kind: &'static str, buttons: u8, force: f64| {
            let session = &session;
            async move {
                let sent = start.elapsed().as_millis();
                let reply = time::timeout(Duration::from_secs(6), session.send_value("Input.dispatchMouseEvent",
                    json!({"type":kind,"button":"left","buttons":buttons,"clickCount":1,"modifiers":0,"force":force,"x":240,"y":168}))).await;
                json!({"sentMs":sent,"finishedMs":start.elapsed().as_millis(),"reply":format!("{reply:?}")})
            }
        };
        let (press, release) = tokio::join!(dispatch("mousePressed", 1, 0.5), async {
            time::sleep(Duration::from_secs(3)).await;
            dispatch("mouseReleased", 0, 0.0).await
        });
        cdp.disconnect().await;
        Ok(json!({"mousePressed":press,"mouseReleased":release}))
    }

    #[expect(
        clippy::print_stderr,
        reason = "the ignored reproduction exports full input and protocol evidence"
    )]
    pub(super) async fn measure(settings: Option<&Value>, methods: &[&str]) -> TestResult {
        let Some(binary) = browser_or_skip() else {
            return Ok(());
        };
        for method in methods {
            let screen = FakeXvnc::start("1600x1000".parse()?).await?;
            let rig = Rig::start_with(
                &binary,
                CaptchaKey::Absent,
                &[("CLAW_XVNC_ADDR", &screen.addr().to_string())],
            )?;
            rig.until_ready().await?;
            if let Some(settings) = settings {
                let state = rig
                    .host("PATCH", "/v1/settings", Some(HOST_TOKEN), Some(settings))
                    .await?
                    .json()?;
                assert_eq!(state.get("unknown"), Some(&json!([])), "{state}");
            }
            let client = rig.client().await?;
            let tab = value(&client, PAGE).await?.as_i64().ok_or("tab id")?;
            let fit = rig
                .host(
                    "POST",
                    &format!("/v1/tabs/{tab}/fit"),
                    Some(HOST_TOKEN),
                    Some(&json!({"width":366,"height":700,"scale":2})),
                )
                .await?
                .json()?;
            let id = fit.get("id").and_then(Value::as_str).ok_or("fit id")?;
            let measured = async {
                let metrics = value(
                    &client,
                    r"
                    return await page.evaluate(() => {
                      window.start = performance.now()
                      return [innerWidth, innerHeight, devicePixelRatio, navigator.maxTouchPoints]
                    })",
                )
                .await?;
                assert_eq!(metrics, json!([366, 700, 2, 5]));
                let outcome = match *method {
                    "locator" => {
                        sdk_click(&client, "page.locator('#target').click({timeout:3000})").await?
                    }
                    "mouse" => sdk_click(&client, "page.mouse.click(120,84)").await?,
                    "cdp" => ack(&rig, tab).await?,
                    _ => return Err("unknown probe".into()),
                };
                time::sleep(Duration::from_millis(600)).await;
                let log = value(
                    &client,
                    r"
                    return await page.evaluate(() => ({
                      observedUntilMs: Math.round(performance.now()-start), events: window.events
                    }))",
                )
                .await?;
                eprintln!(
                    "PHONE {method} {}",
                    json!({"outcome":outcome,"metrics":metrics,"page":log})
                );
                Ok::<_, Box<dyn Error>>(())
            }
            .await;
            let release = rig
                .host(
                    "POST",
                    &format!("/v1/fits/{id}/restore"),
                    Some(HOST_TOKEN),
                    None,
                )
                .await?;
            assert_eq!(release.status, 200, "{}", release.text());
            measured?;
        }
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    #[ignore = "manual 2x phone input and CDP acknowledgement diagnostic"]
    async fn plain_phone_inputs() -> TestResult {
        measure(None, &["locator", "mouse", "cdp"]).await
    }
}
// END standalone phone diagnostic.

#[tokio::test(flavor = "multi_thread")]
#[ignore = "paired measurement for the plain-base phone input diagnostic"]
async fn enabled_phone_inputs() -> TestResult {
    phone_repro::measure(
        Some(&json!({"input":{"cursor":"on"}})),
        &["locator", "mouse"],
    )
    .await
}
