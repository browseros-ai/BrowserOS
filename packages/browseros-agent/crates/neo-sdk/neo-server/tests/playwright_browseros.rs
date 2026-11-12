//! Playwright scripts end to end on a real BrowserOS: an agent's own Playwright calls, sent over
//! MCP `run`, on a page with a same-origin iframe, on a page that logs, and before any op.
//!
//! ```text
//!  MCP run ─▶ browseros-claw-sdk ─ neo-playwright's prelude ─▶ the door ─▶ the crate's ops
//!    page.goto (the run's lazy page: the door opens the tab) ─ page.ariaSnapshot(), the v1 text
//!    ─ getByRef / aria-ref= on a printed ref ─ locator reads ─ reads inside the iframe
//!  page.on('console') first: page.frames opens the lazy tab ─ events.listen ─ the event pump;
//!    the rig's server runs hosted, where the listener hears nothing, by design
//!  context.pages() first: the run's init lists every tab
//!  a form: fill a field and click a button through locators, then expect the page's answer
//!    (a matcher that holds passes, one that does not throws)
//!  a local file: a script cannot open one (page.goto, frame.goto, a page it opens), and
//!    Chromium refuses web pages' own navigations to local files (location, meta refresh, a
//!    link, an iframe, a popup, a clicked link or window.open, a written iframe, a web load's
//!    redirect); a JavaScript URL executes before reporting `ERR_ABORTED`
//!  the owner's own file: tab, opened outside a script: it loads, a script sees it listed and
//!    cannot read it, and cannot step back into a file: entry of a tab's history
//!  a selector with `:scope` inside `:has()`: its deep parsed brand reaches `$$eval` and `count`
//!  page.ariaSnapshot({ delta: true }) across runs: the baseline is the last snapshot a run gave
//!    its caller (not one a failed run took, one only inspected, nor one earlier in the run);
//!    a delta after { text: true } on a page whose default leaves text out lists only the change,
//!    in the text form; and the reply's tip after a full snapshot found a stored one: at most
//!    three per task, or per tab for runs with no task
//!  a ref from an earlier run, used in the new tab a run with no task starts on: its failure says
//!    the ref is not from this tab; one that went stale on a tab with a snapshot does not
//!  await neo.task(id): `page` is the task's tab on the next line (url, tabId, taskId), as is a
//!    host task's from a run's first line; a host lock with blocked reads leaves its URL unread
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

use browseros_rig::{CaptchaKey, Client, Rig, TestResult, browser_or_skip, get, outcome, put, run};
use neo_trace::RunId;
use rmcp::model::{CallToolResult, ContentBlock};
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
await new Promise((resolve) => setTimeout(resolve, 1000));
return { heard, title: await page.title() };
"#;

/// What the two route scripts share: reads that clip, catch a refusal, and look into every tab
/// and frame where a loaded file would show. `LOCAL` (the files), `SERVER` (a web server on
/// loopback) and `VIEW_SOURCE` are defined ahead of it. Each route is also tried with a web URL
/// (`allowed`), proving the route itself works, so a file that did not load was refused rather
/// than never tried. The routes take two runs, each well inside the MCP client's patience.
const READ_HELPERS: &str = r"
const tried = { gotos: {}, scripted: {}, local: {}, allowed: {} };
// A run returns at most 256 KiB. A loaded file's text leads its document, so a clipped read
// still shows it.
const clip = (text) => (typeof text === 'string' ? text.slice(0, 4000) : text);
const refusal = (error) => ({
  owner: error.owner, code: error.code, delivery: error.delivery,
  message: String(error.message).split('\n')[0],
});
const loads = async (target, url) => {
  try {
    await target.goto(url);
    return { url: target.url(), content: clip(await target.content()) };
  } catch (error) {
    return refusal(error);
  }
};
const pause = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
// Every tab's document and every frame's text: wherever a loaded file would show. `opened` is a
// popup the route's own event delivered, read even if the listing has not caught up with it.
const everything = async (opened) => {
  const tabs = [];
  const all = context.pages();
  if (opened && !all.includes(opened)) all.push(opened);
  for (const tab of all) {
    const frames = [];
    for (const frame of tab.frames()) {
      frames.push(clip(await frame.locator('body').textContent({ timeout: 2000 }).catch((error) => error.code)));
    }
    tabs.push({ url: tab.url(), content: clip(await tab.content().catch((error) => error.code)), frames });
  }
  return tabs;
};
// Tabs a route opened (popups), closed after its read so the next route starts clean.
const before = new Set(context.pages());
const closeOpened = async () => {
  for (const tab of context.pages()) if (!before.has(tab) && tab !== page) await tab.close().catch(() => {});
};
";

/// Every way a script could get a tab onto a local file by itself, and a web page's own
/// navigations from a `data:` page and a web load's redirect.
const LOCAL_READS: &str = r#"
for (const url of [...LOCAL, 'chrome://version', VIEW_SOURCE, 'about:version']) {
  tried.gotos[url] = await loads(page, url);
}
tried.javascript = await page.goto("javascript:void(document.title = 'ran')")
  .then(() => ({ returned: true }), (error) => ({ code: error.code, message: String(error.message).split('\n')[0] }));
