//! Playwright scripts end to end on a real BrowserOS: an agent's own Playwright calls, sent over
//! MCP `run`, on a page with a same-origin iframe, on a page that logs, and before any op.
//!
//! ```text
//!  MCP run ─▶ browseros-claw-sdk ─ neo-playwright's prelude ─▶ the door ─▶ the crate's ops
//!    page.goto (the run's lazy page: the door opens the tab) ─ page.ariaSnapshot(), the v1 text
//!    ─ getByRef / aria-ref= on a printed ref ─ locator reads ─ reads inside the iframe
//!  page.on('console') first: page.frames opens the lazy tab ─ events.listen ─ the event pump
//!  context.pages() first: the run's init lists every tab
//!  a form: fill a field and click a button through locators, then expect the page's answer
//!    (a matcher that holds passes, one that does not throws)
//!  a local file: goto loads it; a JavaScript URL executes before reporting `ERR_ABORTED`.
//!    Chromium still refuses web pages' own navigations to local files (location, meta refresh,
//!    a link, an iframe, a popup, a web load's redirect)
//!  a selector with `:scope` inside `:has()`: its deep parsed brand reaches `$$eval` and `count`
//! ```
//!
//! Like `e2e_browseros.rs`, it runs only when `CLAW_SDK_E2E_BROWSER` names a BrowserOS binary.

#[path = "support/browseros_rig.rs"]
pub mod browseros_rig;

use std::error::Error;
use std::fs;
use std::future::Future;
use std::net::Ipv4Addr;
use std::path::Path;
use std::pin::Pin;
use std::time::Duration;

use browseros_rig::{CaptchaKey, Rig, TestResult, browser_or_skip, outcome, run};
use neo_trace::RunId;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::time;

/// The whole drive; see `e2e_browseros.rs` for why the budget sits inside the test.
const TEST_BUDGET: Duration = Duration::from_secs(240);

/// What a model trained on Playwright writes: load a page, read its snapshot, act on a ref it
/// printed, then read through locators and inside the frame. The page is built in the script, so
/// the test's expectations sit next to the markup they come from. A `srcdoc` iframe shares its
/// parent's origin, and a `data:` page's frame tree needs no server.
const SCRIPT: &str = r#"
const html = '<title>Neo Playwright</title>'
  + '<h1>Orders</h1>'
  + '<p>Two buttons, one frame.</p>'
  + '<button>Save order</button><button>Cancel</button>'
  + '<iframe title="Details" srcdoc="<p>Inside the frame</p><button>Frame action</button>"></iframe>';
await page.goto('data:text/html,' + encodeURIComponent(html));
const snapshot = await page.ariaSnapshot();
const ref = /button "Save order" \[ref=(e\d+)\]/.exec(snapshot)?.[1];
const frameRef = /button "Frame action" \[ref=(f\d+e\d+)\]/.exec(snapshot)?.[1];
if (!ref || !frameRef) return { snapshot };
return {
  printed: snapshot.replace(/\[ref=(?:f\d+)?e\d+\]/g, '[ref]'),
  ref,
  frameRef,
  byRef: await page.getByRef(ref).textContent(),
  byAriaRef: await page.locator(`aria-ref=${ref}`).innerText(),
  frameByRef: await page.getByRef(frameRef).textContent(),
  buttons: await page.getByRole('button').count(),
  heading: await page.getByRole('heading', { level: 1 }).textContent(),
  note: await page.getByText('Two buttons').textContent(),
  inFrame: await page.frameLocator('iframe').getByRole('button').textContent(),
  frames: page.frames().length,
  frameText: await page.frames()[1].locator('p').textContent(),
  title: await page.title(),
};
"#;

/// A fresh task whose first SDK call is a listener (decision 25, PB input 3). The run's page has
/// no tab yet, and event ops name no page for the door to resolve, so the prelude first sends a
/// `page.frames`, which the door resolves by opening the tab; the listener then names that tab.
/// The page logs on a timer, so the listener hears it whenever its listen lands.
const LISTENER_FIRST: &str = r#"
const heard = [];
page.on('console', (message) => heard.push(message.text()));
const html = '<title>Console</title><script>let n = 0; setInterval(() => console.log("tick " + n++), 50);</script>';
await page.goto('data:text/html,' + encodeURIComponent(html));
for (let i = 0; i < 100 && heard.length === 0; i++) {
  await new Promise((resolve) => setTimeout(resolve, 50));
}
return { heard: heard.slice(0, 1), title: await page.title() };
"#;

