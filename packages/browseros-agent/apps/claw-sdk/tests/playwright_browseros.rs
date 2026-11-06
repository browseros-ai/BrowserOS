//! Playwright scripts end to end on a real BrowserOS: an agent's own Playwright calls, sent over
//! MCP `run`, on a page with a same-origin iframe, on a page that logs, and before any op.
//!
//! ```text
//!  MCP run ─▶ browseros-claw-sdk ─ browseros-playwright's prelude ─▶ the door ─▶ the crate's ops
//!    page.goto (the run's lazy page: the door opens the tab) ─ page.ariaSnapshot({ mode: 'ai' })
//!    ─ getByRef / aria-ref= on a printed ref ─ locator reads ─ reads inside the iframe
//!  page.on('console') first: page.frames opens the lazy tab ─ events.listen ─ the event pump
//!  context.pages() first: the run's init lists every tab
//!  a form: fill a field and click a button through locators, then expect the page's answer
//!    (a matcher that holds passes, one that does not throws)
//!  a local file (Neo's host token, /etc/hosts): the door refuses each goto to one, and the
//!    page's own navigations to one (location, meta refresh, a link, an iframe, a popup, an
//!    allowed load's redirect) read nothing local
//!  a read between two keystrokes in a password field: the read masks the field, the host reads
//!    none of it back from the trace outside its page facts, and once the tab closes a later run
//!    masks the field's final value and none of its prefixes
//!  a page that shows its own password field in its title as it loads, before any op sweeps it:
//!    neither the realm nor the trace gets the field's value from the goto's page facts (ignored,
//!    a known gap)
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