tried.javascriptTitle = await page.title();
// The script's own navigations besides page.goto: a child frame's goto, and a page it opens.
const scripted = {
  frameGoto: async (to) => {
    await page.goto(SERVER + '/frames');
    const child = page.frames()[1];
    return child ? loads(child, to) : { frames: page.frames().length };
  },
  newPage: async (to) => {
    const opened = await context.newPage();
    try {
      return await loads(opened, to);
    } finally {
      await opened.close();
    }
  },
};
for (const [route, call] of Object.entries(scripted)) {
  for (const to of LOCAL) tried.scripted[`${route} to ${to}`] = await call(to);
  tried.allowed[`${route} to the server`] = await call(SERVER + '/page');
}
// A web page's own navigations. A loopback frame in a data: page is refused as a private
// network request, so the frame's allowed URL is a data: one. A popup has none: with no user
// gesture, the popup blocker refuses window.open whatever its URL, so that route pins the
// outcome alone.
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
    await pause(1500);
    into[`${route} to ${to}`] = await everything();
    await closeOpened();
  }
}
// A load the door allows, redirected by its server: Chromium follows the redirect, not the door.
for (const [path, into] of [['/to-hosts', tried.local], ['/to-secret', tried.local], ['/to-page', tried.allowed]]) {
  into[`redirect ${path}`] = { load: await loads(page, SERVER + path), tabs: await everything() };
}
return tried;
"#;