/// Direct file navigations and the browser's web-to-file boundary. `LOCAL` (the files),
/// `SERVER` (a web server on loopback) and `VIEW_SOURCE` are defined ahead of it. Each page
/// navigation also tries a web URL, proving the browser exercised that route.
const LOCAL_READS: &str = r#"
const tried = { gotos: {}, local: {}, allowed: {} };
const attempt = async (url) => {
  try {
    await page.goto(url);
    return { url: page.url(), title: await page.title(), content: clip(await page.content()) };
  } catch (error) {
    return { owner: error.owner, code: error.code, delivery: error.delivery };
  }
};
// A run returns at most 256 KiB. A loaded file's text leads its document, so a clipped read
// still shows it.
const clip = (text) => (typeof text === 'string' ? text.slice(0, 4000) : text);
// Every tab's document and every frame's text: wherever a loaded file would show.
const everything = async () => {
  const tabs = [];
  for (const tab of context.pages()) {
    const frames = [];
    for (const frame of tab.frames()) {
      frames.push(clip(await frame.locator('body').textContent({ timeout: 2000 }).catch((error) => error.code)));
    }
    tabs.push({ url: tab.url(), content: clip(await tab.content().catch((error) => error.code)), frames });
  }
  return tabs;
};
for (const url of [...LOCAL, 'chrome://version', VIEW_SOURCE, 'about:version']) {
  tried.gotos[url] = await attempt(url);
}
tried.javascript = await page.goto("javascript:void(document.title = 'ran')")
  .then(() => ({ returned: true }), (error) => ({ code: error.code, message: String(error.message).split('\n')[0] }));
tried.javascriptTitle = await page.title();
// A loopback frame in a data: page is refused as a private network request, so the frame's
// allowed URL is a data: one. A popup has none: with no user gesture, the popup blocker refuses
// window.open whatever its URL, so that route pins the outcome alone.
const routes = {
  location: [(to) => `<script>location.href = ${JSON.stringify(to)}</script>`, SERVER + '/page'],
  meta: [(to) => `<meta http-equiv="refresh" content="0;url=${to}">`, SERVER + '/page'],
  link: [(to) => `<a id="to" href="${to}">to</a><script>document.getElementById('to').click()</script>`, SERVER + '/page'],
  iframe: [(to) => `<iframe src="${to}"></iframe>`, 'data:text/html,reached'],
  popup: [(to) => `<script>window.open(${JSON.stringify(to)})</script>`, null],
};
for (const [route, [html, allowed]] of Object.entries(routes)) {
  const targets = LOCAL.map((to) => [to, tried.local]);
  if (allowed) targets.push([allowed, tried.allowed]);
  for (const [to, into] of targets) {
    await page.goto('data:text/html,' + encodeURIComponent(html(to)));
    await new Promise((resolve) => setTimeout(resolve, 1500));
    into[`${route} to ${to}`] = await everything();
  }
}
// A load the door allows, redirected by its server: Chromium follows the redirect, not the door.
for (const [path, into] of [['/to-hosts', tried.local], ['/to-secret', tried.local], ['/to-page', tried.allowed]]) {
  into[`redirect ${path}`] = { load: await attempt(SERVER + path), tabs: await everything() };
}
return tried;
"#;

/// What a model writes to use a form: fill a field by its label, click a button by its role, then
/// `expect` the page's answer. The button's handler writes the greeting, so it shows only if the
/// fill and the click both reached the page. A matcher that does not hold must throw, or a
/// passing `expect` would prove nothing.
const FORM: &str = r#"
const html = '<title>Greeter</title>'
  + '<label>Name <input id="name"></label>'
  + '<button onclick="document.querySelector(\'output\').textContent = \'Hello, \' + document.querySelector(\'#name\').value">Greet</button>'
  + '<output></output>';
await page.goto('data:text/html,' + encodeURIComponent(html));
await page.getByLabel('Name').fill('Ada Lovelace');
await page.getByRole('button', { name: 'Greet' }).click();
await expect(page.getByRole('status')).toHaveText('Hello, Ada Lovelace');
const unmet = await expect(page.getByRole('status'))
  .toHaveText('Goodbye', { timeout: 500 })
  .then(() => 'passed', (error) => String(error.message));