use browseros_rig::{
    CaptchaKey, HOST_TOKEN, PATIENCE, Rig, TestResult, browser_or_skip, outcome, run,
};
use browseros_trace::RunId;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::time::{self, Instant};

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
const snapshot = await page.ariaSnapshot({ mode: 'ai' });
const ref = /button "Save order" \[ref=(e\d+)\]/.exec(snapshot)?.[1];
const frameRef = /button "Frame action" \[ref=(f\d+e\d+)\]/.exec(snapshot)?.[1];
if (!ref || !frameRef) return { snapshot };
return {
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

/// Every way a script could get a local file in front of a read. `LOCAL` (the files), `SERVER`
/// (a web server on loopback) and `VIEW_SOURCE` are defined ahead of it. Each of the page's own
/// navigations runs once to each file and once to a URL Neo allows, which the page reaches, so
/// a route that read nothing local was refused rather than never tried.
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

/// A page that types a password into its own field a key at a time, as autofill or the owner
/// would, and shows the field in its title: its first four keys as it loads, the rest when the
/// agent clicks its button. The script gets the password as character codes (`CODES`) and the
/// page builds it from them, so neither source shows any of it, and whatever of it a run's reply
/// or stored records hold came from the page.
const TYPING_PAGE: &str = r#"
const html = '<title>Sign in</title><input type="password"><button>Type the rest</button>'
  + '<script>'
  + 'const secret = String.fromCharCode(' + CODES.join(',') + ');'
  + 'const field = document.querySelector("input");'
  + 'let keys = 4;'
  + 'const type = () => {'
  + '  field.value = secret.slice(0, keys++);'
  + '  field.dispatchEvent(new Event("input", { bubbles: true }));'
  + '  document.title = "Signing in with " + field.value;'
  + '};'
  + 'type();'
  + 'document.querySelector("button").onclick = () => { while (keys <= secret.length) type(); };'
  + '</script>';
"#;

/// After [`TYPING_PAGE`]: the agent reads the page between its four keys and the rest, so the
/// read lands between two keystrokes by construction, whatever the machine's load.
const READ_WHILE_TYPING: &str = r"
await page.goto('data:text/html,' + encodeURIComponent(html));
const read = { title: await page.title(), content: await page.content() };
await page.getByRole('button', { name: 'Type the rest' }).click();
return read;
";

/// After [`TYPING_PAGE`]: the agent loads the page and reads nothing of it, so no op sweeps its
/// field. It returns the title its realm holds from the goto's own reply as character codes,
/// which no mask matches, so the reply shows exactly what reached the realm.
const UNSWEPT_TITLE: &str = r"
await page.goto('data:text/html,' + encodeURIComponent(html));
return Array.from(page._title, (char) => char.charCodeAt(0));
";

/// claw-sdk's quiet period for the typing test: the longest it takes (`TYPED_QUIET_MAX`), not
/// the real 1 s. At 1 s, a loaded machine that stalls the read for a second between two keys
/// ends the field's typing there, and the prefix it held rightly becomes a secret, which would
/// fail a test about what a read does, not about the clock. No key waits this long here, so the
/// final value leaves what is held only when the test closes its tab.
const PATIENT_TYPING: [(&str, &str); 1] = [("CLAW_TYPED_QUIET_MS", "600000")];

/// Closes the task's page, the typing one: its fields can take no more input, so claw-sdk hands
/// what they hold to the process's set.
const CLOSE_TYPING: &str = "await page.close(); return 'closed';";

/// Every prefix of `SECRET` four characters or longer (a shorter value is never a secret), and the
/// whole password, as a later run's reply shows them.
const PREFIXES_LATER: &str = r"
const prefixes = [];
for (let end = 4; end < SECRET.length; end++) prefixes.push(SECRET.slice(0, end));
return { prefixes, whole: SECRET };
";

#[tokio::test(flavor = "multi_thread")]
async fn a_playwright_script_reads_a_page_its_refs_and_its_frame() -> TestResult {
    on_browseros(|rig| Box::pin(drive(rig))).await
}

#[tokio::test(flavor = "multi_thread")]
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
async fn a_script_reads_no_local_file_through_any_navigation() -> TestResult {
    on_browseros(|rig| Box::pin(no_local_reads(rig))).await
}

#[tokio::test(flavor = "multi_thread")]
async fn a_read_between_two_keystrokes_shows_the_field_to_no_one_and_registers_no_prefix()
-> TestResult {
    on_browseros_with(&PATIENT_TYPING, |rig| Box::pin(read_while_typing(rig))).await
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "known gap: page facts are recorded before any sweep of the page; see sdk-skeleton decision 35n"]
async fn a_page_fact_recorded_before_any_sweep_holds_no_field_value() -> TestResult {
    on_browseros(|rig| Box::pin(page_fact_before_sweep(rig))).await
}

#[tokio::test(flavor = "multi_thread")]
async fn a_selector_with_scope_inside_has_reaches_its_op() -> TestResult {
    on_browseros(|rig| Box::pin(scope_inside_has(rig))).await
}

/// `drive` on a fresh BrowserOS within the test's budget, with both processes' logs on failure.
async fn on_browseros(
    drive: impl for<'a> FnOnce(&'a Rig) -> Pin<Box<dyn Future<Output = TestResult> + 'a>>,
) -> TestResult {
    on_browseros_with(&[], drive).await
}

/// As [`on_browseros`], with `env` also set on claw-sdk.
async fn on_browseros_with(
    env: &[(&str, &str)],
    drive: impl for<'a> FnOnce(&'a Rig) -> Pin<Box<dyn Future<Output = TestResult> + 'a>>,
) -> TestResult {
    let Some(browser) = browser_or_skip() else {
        return Ok(());
    };
    let rig = Rig::start_with(&browser, CaptchaKey::Absent, env)?;
    let failure = match time::timeout(TEST_BUDGET, drive(&rig)).await {
        Ok(Ok(())) => return Ok(()),
        Ok(Err(error)) => error.to_string(),
        Err(_elapsed) => format!("the drive did not finish within {TEST_BUDGET:?}"),
    };
    Err(format!("{failure}\n{}", rig.logs()).into())
}