/// A web page's ways to a local file through the script's own click or the HTML it writes.
const WEB_PAGE_READS: &str = r#"
// The same from a web page, through the script's own click (a user gesture, so the popup blocker
// lets a new tab through) or HTML the script writes with setContent: on the web page, and on the
// blank page context.newPage opens.
const clicked = {
  link: (to) => `<a href="${to}">to</a>`,
  'link to a new tab': (to) => `<a target="_blank" href="${to}">to</a>`,
  'window.open': (to) => `<button onclick='window.open(${JSON.stringify(to)})'>to</button>`,
  'written iframe': (to) => `<iframe src="${to}"></iframe>`,
};
for (const [route, html] of Object.entries(clicked)) {
  for (const [to, into] of [...LOCAL.map((to) => [to, tried.local]), [SERVER + '/page', tried.allowed]]) {
    await page.goto(SERVER + '/page');
    await page.setContent(html(to));
    let opened = null;
    if (route === 'link') {
      await page.locator('a').click();
    } else if (route !== 'written iframe') {
      const popup = page.waitForEvent('popup', { timeout: 2000 }).catch(() => null);
      await page.locator('a, button').click();
      opened = await popup;
      if (opened) await opened.waitForLoadState().catch(() => {});
    }
    await pause(1500);
    into[`${route} on a web page to ${to}`] = await everything(opened);
    await closeOpened();
  }
}
for (const [to, into] of [...LOCAL.map((to) => [to, tried.local]), ['data:text/html,reached', tried.allowed]]) {
  const opened = await context.newPage();
  await opened.setContent(`<iframe src="${to}"></iframe>`);
  await pause(1500);
  into[`written iframe on a new page to ${to}`] = await everything();
  await opened.close();
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

/// Hosted mode hears no console messages, by design (`neo-playwright`'s `events/event_op.rs`,
/// `lease_runtime`), so the listener stays silent while the page logs every 50 ms.
#[tokio::test(flavor = "multi_thread")]
async fn a_fresh_tasks_first_call_can_be_a_console_listener_that_hosted_mode_keeps_silent()
-> TestResult {
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
async fn a_script_cannot_open_file_pages_and_web_pages_cannot_navigate_to_them() -> TestResult {
    on_browseros(|rig| Box::pin(local_navigations(rig))).await
}

#[tokio::test(flavor = "multi_thread")]
async fn the_owners_file_tab_loads_but_a_script_cannot_read_it() -> TestResult {
    on_browseros(|rig| Box::pin(owner_file_tab(rig))).await
}

#[tokio::test(flavor = "multi_thread")]
async fn a_selector_with_scope_inside_has_reaches_its_op() -> TestResult {
    on_browseros(|rig| Box::pin(scope_inside_has(rig))).await
}

#[tokio::test(flavor = "multi_thread")]
async fn a_delta_snapshot_and_its_tip_reach_the_model_over_mcp() -> TestResult {
    on_browseros(|rig| Box::pin(delta_and_tip(rig))).await
}

/// The delta tip's line, as the run reply's text ends with it.
const DELTA_TIP: &str =
    "Tip: page.ariaSnapshot({ delta: true }) returns only what changed since your last snapshot.";

/// A page long enough that one changed line is well under half of its snapshot, and a script
/// that opens it in the run's page, or finds the tab that has it.
const DELTA_PAGE: &str = r##"
const items = Array.from({ length: 40 }, (_, n) => `<li><a href="#s${n}">Story ${n}</a></li>`).join('');
const html = '<title>Delta tip</title><main><button>Collapse</button><ul>' + items + '</ul></main>';
const url = 'data:text/html,' + encodeURIComponent(html);
const tab = context.pages().find((tab) => tab.url() === url);
const p = tab ?? page;
if (!tab) await p.goto(url);
"##;

/// The text the model reads, and the run's status.
fn read(result: &CallToolResult) -> Result<(String, Value), Box<dyn Error>> {
    let text = result
        .content
        .first()
        .and_then(ContentBlock::as_text)
        .map(|content| content.text.clone())
        .ok_or("a reply has its text")?;
    let status = outcome(result)?
        .get("status")
        .cloned()
        .unwrap_or(Value::Null);
    Ok((text, status))
}

async fn delta_and_tip(rig: &Rig) -> TestResult {
    rig.until_ready().await?;
    let client = rig.client().await?;
    delta_baselines(&client).await?;
    delta_after_text(&client).await?;
    delta_tips(&client).await?;
    client.cancel().await?;
    Ok(())
}

/// `{DELTA_PAGE}` then `body`: the page's script, with its button's text set to `label` first.
fn on_delta_page(label: &str, body: &str) -> String {
    format!(
        "{DELTA_PAGE}\nawait p.evaluate((label) => {{ document.querySelector('button').textContent = label }}, {label:?})\n{body}"
    )
}

/// The delta's value, from a run that must have succeeded.
async fn delta_of(client: &Client, code: &str, task: &Value) -> Result<String, Box<dyn Error>> {
    let result = run(client, code, task).await?;
    let (text, status) = read(&result)?;
    assert_eq!(status, json!("ok"), "{result:?}");
    let first = text.lines().next().ok_or("a reply has its value")?;
    Ok(serde_json::from_str(first)?)
}

/// The baseline is the last snapshot text a run gave its caller: never one a failed run took,
/// one the script only inspected, nor one taken earlier in the same run.
async fn delta_baselines(client: &Client) -> TestResult {
    let task = json!({ "taskId": "delta-baseline" });
    let delta = "return await p.ariaSnapshot({ delta: true })";
    let first = delta_of(client, &on_delta_page("Collapse", delta), &task).await?;
    assert!(
        first.starts_with("# whole page: no earlier snapshot of this page\n- main [ref="),
        "the first delta is the whole page: {first}"
    );
    // Each of these takes the changed page and never gives it to the caller.
    let failed = run(
        client,
        &on_delta_page(
            "Expand",
            "await p.ariaSnapshot()\nthrow new Error('after the snapshot')",
        ),
        &task,
    )
    .await?;
    assert_eq!(read(&failed)?.1, json!("error"));
    let inspected = run(
        client,
        &on_delta_page("Expand", "return (await p.ariaSnapshot()).length"),
        &task,
    )
    .await?;
    assert_eq!(read(&inspected)?.1, json!("ok"), "{inspected:?}");
    let since_first = delta_of(client, &on_delta_page("Expand", delta), &task).await?;
    assert!(
        since_first
            .starts_with("# changes since your last snapshot of this tab: 1 added, 1 removed\n")
            && since_first.contains("\n-  - button \"Collapse\" [ref=")
            && since_first.contains("\n+  - button \"Expand\" [ref="),
        "still against the first run's text: {since_first}"
    );
    // A console line counts as given.
    let logged = on_delta_page("Logged", "console.log(await p.ariaSnapshot())\nreturn 1");
    run(client, &logged, &task).await?;
    let after_log = delta_of(client, &on_delta_page("Logged", delta), &task).await?;
    assert_eq!(
        after_log,
        "# no change since your last snapshot of this tab"
    );
    // One run: a full snapshot, an act, then a delta, which compares against the text stored
    // when the run started ("Logged"), not the run's own full snapshot ("Middle").
    let in_one_run = on_delta_page(
        "Middle",
        "const full = await p.ariaSnapshot()\n\
         await p.evaluate(() => { document.querySelector('button').textContent = 'Last' })\n\
         return [full.length, await p.ariaSnapshot({ delta: true })]",
    );
    let result = run(client, &in_one_run, &task).await?;
    let (text, _) = read(&result)?;
    let value: Value = serde_json::from_str(text.lines().next().ok_or("a value")?)?;
    let one_run = value.get(1).and_then(Value::as_str).unwrap_or_default();
    assert!(
        one_run.contains("\n-  - button \"Logged\" [ref=")
            && one_run.contains("\n+  - button \"Last\" [ref="),
        "against the run's starting baseline: {one_run}"
    );
    Ok(())
}

/// A page whose default snapshot leaves its text out (160 paragraphs, well over the 3,000-token
/// limit), with a button that renames itself and shows a note when clicked.
const TEXT_PAGE: &str = r##"
const paragraphs = Array.from({ length: 160 }, (_, n) => `<p>Paragraph ${n} of the article, with words enough to fill a line.</p><a href="#p${n}">Link ${n}</a>`).join('');
const html = '<title>Delta form</title><main><button onclick="this.textContent=\'Close\'; document.getElementById(\'note\').hidden=false">Open</button><p id="note" hidden>A new note</p>' + paragraphs + '</main>';
const url = 'data:text/html,' + encodeURIComponent(html);
const tab = context.pages().find((tab) => tab.url() === url);
const p = tab ?? page;
if (!tab) await p.goto(url);
"##;

/// The owner's flow (design section 7, the form of a delta): `{ text: true }` is given, a click,
/// then `{ delta: true }`, which takes the text option from the baseline and so lists only what
/// the click changed, with the page's text, instead of the whole page again.
async fn delta_after_text(client: &Client) -> TestResult {
    let task = json!({ "taskId": "delta-text-form" });
    let first_line = delta_of(
        client,
        &format!("{TEXT_PAGE}\nreturn (await p.ariaSnapshot()).split('\\n')[0]"),
        &task,
    )
    .await?;
    assert!(
        first_line.starts_with("# text left out: about "),
        "the default leaves this page's text out: {first_line}"
    );
    let with_text = delta_of(
        client,
        &format!("{TEXT_PAGE}\nreturn await p.ariaSnapshot({{ text: true }})"),
        &task,
    )
    .await?;
    assert!(with_text.contains("\n  - text: Paragraph 159 of the article"));
    let clicked = delta_of(
        client,
        &format!(
            "{TEXT_PAGE}\nawait p.getByRole('button', {{ name: 'Open' }}).click()\n\
             return await p.ariaSnapshot({{ delta: true }})"
        ),
        &task,
    )
    .await?;
    assert!(
        clicked.starts_with("# changes since your last snapshot of this tab: ")
            && clicked.contains("\n+  - button \"Close\" ")
            && clicked.contains("\n+  - text: A new note")
            && !clicked.contains("Paragraph 40 "),
        "only the click's change, in the text form: {clicked}"
    );
    assert!(clicked.len() < 2_000, "{} characters", clicked.len());
    let unchanged = delta_of(
        client,
        &format!("{TEXT_PAGE}\nreturn await p.ariaSnapshot({{ delta: true }})"),
        &task,
    )
    .await?;
    assert_eq!(
        unchanged,
        "# no change since your last snapshot of this tab"
    );
    // A text option that disagrees with the baseline's is never compared: the whole page, as
    // asked, without the page's text and without the notice.
    let without = delta_of(
        client,
        &format!("{TEXT_PAGE}\nreturn await p.ariaSnapshot({{ delta: true, text: false }})"),
        &task,
    )
    .await?;
    assert!(
        without.starts_with(
            "# whole page: too much differs from your last snapshot to list as changes\n\
             - main [ref="
        ) && !without.contains("- text: Paragraph"),
        "{without}"
    );
    Ok(())
}

/// A full snapshot that found a stored baseline ends the reply with the tip: never on a failed
/// run, three times per task, and three per tab for runs with no task.
async fn delta_tips(client: &Client) -> TestResult {
    let task = json!({ "taskId": "delta-tip" });
    let snap = |body: &str| on_delta_page("Collapse", body);
    let given = run(
        client,
        &snap("return await p.ariaSnapshot({ delta: true })"),
        &task,
    )
    .await?;
    assert!(!read(&given)?.0.contains(DELTA_TIP), "a delta needs no tip");
    let failed = run(
        client,
        &snap("await p.ariaSnapshot()\nthrow new Error('after the snapshot')"),
        &task,
    )
    .await?;
    let (text, status) = read(&failed)?;
    assert_eq!(status, json!("error"));
    assert!(!text.contains(DELTA_TIP), "never on a failed run: {text}");
    let mut tips = 0;
    for _ in 0..5 {
        let full = run(
            client,
            &snap("return (await p.ariaSnapshot()).length"),
            &task,
        )
        .await?;
        let (text, status) = read(&full)?;
        assert_eq!(status, json!("ok"), "{full:?}");
        let lines: Vec<&str> = text.lines().collect();
        if lines.last() == Some(&DELTA_TIP) {
            assert_eq!(lines.len(), 2, "the value, then the tip: {text}");
            tips += 1;
        } else {
            assert_eq!(lines.len(), 1, "the value alone: {text}");
        }
    }
    assert_eq!(tips, 3, "at most three tips per task");
    // Runs with no task count per tab: the same tab's three are its own.
    let mut untasked = 0;
    for _ in 0..4 {
        let full = run(
            client,
            &snap("return (await p.ariaSnapshot()).length"),
            &json!({}),
        )
        .await?;
        if read(&full)?.0.ends_with(DELTA_TIP) {
            untasked += 1;
        }
    }
    assert_eq!(untasked, 3, "a tab with no task has three of its own");
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn a_ref_from_an_earlier_run_says_it_is_not_from_the_tab_this_run_opened() -> TestResult {
    on_browseros(|rig| Box::pin(refs_from_elsewhere(rig))).await
}

/// The text a ref's failure gives on a tab the run opened and never snapshotted.
fn not_from_this_tab(r#ref: &str) -> String {
    format!(
        "{ref} is not from this tab, which this run opened: a ref works only in the tab whose \
         snapshot printed it. A task keeps its tab across runs: await neo.task(id)"
    )
}

/// A run's failure line: the text the model reads up to the call log.
async fn failure_of(client: &Client, code: &str, meta: &Value) -> Result<String, Box<dyn Error>> {
    let result = run(client, code, meta).await?;
    let (text, status) = read(&result)?;
    assert_eq!(status, json!("error"), "{result:?}");
    Ok(text.lines().next().unwrap_or_default().to_owned())
}

/// Design section 5, "A ref used in a tab this run opened": each run with no task starts on a
/// new tab, so a ref from the last run cannot be clicked there, and the message says why and how
/// to keep a tab. A ref that went stale on a tab that had a snapshot, and one used on a task's tab
/// this run did not open, keep the ordinary message.
async fn refs_from_elsewhere(rig: &Rig) -> TestResult {
    rig.until_ready().await?;
    let client = rig.client().await?;
    let none = json!({});
    let printed = delta_of(
        &client,
        "await page.goto('data:text/html,' + encodeURIComponent('<main><button>One</button>\
         <button>Two</button><iframe srcdoc=\"<button>Inside</button>\"></iframe></main>'))\n\
         return await page.ariaSnapshot()",
        &none,
    )
    .await?;
    // The ref on the line that prints `pattern`.
    let ref_of = |pattern: &str| -> Result<String, Box<dyn Error>> {
        let line = printed
            .lines()
            .find(|line| line.contains(pattern))
            .ok_or(format!("{pattern} in {printed}"))?;
        let r#ref = line
            .split("[ref=")
            .nth(1)
            .and_then(|tail| tail.split(']').next())
            .ok_or(format!("a ref on {line}"))?;
        Ok(r#ref.to_owned())
    };
    let two = ref_of("button \"Two\"")?;
    let inside = ref_of("button \"Inside\"")?;
    assert!(inside.starts_with('f'), "a frame's ref: {inside}");
    // The next run with no task is on a new tab.
    let click = |r#ref: &str| format!("await page.getByRef('{ref}').click({{ timeout: 2000 }})");
    assert_eq!(
        failure_of(&client, &click(&two), &none).await?,
        format!(
            "website / element_detached: locator.click: {}",
            not_from_this_tab(&two)
        )
    );
    assert_eq!(
        failure_of(&client, &click(&inside), &none).await?,
        format!(
            "script / invalid_input: locator.click: {}",
            not_from_this_tab(&inside)
        )
    );
    // The same tab once this run snapshotted it: no longer provably another tab's ref.
    assert_eq!(
        failure_of(
            &client,
            &format!("await page.ariaSnapshot()\n{}", click("e999")),
            &none
        )
        .await?,
        "website / element_detached: locator.click: e999 is not on the page any more; take a \
         new snapshot"
    );
    // A ref that really went stale on a tab with a snapshot.
    let stale = "await page.setContent('<button>Gone soon</button>')\n\
                 const ref = /\\[ref=(e\\d+)\\]/.exec(await page.ariaSnapshot())[1]\n\
                 await page.setContent('<p>Replaced</p>')\n\
                 await page.getByRef(ref).click({ timeout: 2000 })";
    let failure = failure_of(&client, stale, &none).await?;
    assert!(
        failure.starts_with("website / element_detached: locator.click: e")
            && failure.ends_with(" is not on the page any more; take a new snapshot"),
        "{failure}"
    );
    // A task's tab, opened by an earlier run and never snapshotted, keeps the ordinary text too.
    let task = delta_of(
        &client,
        "const task = await neo.createTask('refs elsewhere')\n\
         await neo.task(task.id)\n\
         await page.title()\n\
         return task.id",
        &none,
    )
    .await?;
    assert_eq!(
        failure_of(
            &client,
            &format!("await neo.task({task:?})\n{}", click("e999")),
            &none
        )
        .await?,
        "website / element_detached: locator.click: e999 is not on the page any more; take a \
         new snapshot"
    );
    client.cancel().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn a_picked_tasks_page_has_its_tabs_url_before_any_action() -> TestResult {
    on_browseros(|rig| Box::pin(task_page_at_once(rig))).await
}

/// The value of a run that must have succeeded.
async fn value_of(client: &Client, code: &str, meta: &Value) -> Result<Value, Box<dyn Error>> {
    let result = run(client, code, meta).await?;
    assert_eq!(
        outcome(&result)?.get("status"),
        Some(&json!("ok")),
        "{result:?}"
    );
    result
        .structured_content
        .as_ref()
        .and_then(|reply| reply.get("value").cloned())
        .ok_or_else(|| format!("the script returns: {result:?}").into())
}

/// The host locks `tab` with `reads`, runs `code`, and releases the lock. Returns the run's value.
async fn under_lock(
    rig: &Rig,
    client: &Client,
    tab: &Value,
    reads: &str,
    code: &str,
) -> Result<Value, Box<dyn Error>> {
    let take = json!({ "tabId": tab, "reads": reads, "ttlSeconds": 600 });
    let taken = rig
        .host(
            "POST",
            "/v1/locks",
            Some(browseros_rig::HOST_TOKEN),
            Some(&take),
        )
        .await?;
    assert_eq!(
        taken.status,
        200,
        "the host locks the tab: {}",
        taken.text()
    );
    let lock = taken.json()?;
    let id = lock
        .get("id")
        .and_then(Value::as_str)
        .ok_or("a lock has an id")?;
    let value = value_of(client, code, &json!({})).await;
    let released = rig
        .host(
            "DELETE",
            &format!("/v1/locks/{id}"),
            Some(browseros_rig::HOST_TOKEN),
            None,
        )
        .await?;
    assert_eq!(released.status, 200, "{}", released.text());
    value
}

/// The viewer's report (night 4e58b2527, VIEWER.md 4.5): after `await neo.task(id)` the built-in
/// `page` read `about:blank` until its first awaited call. Playwright's synchronous getters are
/// right as soon as a script holds a page: here on the next line after the pick, and on the first
/// line of a run whose host names the task in `_meta`. A tab the host holds with blocked reads
/// still binds `page` and gives it nothing to read; allowed reads give it the URL.
async fn task_page_at_once(rig: &Rig) -> TestResult {
    rig.until_ready().await?;
    let client = rig.client().await?;
    let none = json!({});
    let made = value_of(
        &client,
        "const task = await neo.createTask('url at once')\n\
         await page.goto('data:text/html,' + encodeURIComponent('<title>At once</title><h1>Task tab</h1>'))\n\
         return { id: task.id, url: page.url(), tab: page.tabId }",
        &none,
    )
    .await?;
    let field = |name: &str| made.get(name).cloned().unwrap_or(Value::Null);
    let (id, url, tab) = (field("id"), field("url"), field("tab"));
    assert!(
        url.as_str().is_some_and(|url| url.starts_with("data:")),
        "{made}"
    );
    let getters = "return { url: page.url(), frame: page.mainFrame().url(), tab: page.tabId, \
                   task: page.taskId, closed: page.isClosed(), \
                   listed: context.pages().includes(page), viewport: page.viewportSize() }";
    let picked = value_of(
        &client,
        &format!(
            "const task = await neo.task({id})\n\
             const before = (() => {{ {getters} }})()\n\
             const title = await page.title()\n\
             const after = (() => {{ {getters} }})()\n\
             return {{ before, title, after, inTask: task.pages().includes(page) }}"
        ),
        &none,
    )
    .await?;
    let at = |path: &str| picked.pointer(path).cloned().unwrap_or(Value::Null);
    assert_eq!(
        (at("/before/url"), at("/before/frame")),
        (url.clone(), url.clone()),
        "page.url() is the task tab's on the line after the pick: {picked}"
    );
    assert_eq!(
        (
            at("/before/tab"),
            at("/before/task"),
            at("/before/closed"),
            at("/before/listed"),
            at("/inTask"),
        ),
        (
            tab.clone(),
            id.clone(),
            json!(false),
            json!(true),
            json!(true)
        ),
        "{picked}"
    );
    assert_eq!(
        at("/before"),
        at("/after"),
        "an awaited call changes no getter: {picked}"
    );
    assert_eq!(at("/title"), json!("At once"), "{picked}");

    // A host task: its tab is the run's page from the run's first line.
    let host = json!({ "taskId": "url-at-once-host" });
    let hosted = value_of(
        &client,
        "await page.goto('data:text/html,' + encodeURIComponent('<title>Host task</title>'))\n\
         return page.url()",
        &host,
    )
    .await?;
    assert_eq!(
        value_of(&client, "return page.url()", &host).await?,
        hosted,
        "the host task's tab, before any op"
    );

    let pick = format!("await neo.task({id})\nreturn [page.url(), page.tabId]");
    assert_eq!(
        under_lock(rig, &client, &tab, "block", &pick).await?,
        json!(["about:blank", tab]),
        "blocked reads: `page` is the tab, with nothing of it to read"
    );
    assert_eq!(
        under_lock(rig, &client, &tab, "allow", &pick).await?,
        json!([url, tab]),
        "allowed reads: the tab's URL"
    );
    client.cancel().await?;
    Ok(())
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
    assert_eq!(
        value.get("heard"),
        Some(&json!([])),
        "a hosted console listener is accepted and hears nothing: {reply}"
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
             - iframe [ref]:\n  - text: Inside the frame\n  \
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
    // A synthetic local file proves a read came from disk rather than a web fixture.
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
    let mut tried = Value::Null;
    for (script, task) in [
        (LOCAL_READS, "local-reads"),
        (WEB_PAGE_READS, "web-page-reads"),
    ] {
        let code = format!(
            "const LOCAL = {};\nconst SERVER = {};\nconst VIEW_SOURCE = {};\n{READ_HELPERS}\n{script}",
            json!(local),
            json!(server),
            json!(view_source),
        );
        let reply = reply_of(rig, &code, task).await?;
        let value = reply
            .get("value")
            .ok_or("the script returns what it tried")?;
        merge(&mut tried, value);
    }
    let tried = &tried;

    assert_script_routes_refused(tried, &local, &view_source)?;
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
        22,
        "five data: routes, four web-page routes, a new page's iframe and a redirect, each to \
         both files: {tried}"
    );
    // Nothing a script tried, by any route, read either file.
    for text in texts(tried) {
        for secret in &secrets {
            assert!(
                !text.contains(secret),
                "a script read {secret:?} from a local file: {text}"
            );
        }
    }
    let allowed = tried
        .get("allowed")
        .and_then(Value::as_object)
        .ok_or_else(|| format!("the routes to allowed URLs: {tried}"))?;
    assert_eq!(
        allowed.len(),
        12,
        "each route once to a URL Neo allows: {tried}"
    );
    for (route, read) in allowed {
        assert!(
            texts(read).iter().any(|text| text.contains("reached")),
            "{route} reaches a URL Neo allows, so the same route to a file was really tried: {read}"
        );
    }
    Ok(())
}

/// A local page that shows `marker` and moves on to `to` once loaded and a moment later, so the
/// `file:` page stays in its tab's history as an entry of its own rather than one the next load
/// replaces.
fn hop_page(marker: &str, to: &str) -> String {
    format!(
        "<title>hop</title><p>{marker} behind the web page</p><script>addEventListener('load', () \
         => setTimeout(() => {{ location.href = {}; }}, 300))</script>",
        json!(to)
    )
}

/// Every read of the owner's file: tab is refused, the step back onto a file: entry too, and
/// whatever an op took after its tab stepped onto one is withheld.
fn assert_owner_reads_refused(seen: &Value) {
    for read in [
        "content",
        "title",
        "text",
        "evaluate",
        "snapshot",
        "screenshot",
        "fetch",
        "iframe",
        "listing",
    ] {
        assert_file_refusal(
            seen.pointer(&format!("/owner/{read}")),
            &format!("the owner's file: tab, {read}"),
        );
    }
    assert_file_refusal(seen.pointer("/back/goBack"), "goBack to a file: entry");
    assert_withheld(
        seen.pointer("/midOp/read"),
        "a read that the page's own step back moved onto a file: page",
    );
    assert_withheld(
        seen.pointer("/callback/evaluate"),
        "a callback whose element turned up in a file: page",
    );
}

/// Opens `urls` as the owner's own tabs, outside any script, and waits until the browser lists
/// the owner's file page at `owner_url` and every hop page on the web page it moves on to.
/// Returns the browser's listing.
async fn open_owner_tabs(
    rig: &Rig,
    urls: &[String],
    owner_url: &str,
) -> Result<String, Box<dyn Error>> {
    for url in urls {
        put(rig.cdp_port(), &format!("/json/new?{url}")).await?;
    }
    time::timeout(Duration::from_secs(15), async {
        loop {
            let listed = get(rig.cdp_port(), "/json/list").await?;
            if listed.contains(&format!("\"url\": \"{owner_url}\""))
                && listed.contains("owner file")
                && listed.contains("/page?back")
                && listed.contains("/page?history")
                && listed.contains("/page?midop")
                && listed.contains("/page?callback")
            {
                return Ok::<_, Box<dyn Error>>(listed);
            }
            time::sleep(Duration::from_millis(200)).await;
        }
    })
    .await
    .map_err(|_elapsed| "the owner's file tabs did not load and move on")?
}

/// The script's own routes to a local file are refused, as other unsupported schemes are.
fn assert_script_routes_refused(tried: &Value, local: &[String], view_source: &str) -> TestResult {
    let refused = json!({ "owner": "script", "code": "invalid_input", "delivery": "not_sent" });
    for url in ["chrome://version", view_source] {
        let tried_url = tried.get("gotos").and_then(|gotos| gotos.get(url));
        assert_eq!(
            tried_url.map(without_message),
            Some(refused.clone()),
            "other unsupported schemes retain their refusal: {tried}"
        );
    }
    for url in local {
        assert_file_refusal(
            tried.get("gotos").and_then(|gotos| gotos.get(url)),
            &format!("page.goto to {url}"),
        );
    }
    let scripted = tried
        .get("scripted")
        .and_then(Value::as_object)
        .ok_or_else(|| format!("the script's other navigations: {tried}"))?;
    assert_eq!(
        scripted.len(),
        4,
        "frame.goto and newPage, each to both files: {tried}"
    );
    for (route, result) in scripted {
        assert_file_refusal(Some(result), route);
    }
    Ok(())
}

/// The door's refusal of a `file:` page, as a script catches it: the script's to fix, nothing
/// sent, and a message that says what is blocked and what to use instead.
fn assert_file_refusal(result: Option<&Value>, route: &str) {
    let result = result.unwrap_or(&Value::Null);
    assert_eq!(
        without_message(result),
        json!({ "owner": "script", "code": "invalid_input", "delivery": "not_sent" }),
        "{route} is refused before anything reaches the tab: {result}"
    );
    assert!(
        result
            .get("message")
            .and_then(Value::as_str)
            .is_some_and(|message| message.contains(FILE_REFUSAL)),
        "{route}'s refusal says what is blocked: {result}"
    );
}

/// The door's answer to an op whose tab moved onto a `file:` page while it ran: whatever the op
/// read there is withheld. Its delivery is the op's own, since it may have acted before the move.
fn assert_withheld(result: Option<&Value>, route: &str) {
    let result = result.unwrap_or(&Value::Null);
    assert_eq!(
        (result.get("owner"), result.get("code")),
        (Some(&json!("script")), Some(&json!("invalid_input"))),
        "{route} is withheld: {result}"
    );
    assert!(
        result
            .get("message")
            .and_then(Value::as_str)
            .is_some_and(|message| message.contains(FILE_REFUSAL)),
        "{route}'s refusal says what is blocked: {result}"
    );
}

/// What every refusal of a `file:` page says, whatever the route.
const FILE_REFUSAL: &str = "a script cannot open file: pages; use a URL, or page.setContent";

/// A caught error's owner, code and delivery, without its message.
fn without_message(result: &Value) -> Value {
    let mut result = result.clone();
    if let Some(fields) = result.as_object_mut() {
        fields.remove("message");
    }
    result
}

/// A local HTML file the owner opens in a tab of their own, and another whose page goes on to a
/// web page, which leaves a `file:` entry in that tab's history. The script then finds both in
/// `context.pages()`. `OWNER_URL`, `SECRET_URL`, `DIR_URL` and `SERVER` are defined ahead of it.
const OWNER_READS: &str = r"
const clip = (text) => (typeof text === 'string' ? text.slice(0, 4000) : text);
const refusal = (error) => ({
  owner: error.owner, code: error.code, delivery: error.delivery,
  message: String(error.message).split('\n')[0],
});
const attempt = async (call) => {
  try {
    return { value: clip(await call()) };
  } catch (error) {
    return refusal(error);
  }
};
const pause = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
const listed = context.pages().map((tab) => tab.url());
const owner = context.pages().find((tab) => tab.url() === OWNER_URL);
const back = context.pages().find((tab) => tab.url() === SERVER + '/page?back');
const historyBack = context.pages().find((tab) => tab.url() === SERVER + '/page?history');
const midOp = context.pages().find((tab) => tab.url() === SERVER + '/page?midop');
const callback = context.pages().find((tab) => tab.url() === SERVER + '/page?callback');
if (!owner || !back || !historyBack || !midOp || !callback) return { listed };
const seen = { listed, owner: { url: owner.url() }, back: {}, historyBack: {}, midOp: {}, callback: {} };
seen.owner.content = await attempt(() => owner.content());
seen.owner.title = await attempt(() => owner.title());
seen.owner.text = await attempt(() => owner.locator('body').textContent({ timeout: 2000 }));
seen.owner.evaluate = await attempt(() => owner.evaluate(() => document.body.innerText));
seen.owner.snapshot = await attempt(() => owner.ariaSnapshot());
seen.owner.screenshot = await attempt(async () => (await owner.screenshot()).length);
// What a file: page itself reaches, if a script could drive it: another file by fetch and in an
// iframe, and a directory's listing (last: it leaves the page).
seen.owner.fetch = await attempt(() => owner.evaluate((url) => fetch(url).then((response) => response.text()), SECRET_URL));
seen.owner.iframe = await attempt(async () => {
  await owner.evaluate((url) => {
    const frame = document.createElement('iframe');
    frame.src = url;
    document.body.append(frame);
  }, SECRET_URL);
  await pause(1000);
  return JSON.stringify(await Promise.all(owner.frames().map((frame) => frame.locator('body').textContent({ timeout: 2000 }).catch((error) => error.code))));
});
seen.owner.listing = await attempt(async () => {
  await owner.evaluate((url) => { location.href = url; }, DIR_URL);
  await pause(1000);
  return owner.content();
});
// The tab's own history holds a file: page: Playwright's step back, and the page's own.
seen.back.goBack = await attempt(async () => {
  await back.goBack();
  return { url: back.url(), content: clip(await back.content()) };
});
seen.back.after = await attempt(async () => ({ url: back.url(), content: clip(await back.content()) }));
// A read already waiting when the page steps itself back onto the file: entry (here from a
// clicked button): the locator re-resolves in the new document, so the door checks the tab
// again when the read ends.
await midOp.setContent('<button onclick=history.back()>back</button>');
const waiting = attempt(() => midOp.locator('p', { hasText: 'behind' }).textContent({ timeout: 6000 }));
seen.midOp.click = await attempt(() => midOp.locator('button').click());
seen.midOp.read = await waiting;
// The same for a main-world callback: it would mark the page's title, but the guard refuses to
// run it in the file: document its element turned up in.
await callback.setContent('<button onclick=history.back()>back</button>');
const marking = attempt(() => callback.locator('p', { hasText: 'behind' })
  .evaluate((element) => { document.title = 'callback ran'; return element.textContent; }, null, { timeout: 6000 }));
seen.callback.click = await attempt(() => callback.locator('button').click());
seen.callback.evaluate = await marking;
seen.historyBack.step = await attempt(async () => {
  await historyBack.evaluate(() => history.back());
  await pause(1500);
  return { url: historyBack.url(), content: clip(await historyBack.content()) };
});
return seen;
";

async fn owner_file_tab(rig: &Rig) -> TestResult {
    let dir = tempfile::tempdir()?;
    let marker = format!("neo-owner-file-{}", RunId::new());
    let secret = dir.path().join("host-token");
    fs::write(&secret, &marker)?;
    let server = format!("http://127.0.0.1:{}", serve_pages(&secret).await?);
    let owner = dir.path().join("owner.html");
    fs::write(
        &owner,
        format!("<title>owner file</title><p>{marker} in the owner's page</p>"),
    )?;
    let hops = ["back", "history", "midop", "callback"];
    let [back, history_back, mid_op, callback] =
        hops.map(|to| dir.path().join(format!("{to}.html")));
    for (path, to) in [&back, &history_back, &mid_op, &callback]
        .into_iter()
        .zip(hops)
    {
        fs::write(path, hop_page(&marker, &format!("{server}/page?{to}")))?;
    }
    let file_url = |path: &Path| format!("file://{}", path.display());

    rig.until_ready().await?;
    let owner_url = file_url(&owner);
    let opened = open_owner_tabs(
        rig,
        &[
            owner_url.clone(),
            file_url(&back),
            file_url(&history_back),
            file_url(&mid_op),
            file_url(&callback),
        ],
        &owner_url,
    )
    .await?;
    assert!(
        opened.contains("owner file"),
        "the owner's file: tab loads, outside a script: {opened}"
    );

    let code = format!(
        "const OWNER_URL = {};\nconst SECRET_URL = {};\nconst DIR_URL = {};\nconst SERVER = {};\n{OWNER_READS}",
        json!(owner_url),
        json!(file_url(&secret)),
        json!(format!("file://{}/", dir.path().display())),
        json!(server),
    );
    let reply = reply_of(rig, &code, "owner-file-tab").await?;
    let seen = reply.get("value").ok_or("the script returns what it saw")?;

    // What a script sees: the tab is listed, with its URL, and nothing else.
    assert!(
        seen.get("listed")
            .and_then(Value::as_array)
            .is_some_and(|listed| listed.contains(&json!(owner_url))),
        "context.pages() lists the owner's file: tab: {seen}"
    );
    assert_eq!(
        seen.pointer("/owner/url"),
        Some(&json!(owner_url)),
        "{seen}"
    );
    assert_owner_reads_refused(seen);
    assert_eq!(
        seen.pointer("/back/after/value/url"),
        Some(&json!(format!("{server}/page?back"))),
        "the refused step back left the tab on its web page: {seen}"
    );
    for text in texts(seen) {
        assert!(
            !text.contains(&marker),
            "a script read the owner's local file: {text}"
        );
    }
    let listed = get(rig.cdp_port(), "/json/list").await?;
    assert!(
        listed.contains(&format!("\"url\": \"{owner_url}\"")),
        "the owner's file: tab still shows its file: {listed}"
    );
    // Read outside any script: the callback never ran in the file: page it reached.
    assert!(
        !listed.contains("callback ran"),
        "the callback never ran in the file: page: {listed}"
    );
    Ok(())
}

/// `from`'s fields into `into`, object fields merged however deep.
fn merge(into: &mut Value, from: &Value) {
    match (into, from) {
        (Value::Object(into), Value::Object(from)) => {
            for (key, value) in from {
                merge(into.entry(key.clone()).or_insert(Value::Null), value);
            }
        }
        (into, from) => from.clone_into(into),
    }
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

/// A web server on loopback for the cases Neo allows and for the redirects: `/page` (with any
/// query) says `reached`, `/frames` holds `/page` in an iframe, and `/to-hosts`, `/to-secret` and
/// `/to-page` redirect to `/etc/hosts`, `secret` and `/page`. Returns its port; it serves until
/// the test's runtime ends.
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
                    Some("/frames") => {
                        let body = "<title>frames</title><iframe src=\"/page\"></iframe>";
                        format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                            body.len()
                        )
                    }
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