return {
  value: await page.getByLabel('Name').inputValue(),
  greeting: await page.getByRole('status').textContent(),
  unmet,
};
"#;

#[tokio::test(flavor = "multi_thread")]
async fn a_playwright_script_reads_a_page_its_refs_and_its_frame() -> TestResult {
    on_browseros(|rig| Box::pin(drive(rig))).await
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "hosted console listening is off since Runtime is no longer left enabled; owner: ignore hosted for now (2026-11-11)"]
async fn a_fresh_tasks_first_call_can_be_a_console_listener() -> TestResult {
    on_browseros(|rig| Box::pin(listen_first(rig))).await
}

#[tokio::test(flavor = "multi_thread")]
async fn context_pages_lists_every_tab_in_a_runs_first_statement() -> TestResult {
    on_browseros(|rig| Box::pin(tabs_first(rig))).await
}

#[tokio::test(flavor = "multi_thread")]
async fn a_script_fills_and_clicks_a_form_and_expects_the_answer() -> TestResult {
    on_browseros(|rig| Box::pin(fill_and_click(rig))).await
}

#[tokio::test(flavor = "multi_thread")]
async fn a_script_loads_local_files_but_web_pages_cannot_navigate_to_them() -> TestResult {
    on_browseros(|rig| Box::pin(local_navigations(rig))).await
}

#[tokio::test(flavor = "multi_thread")]
async fn a_selector_with_scope_inside_has_reaches_its_op() -> TestResult {
    on_browseros(|rig| Box::pin(scope_inside_has(rig))).await
}

/// Every task starts its own worker monitors when its first page operation opens a tab.
/// Their synchronous listener setup must never become an obligation the script left behind.
#[tokio::test(flavor = "multi_thread")]
async fn fresh_short_tasks_never_report_prelude_listeners_as_not_awaited() -> TestResult {
    on_browseros(|rig| Box::pin(short_fresh_tasks(rig))).await
}

async fn short_fresh_tasks(rig: &Rig) -> TestResult {
    rig.until_ready().await?;
    let client = rig.client().await?;
    for attempt in 0..12 {
        let task = format!("fresh-short-title-{attempt}");
        let reply = run(
            &client,
            "return await page.title()",
            &json!({"taskId":task}),
        )
        .await?;
        let meta = outcome(&reply)?;
        assert_eq!(
            meta.get("status"),
            Some(&json!("ok")),
            "task {attempt}: {reply:?}"
        );
        let public = serde_json::to_value(&reply)?;
        let encoded = public.to_string();
        for forbidden in ["[not awaited]", "notAwaited", "not_awaited"] {
            assert!(
                !encoded.contains(forbidden),
                "task {attempt}: unexpected {forbidden}: {public}"
            );
        }
        let id = meta
            .get("id")
            .and_then(Value::as_str)
            .ok_or("a run has its id")?;
        let trace = rig
            .host(
                "GET",
                &format!("/v1/runs/{id}?wait=30"),
                Some(browseros_rig::HOST_TOKEN),
                None,
            )
            .await?;
        assert_eq!(trace.status, 200, "task {attempt}: {}", trace.text());
        let trace = trace.json()?;
        assert!(
            !trace.to_string().contains("not_awaited"),
            "task {attempt}: {trace}"
        );
        assert_eq!(
            trace.get("complete"),
            Some(&json!(true)),
            "task {attempt}: {trace}"
        );
    }
    client.cancel().await?;
    Ok(())
}