/// The reply of one `run`, which must have succeeded.
async fn reply_of(rig: &Rig, code: &str, task: &str) -> Result<Value, Box<dyn Error>> {
    Ok(run_of(rig, code, task).await?.1)
}

/// One `run`, which must have succeeded: its id and its reply.
async fn run_of(rig: &Rig, code: &str, task: &str) -> Result<(String, Value), Box<dyn Error>> {
    rig.until_ready().await?;
    let client = rig.client().await?;
    let result = run(&client, code, &json!({ "taskId": task })).await?;
    let meta = outcome(&result)?;
    assert_eq!(meta.get("status"), Some(&json!("ok")), "{result:?}");
    let id = meta
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("the run has an id: {meta}"))?
        .to_owned();
    client.cancel().await?;
    let reply = result
        .structured_content
        .ok_or("a run's reply is its structured content")?;
    Ok((id, reply))
}

/// Run `id` as the host reads it back from the trace store, once the run has ended: its code,
/// its reply, and each op's input, result and page facts.
async fn recorded(rig: &Rig, id: &str) -> Result<Value, Box<dyn Error>> {
    let read = rig
        .host(
            "GET",
            &format!("/v1/runs/{id}?wait=30"),
            Some(HOST_TOKEN),
            None,
        )
        .await?;
    assert_eq!(read.status, 200, "{}", read.text());
    Ok(serde_json::from_slice(&read.body)?)
}

/// `records` as text without each op's page facts (`page`, its tab's URL and title after it).
fn without_page_facts(mut records: Value) -> String {
    if let Some(operations) = records.get_mut("operations").and_then(Value::as_array_mut) {
        for operation in operations.iter_mut().filter_map(Value::as_object_mut) {
            operation.remove("page");
        }
    }
    records.to_string()
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

async fn read_while_typing(rig: &Rig) -> TestResult {
    let secret = "hunter2-correct-horse";
    let task = "read-while-typing";
    let (typing, reply) = run_of(rig, &on_typing_page(secret, READ_WHILE_TYPING), task).await?;
    let read = reply.get("value").ok_or("the script returns its read")?;
    let masked_title = "Signing in with [private]";
    assert_eq!(
        read.get("title"),
        Some(&json!(masked_title)),
        "the read masked the field's value where the page showed it: {read}"
    );
    let prefixes: Vec<&str> = (4..secret.len())
        .filter_map(|end| secret.get(..end))
        .collect();
    for text in texts(read) {
        assert!(
            prefixes.iter().all(|prefix| !text.contains(prefix)),
            "the read shows none of the field's value: {text}"
        );
    }
    // Read now, while every value the page typed is still only held: the process's set has
    // learned none of them, so a record its run did not mask itself would show one raw. Its page
    // facts are left out: core cached the page's title before any op swept the field, and
    // `a_page_fact_recorded_before_any_sweep_holds_no_field_value` owns that case.
    let typing_records = without_page_facts(recorded(rig, &typing).await?);
    assert!(
        typing_records.contains(masked_title),
        "the typing run's records keep the read's title, masked: {typing_records}"
    );
    assert_no_fragment(secret, &typing_records, "the typing run's records")?;

    // The click's keys reach claw-sdk through the journal, beside the op rather than in it, so
    // the last may land after the run ends. A later run masks the final value once it is held.
    let declared = format!("const SECRET = {};\n", json!(secret));
    let later_code = format!("{declared}{PREFIXES_LATER}");
    let later_run = || reply_of(rig, &later_code, task);
    let deadline = Instant::now() + PATIENCE;
    loop {
        let later = later_run().await?;
        if later.pointer("/value/whole") == Some(&json!("[private]")) {
            break;
        }
        if Instant::now() >= deadline {
            return Err(format!("the field's final value never reached claw-sdk: {later}").into());
        }
        time::sleep(Duration::from_millis(100)).await;
    }

    // Only after the last key, so the close cannot cut the typing short. Closing the tab takes
    // its values out of what is held and hands them to the process's set, the one way out of
    // what is held this test reaches. The run after it shows the final value survived that
    // handoff and no prefix was handed over with it. It cannot show the set learned the value:
    // a value still held masks every run exactly as a learned one does, so a close that hands
    // over nothing passes here, and typed_values' `a_closed_tab_registers_its_values_at_once`
    // pins the handoff itself. For the same reason no ordering is needed: the registry reports
    // the close to claw-sdk's timer asynchronously, and the value masks the next run whether it
    // is held, learning or learned, while every prefix was superseded before that run began.
    let (closing, closed) = run_of(rig, CLOSE_TYPING, task).await?;
    assert_eq!(closed.get("value"), Some(&json!("closed")), "{closed}");
    let closing_records = recorded(rig, &closing).await?.to_string();
    assert_no_fragment(secret, &closing_records, "the closing run's records")?;

    // Begun after the close and after every keystroke: no prefix was held while it went on, so
    // one it masked could only be a secret of the process's set, and a final value it showed raw
    // would have been dropped by the close.
    let later = later_run().await?;
    assert_eq!(
        later.get("value"),
        Some(&json!({ "prefixes": prefixes, "whole": "[private]" })),
        "once its tab closed, the field's final value is still masked, and no prefix of it, the \
         one the read found included, is a secret: {later}"
    );
    Ok(())
}

async fn page_fact_before_sweep(rig: &Rig) -> TestResult {
    let secret = "hunter2-correct-horse";
    let (loading, reply) =
        run_of(rig, &on_typing_page(secret, UNSWEPT_TITLE), "unswept-title").await?;
    let title: String = reply
        .get("value")
        .and_then(Value::as_array)
        .ok_or_else(|| format!("the script returns its title's codes: {reply}"))?
        .iter()
        .filter_map(Value::as_u64)
        .filter_map(|code| char::from_u32(u32::try_from(code).ok()?))
        .collect();
    // Both, so one red run names every place the field's value reached.
    let realm = assert_no_fragment(secret, &title, "the title the script's realm got");
    let records = recorded(rig, &loading).await?.to_string();
    let stored = assert_no_fragment(secret, &records, "the loading run's records");
    match (realm, stored) {
        (Ok(()), Ok(())) => Ok(()),
        (realm, stored) => Err([realm.err(), stored.err()]
            .into_iter()
            .flatten()
            .map(|error| error.to_string())
            .collect::<Vec<_>>()
            .join("\n")
            .into()),
    }
}

/// A script that builds [`TYPING_PAGE`] for `secret`, then runs `then` on it.
fn on_typing_page(secret: &str, then: &str) -> String {
    let codes: Vec<u32> = secret.chars().map(u32::from).collect();
    format!("const CODES = {};\n{TYPING_PAGE}{then}", json!(codes))
}

/// Fails if `text` holds any raw piece of `secret` a read could have shown: a prefix of four
/// characters or more (the value a read between two keys finds), or any eight characters in a
/// row (what a mask of only a prefix would leave of the rest).
fn assert_no_fragment(secret: &str, text: &str, what: &str) -> TestResult {
    let chars: Vec<char> = secret.chars().collect();
    let prefixes = (4..=chars.len()).map(|end| chars.get(..end));
    let windows = chars.windows(8).map(Some);
    for fragment in prefixes.chain(windows).flatten() {
        let fragment: String = fragment.iter().collect();
        if text.contains(&fragment) {
            return Err(
                format!("{what} hold `{fragment}` of the field's value raw: {text}").into(),
            );
        }
    }
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

async fn no_local_reads(rig: &Rig) -> TestResult {
    // Stands in for Neo's host token and CAPTCHA key files: a file the browser's user can read,
    // whose contents appear nowhere else.
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
    for url in local
        .iter()
        .map(String::as_str)
        .chain(["chrome://version", view_source.as_str()])
    {
        assert_eq!(
            tried.get("gotos").and_then(|gotos| gotos.get(url)),
            Some(&refused),
            "the door refuses a goto to {url}: {tried}"
        );
    }
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