/// `drive` on a fresh BrowserOS within the test's budget, with both processes' logs on failure.
async fn on_browseros(
    drive: impl for<'a> FnOnce(&'a Rig) -> Pin<Box<dyn Future<Output = TestResult> + 'a>>,
) -> TestResult {
    let Some(browser) = browser_or_skip() else {
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

/// The reply of one `run`, which must have succeeded.
async fn reply_of(rig: &Rig, code: &str, task: &str) -> Result<Value, Box<dyn Error>> {
    rig.until_ready().await?;
    let client = rig.client().await?;
    let result = run(&client, code, &json!({ "taskId": task })).await?;
    let meta = outcome(&result)?;
    assert_eq!(meta.get("status"), Some(&json!("ok")), "{result:?}");
    client.cancel().await?;
    let reply = result
        .structured_content
        .ok_or("a run's reply is its structured content")?;
    Ok(reply)
}

async fn listen_first(rig: &Rig) -> TestResult {
    let reply = reply_of(rig, LISTENER_FIRST, "listener-first").await?;
    let value = reply.get("value").ok_or("the script returns")?;
    assert_eq!(value.get("title"), Some(&json!("Console")), "{reply}");
    let heard = value
        .get("heard")
        .and_then(Value::as_array)
        .and_then(|heard| heard.first())
        .and_then(Value::as_str);
    assert!(
        heard.is_some_and(|text| text.starts_with("tick ")),
        "the listener heard the page it was set on: {reply}"
    );
    assert_eq!(
        reply.get("notAwaited"),
        None,
        "the event pump still pending at the end is cut quietly: {reply}"
    );
    Ok(())
}

async fn tabs_first(rig: &Rig) -> TestResult {
    let reply = reply_of(
        rig,
        "return context.pages().map((tab) => tab.url());",
        "tabs-first",
    )
    .await?;
    assert_eq!(
        reply.get("value"),
        Some(&json!(["about:blank"])),
        "the tab the browser started with, listed before any op: {reply}"
    );
    Ok(())
}

async fn drive(rig: &Rig) -> TestResult {
    rig.until_ready().await?;
    let client = rig.client().await?;
    let result = run(&client, SCRIPT, &json!({ "taskId": "playwright" })).await?;
    assert_eq!(
        outcome(&result)?.get("status"),
        Some(&json!("ok")),
        "{result:?}"
    );
    let value = result
        .structured_content
        .as_ref()
        .and_then(|content| content.get("value"))
        .ok_or_else(|| format!("the script returns its reads: {result:?}"))?;
    let snapshot = value.get("snapshot").and_then(Value::as_str);
    assert_eq!(
        snapshot, None,
        "the snapshot printed a ref for both buttons: {value}"
    );
    let text = |name: &str| value.get(name).and_then(Value::as_str);
    assert!(
        text("ref").is_some_and(|printed| printed.starts_with('e')),
        "a main-frame ref is bare (`e5`): {value}"
    );
    assert!(
        text("frameRef").is_some_and(|printed| printed.starts_with('f')),
        "a ref inside the iframe names its document (`f1e2`): {value}"
    );
    // The v1 text over MCP: what can be acted on, with refs, and the page's text, since the page
    // is small; the iframe is a section, its content indented under it.
    assert_eq!(
        text("printed"),
        Some(
            "- heading \"Orders\" [level=1]\n- text: Two buttons, one frame.\n\
             - button \"Save order\" [ref]\n- button \"Cancel\" [ref]\n\
             - iframe \"Details\" [ref]:\n  - text: Inside the frame\n  \
             - button \"Frame action\" [ref]"
        ),
        "the default snapshot is the v1 text: {value}"
    );
    let reads: Vec<(&str, Value)> = [
        "byRef",
        "byAriaRef",
        "frameByRef",
        "buttons",
        "heading",
        "note",
        "inFrame",
        "frames",
        "frameText",
        "title",
    ]
    .into_iter()
    .map(|name| (name, value.get(name).cloned().unwrap_or(Value::Null)))
    .collect();
    assert_eq!(
        reads,
        [
            ("byRef", json!("Save order")),
            ("byAriaRef", json!("Save order")),
            ("frameByRef", json!("Frame action")),
            ("buttons", json!(2)),
            ("heading", json!("Orders")),
            ("note", json!("Two buttons, one frame.")),
            ("inFrame", json!("Frame action")),
            ("frames", json!(2)),
            ("frameText", json!("Inside the frame")),
            ("title", json!("Neo Playwright")),
        ],
        "each read answers from the markup the script loaded: {value}"
    );
    client.cancel().await?;
    Ok(())
}

async fn fill_and_click(rig: &Rig) -> TestResult {
    let reply = reply_of(rig, FORM, "form").await?;
    let value = reply.get("value").ok_or("the script returns")?;
    let text = |name: &str| value.get(name).and_then(Value::as_str);
    assert_eq!(
        (text("value"), text("greeting")),
        (Some("Ada Lovelace"), Some("Hello, Ada Lovelace")),
        "the fill reached the field and the click ran the page's handler: {reply}"
    );
    assert!(
        text("unmet").is_some_and(|message| message != "passed" && message.contains("toHaveText")),
        "a matcher that does not hold throws, naming itself: {reply}"
    );
    Ok(())
}

/// Upstream's `tests/assets/deep-shadow.html`, which Playwright 1.64.1's `:is` case loads.
const DEEP_SHADOW: &str = r"<script>
window.addEventListener('DOMContentLoaded', () => {
  const outer = document.createElement('section');
  document.body.appendChild(outer);
  const root1 = document.createElement('div');
  root1.setAttribute('id', 'root1');
  outer.appendChild(root1);
  const shadowRoot1 = root1.attachShadow({mode: 'open'});
  const span1 = document.createElement('span');
  span1.setAttribute('data-testid', 'foo');
  span1.textContent = 'Hello from root1';
  shadowRoot1.appendChild(span1);
  const root2 = document.createElement('div');
  shadowRoot1.appendChild(root2);
  const shadowRoot2 = root2.attachShadow({mode: 'open'});
  const span2 = document.createElement('span');
  span2.setAttribute('data-testid', 'foo');
  span2.setAttribute('id', 'target');
  span2.textContent = 'Hello from root2';
  shadowRoot2.appendChild(span2);
  const root3 = document.createElement('div');
  shadowRoot1.appendChild(root3);
  const shadowRoot3 = root3.attachShadow({mode: 'open'});
  const span3 = document.createElement('span');
  span3.setAttribute('data-testid', 'foo');
  span3.textContent = 'Hello from root3';
  shadowRoot3.appendChild(span3);
  const span4 = document.createElement('span');
  span4.textContent = 'Hello from root3 #2';
  span4.setAttribute('attr', 'value space');
  shadowRoot3.appendChild(span4);
});
</script>";

/// S3b, tier 2's `selectors-css` cases `should work with :is` (L389) and `:scope` (L431). A
/// locator's brand carries its parsed selector, and `:scope` inside `:has()` parses 28 levels deep,
/// past where the run's encoding of an op's input overflowed the realm's stack. Each call answers
/// what Playwright 1.64.1 does, as tier 2 records it, through `$$eval` and a locator's `count`.
async fn scope_inside_has(rig: &Rig) -> TestResult {
    let script = format!(
        r"
        const settled = (call) => call().catch((error) => String(error.message).split('\n')[0]);
        const IS_SCOPE = 'css=#root1:has(:is(:scope, #root1))';
        await page.goto('data:text/html,' + encodeURIComponent({deep_shadow}));
        const isScope = await settled(() => page.$$eval(IS_SCOPE, (els) => els.length));
        const counted = await settled(() => page.locator(IS_SCOPE).count());
        await page.goto('data:text/html,' + encodeURIComponent(
          '<section><span id=span1><span id=inner></span></span><span id=span2></span></section>'));
        const notHasScope = await settled(() =>
          page.$$eval('#span1 >> span:not(:has(:scope > div))', (els) => els.map((e) => e.id)));
        return {{ isScope, counted, notHasScope }};",
        deep_shadow = json!(DEEP_SHADOW)
    );
    let reply = reply_of(rig, &script, "scope-inside-has").await?;
    assert_eq!(
        reply.get("value"),
        Some(&json!({ "isScope": 1, "counted": 1, "notHasScope": ["inner"] })),
        "the parsed selector reaches each op whole: {reply}"
    );
    Ok(())
}

/// The navigation error describes Chromium's abort, while the title proves script execution.
fn assert_javascript_navigation(tried: &Value) {
    assert_eq!(
        tried.pointer("/javascript/code"),
        Some(&json!("navigation_failed")),
        "{tried}"
    );
    assert_eq!(
        tried.pointer("/javascript/message"),
        Some(&json!(
            "page.goto: net::ERR_ABORTED at javascript:void(document.title = 'ran')"
        )),
        "{tried}"
    );
    assert_eq!(
        tried.get("javascriptTitle"),
        Some(&json!("ran")),
        "the script executed before the error"
    );
}

async fn local_navigations(rig: &Rig) -> TestResult {
    // A synthetic local file proves goto reads from disk rather than a web fixture.
    let dir = tempfile::tempdir()?;
    let secret = dir.path().join("host-token");
    let marker = format!("neo-local-secret-{}", RunId::new());
    fs::write(&secret, &marker)?;
    let hosts = fs::read_to_string("/etc/hosts")?;
    let server = format!("http://127.0.0.1:{}", serve_pages(&secret).await?);
    let local = [
        "file:///etc/hosts".to_owned(),
        format!("file://{}", secret.display()),
    ];
    let view_source = format!("view-source:{server}/page");
    let code = format!(
        "const LOCAL = {};\nconst SERVER = {};\nconst VIEW_SOURCE = {};\n{LOCAL_READS}",
        json!(local),
        json!(server),
        json!(view_source),
    );
    let reply = reply_of(rig, &code, "local-reads").await?;
    let tried = reply
        .get("value")
        .ok_or("the script returns what it tried")?;

    let refused = json!({ "owner": "script", "code": "invalid_input", "delivery": "not_sent" });
    for url in ["chrome://version", view_source.as_str()] {
        assert_eq!(
            tried.get("gotos").and_then(|gotos| gotos.get(url)),
            Some(&refused),
            "other unsupported schemes retain their refusal: {tried}"
        );
    }
    for (url, expected) in local.iter().zip(["localhost", marker.as_str()]) {
        let loaded = tried
            .get("gotos")
            .and_then(|gotos| gotos.get(url))
            .ok_or("each local navigation has a result")?;
        assert_eq!(loaded.get("url"), Some(&json!(url)), "{loaded}");
        assert!(
            loaded
                .get("content")
                .and_then(Value::as_str)
                .is_some_and(|content| content.contains(expected)),
            "goto loaded the local file: {loaded}"
        );
    }
    assert_javascript_navigation(tried);
    assert_eq!(
        tried.pointer("/gotos/about:version/url"),
        Some(&json!("about:version")),
        "an about: URL stays an about: page and never becomes chrome://version: {tried}"
    );

    let secrets: Vec<&str> = hosts
        .lines()
        .map(str::trim)
        .filter(|line| line.len() >= 8)
        .chain([marker.as_str()])
        .collect();
    let local_reads = tried
        .get("local")
        .and_then(Value::as_object)
        .ok_or_else(|| format!("the routes to local files: {tried}"))?;
    assert_eq!(
        local_reads.len(),
        12,
        "five routes and a redirect, each to both files: {tried}"
    );
    for (route, read) in local_reads {
        for text in texts(read) {
            for secret in &secrets {
                assert!(
                    !text.contains(secret),
                    "{route} read {secret:?} from a local file: {text}"
                );
            }
        }
    }
    let allowed = tried
        .get("allowed")
        .and_then(Value::as_object)
        .ok_or_else(|| format!("the routes to allowed URLs: {tried}"))?;
    assert_eq!(allowed.len(), 5, "four routes and a redirect: {tried}");
    for (route, read) in allowed {
        assert!(
            texts(read).iter().any(|text| text.contains("reached")),
            "{route} reaches a URL Neo allows, so the same route to a file was really tried: {read}"
        );
    }
    Ok(())
}

/// Every string in `value`, however deep.
fn texts(value: &Value) -> Vec<&str> {
    match value {
        Value::String(text) => vec![text.as_str()],
        Value::Array(items) => items.iter().flat_map(texts).collect(),
        Value::Object(fields) => fields.values().flat_map(texts).collect(),
        Value::Null | Value::Bool(_) | Value::Number(_) => Vec::new(),
    }
}

/// A web server on loopback for the cases Neo allows and for the redirects: `/page` says
/// `reached`, and `/to-hosts`, `/to-secret` and `/to-page` redirect to `/etc/hosts`, `secret` and
/// `/page`. Returns its port; it serves until the test's runtime ends.
async fn serve_pages(secret: &Path) -> Result<u16, Box<dyn Error>> {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await?;
    let port = listener.local_addr()?.port();
    let secret = format!("file://{}", secret.display());
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            let secret = secret.clone();
            tokio::spawn(async move {
                let mut request = [0_u8; 4096];
                let read = stream.read(&mut request).await.unwrap_or(0);
                let head = String::from_utf8_lossy(request.get(..read).unwrap_or_default());
                let response = match head.split(' ').nth(1) {
                    Some("/to-hosts") => redirect("file:///etc/hosts"),
                    Some("/to-secret") => redirect(&secret),
                    Some("/to-page") => redirect("/page"),
                    _ => {
                        let body = "<title>reached</title><p>reached</p>";
                        format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                            body.len()
                        )
                    }
                };
                let _ = stream.write_all(response.as_bytes()).await;
            });
        }
    });
    Ok(port)
}

fn redirect(location: &str) -> String {
    format!(
        "HTTP/1.1 302 Found\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    )
}
