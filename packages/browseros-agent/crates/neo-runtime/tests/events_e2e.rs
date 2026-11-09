//! The run's event pipe on a real BrowserOS neo (`src/events.rs`, `src/chromium_bridge/page_events.rs`,
//! neo-quickjs-injected `events.ts`): listeners run before the call that caused their event
//! resolves, an idle script still gets its events, what the page logged before the run is history
//! and never an event, a dialog a listener leaves open is answered when the run ends, an uncaught
//! listener error ends the run with it, and a full queue drops events with one log note.
#![expect(
    clippy::disallowed_methods,
    reason = "the test drives an independent CDP session as the browser oracle"
)]

#[expect(
    dead_code,
    reason = "the shared fixture also provides navigation helpers"
)]
#[path = "support/browser.rs"]
mod browser;

use std::time::Duration;

use browser::{Fixture, Tab};
use neo_runtime::{RunOutcome, RunStatus};
use serde_json::{Value, json};

const EVENTS: &str = r#"<!doctype html><html><body>
<div id="prompt-result"></div><div id="confirm-result"></div><div id="alert-result"></div>
<button id="log" onclick="console.log('clicked', 1)">log</button>
<button id="format" onclick="console.warn('careful', 5, {a: 1, b: 'x'}, [1, 2], null, undefined)">format</button>
<button id="later" onclick="setTimeout(() => console.log('later'), 100)">later</button>
<button id="alert" onclick="alert('hi'); document.getElementById('alert-result').textContent = 'answered'">alert</button>
<button id="prompt" onclick="document.getElementById('prompt-result').textContent = JSON.stringify(prompt('Name?', 'anon'))">prompt</button>
<button id="confirm" onclick="document.getElementById('confirm-result').textContent = JSON.stringify(confirm('Sure?'))">confirm</button>
<button id="throw" onclick="throw new Error('boom')">throw</button>
<button id="flood" onclick="const big = 'x'.repeat(1 << 20); for (let i = 0; i < 24; i++) console.log(big); history.pushState({}, '', '#flooded')">flood</button>
<script>console.log('before run')</script>
</body></html>"#;

/// A top document on 127.0.0.1 with a frame from localhost, another site, which Chromium runs in
/// its own process and Neo reaches on its own CDP session.
const OOPIF_TOP: &str = r"<!doctype html><html><body>
<script>console.log('top loaded');
const frame = document.createElement('iframe');
frame.id = 'child';
frame.src = 'http://localhost:' + location.port + '/oopif-child.html';
document.body.append(frame);</script>
</body></html>";

const OOPIF_CHILD: &str = r#"<!doctype html><html><body>
<button id="log" onclick="console.log('child click')">log</button>
<button id="throw" onclick="throw new RangeError('child boom')">throw</button>
<script>console.log('child loaded')</script>
</body></html>"#;

async fn open() -> Option<Tab> {
    let binary = browser::e2e_browser()?;
    Some(
        Tab::open(
            binary,
            Fixture {
                pages: vec![("/events.html".into(), EVENTS.into())],
                ..Fixture::default()
            },
            "/events.html",
        )
        .await,
    )
}

async fn run(tab: &Tab, code: &str, timeout: Duration) -> RunOutcome {
    tab.start_run_within(code.to_string(), timeout)
        .await
        .await
        .unwrap_or_else(|error| panic!("run task: {error}"))
}

fn value(outcome: &RunOutcome) -> &Value {
    assert_eq!(outcome.status, RunStatus::Ok, "{outcome:#?}");
    outcome.value.as_ref().unwrap_or(&Value::Null)
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_listeners_see_events_before_the_call_that_caused_them_resolves() {
    let Some(tab) = open().await else {
        return;
    };
    let outcome = run(
        &tab,
        r"const seen = []
page.on('console', m => seen.push([m.type(), m.text()]))
await page.click('#log')
const atClick = seen.slice()
const [waited] = await Promise.all([page.waitForEvent('console'), page.click('#later')])
await page.click('#format')
const formatted = seen[seen.length - 1]
const location = waited.location()
const dialogs = []
page.on('dialog', async dialog => {
  dialogs.push([dialog.type(), dialog.message(), dialog.defaultValue(), dialog.page() === page])
  if (dialog.type() === 'prompt') await dialog.accept('typed')
  else await dialog.dismiss()
})
await page.click('#prompt')
await page.click('#confirm')
const errors = []
page.on('pageerror', error => errors.push([error.name, error.message, error instanceof Error]))
const web = []
context.on('weberror', error => web.push([error.page() === page, error.error().message]))
await page.click('#throw')
return {
  atClick,
  waited: [waited.type(), waited.text(), waited.page() === page, waited.worker()],
  location: [typeof location.url, location.lineNumber === location.line, typeof waited.timestamp()],
  formatted,
  dialogs,
  prompt: await page.textContent('#prompt-result'),
  confirm: await page.textContent('#confirm-result'),
  errors,
  web,
}",
        Duration::from_secs(30),
    )
    .await;
    tab.close().await;
    assert_eq!(
        value(&outcome),
        &json!({
            "atClick": [["log", "clicked 1"]],
            "waited": ["log", "later", true, null],
            "location": ["string", true, "number"],
            "formatted": ["warning", "careful 5 {a: 1, b: x} [1, 2] null undefined"],
            "dialogs": [
                ["prompt", "Name?", "anon", true],
                ["confirm", "Sure?", "", true],
            ],
            "prompt": "\"typed\"",
            "confirm": "false",
            "errors": [["Error", "boom", true]],
            "web": [[true, "boom"]],
        }),
        "events reach their listeners before the call that caused them resolves"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_an_out_of_process_frame_reports_its_console_to_the_page() {
    let Some(binary) = browser::e2e_browser() else {
        return;
    };
    let tab = Tab::open(
        binary,
        Fixture {
            pages: vec![
                ("/oopif-top.html".into(), OOPIF_TOP.into()),
                ("/oopif-child.html".into(), OOPIF_CHILD.into()),
            ],
            ..Fixture::default()
        },
        "/oopif-top.html",
    )
    .await;
    let outcome = run(
        &tab,
        r"const seen = []
page.on('console', m => seen.push([m.text(), m.page() === page, m.location().url.endsWith('/oopif-child.html')]))
const errors = []
page.on('pageerror', e => errors.push([e.name, e.message]))
const child = page.frameLocator('#child')
await child.locator('#log').click()
const atClick = seen.slice()
await child.locator('#throw').click()
const history = (await page.consoleMessages()).map(m => m.text())
return { atClick, errors, history }",
        Duration::from_secs(30),
    )
    .await;
    // The frame really is out of process: it is a target of its own.
    let targets = tab
        .cdp
        .send(
            "Target.getTargets",
            json!({ "filter": [{ "type": "iframe" }] }),
            None,
        )
        .await
        .unwrap_or_else(|error| panic!("Target.getTargets: {error}"));
    tab.close().await;
    let out_of_process = targets["targetInfos"].as_array().is_some_and(|targets| {
        targets.iter().any(|target| {
            target["url"]
                .as_str()
                .is_some_and(|url| url.ends_with("/oopif-child.html"))
        })
    });
    assert!(
        out_of_process,
        "the child frame is not out of process: {targets}"
    );
    assert_eq!(
        value(&outcome),
        &json!({
            "atClick": [["child click", true, true]],
            "errors": [["RangeError", "child boom"]],
            "history": ["top loaded", "child loaded", "child click"],
        }),
        "the frame's messages are the page's; its load-time message is history"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_messages_logged_before_the_run_are_history_and_not_events() {
    let Some(tab) = open().await else {
        return;
    };
    let outcome = run(
        &tab,
        r"const seen = []
page.on('console', m => seen.push(m.text()))
const before = (await page.consoleMessages()).map(m => m.text())
await page.click('#log')
const after = (await page.consoleMessages()).map(m => m.text())
await page.clearConsoleMessages()
const cleared = (await page.consoleMessages()).length
return { seen, before, after, cleared }",
        Duration::from_secs(30),
    )
    .await;
    assert_eq!(
        value(&outcome),
        &json!({
            "seen": ["clicked 1"],
            "before": ["before run"],
            "after": ["before run", "clicked 1"],
            "cleared": 0,
        }),
        "the enable's replay is history; a click right after the subscription is live"
    );
    // A second run on the same tab: its history starts again from what V8 kept.
    let outcome = run(
        &tab,
        r"return (await page.consoleMessages()).map(m => m.text())",
        Duration::from_secs(30),
    )
    .await;
    tab.close().await;
    assert_eq!(
        value(&outcome),
        &json!(["before run", "clicked 1"]),
        "a later run reads the document's kept messages"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_a_dialog_a_listener_left_open_is_answered_when_the_run_ends() {
    let Some(tab) = open().await else {
        return;
    };
    let outcome = run(
        &tab,
        r"page.on('dialog', () => {})
page.click('#alert').catch(() => {})
const dialog = await page.waitForEvent('dialog')
return dialog.message()",
        Duration::from_secs(30),
    )
    .await;
    assert_eq!(value(&outcome), &json!("hi"), "{outcome:#?}");
    // The tab is not left frozen: the next run reads it at once.
    let outcome = run(
        &tab,
        r"return await page.textContent('#alert-result')",
        Duration::from_secs(10),
    )
    .await;
    tab.close().await;
    assert_eq!(value(&outcome), &json!("answered"), "{outcome:#?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_an_alert_without_listener_never_hangs_a_click() {
    let Some(tab) = open().await else {
        return;
    };
    let outcome = run(
        &tab,
        r"page.on('console', () => {})
await page.click('#alert')
return await page.textContent('#alert-result')",
        Duration::from_secs(20),
    )
    .await;
    tab.close().await;
    assert_eq!(value(&outcome), &json!("answered"));
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_a_dialog_answer_never_waits_for_an_enable_the_dialog_holds() {
    let Some(tab) = open().await else {
        return;
    };
    // The console subscription's Runtime.enable starts while the alert blocks the renderer; the
    // listener's answer must not wait for it.
    let outcome = run(
        &tab,
        r"page.on('dialog', async dialog => {
  page.on('console', () => {})
  await dialog.accept()
})
await page.click('#alert')
return await page.textContent('#alert-result')",
        Duration::from_secs(30),
    )
    .await;
    tab.close().await;
    assert_eq!(value(&outcome), &json!("answered"));
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_an_uncaught_listener_error_ends_the_run_with_it() {
    let Some(tab) = open().await else {
        return;
    };
    let outcome = run(
        &tab,
        r"page.on('console', () => { throw new TypeError('listener boom') })
await page.click('#log')
await new Promise(resolve => setTimeout(resolve, 5000))
return 'kept running'",
        Duration::from_secs(30),
    )
    .await;
    let async_outcome = run(
        &tab,
        r"page.on('console', async () => { throw new RangeError('async boom') })
await page.click('#log')
await new Promise(resolve => setTimeout(resolve, 5000))
return 'kept running'",
        Duration::from_secs(30),
    )
    .await;
    tab.close().await;
    for (outcome, name, message) in [
        (&outcome, "TypeError", "listener boom"),
        (&async_outcome, "RangeError", "async boom"),
    ] {
        assert_eq!(outcome.status, RunStatus::Error, "{outcome:#?}");
        let error = outcome
            .error
            .as_ref()
            .unwrap_or_else(|| panic!("{outcome:#?}"));
        assert_eq!(
            (error.name.as_str(), error.message.as_str()),
            (name, message)
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_a_full_queue_drops_events_and_notes_it_once() {
    let Some(tab) = open().await else {
        return;
    };
    // The page logs 24 MiB, then marks its URL. The script never yields meanwhile: it blocks in
    // the synchronous `url()` getter until the mark, then spins a moment for the last messages
    // in flight, so nothing drains the queue past its 16 MiB.
    let outcome = run(
        &tab,
        r"let count = 0
page.on('console', () => count++)
const clicking = page.click('#flood')
while (!page.url().endsWith('#flooded')) {}
const end = Date.now() + 1000
while (Date.now() < end) {}
await clicking
await new Promise(resolve => setTimeout(resolve, 200))
return count",
        Duration::from_mins(1),
    )
    .await;
    tab.close().await;
    let count = value(&outcome).as_u64().unwrap_or(0);
    assert!(
        (1..24).contains(&count),
        "some delivered, some dropped: {count}"
    );
    let notes: Vec<&String> = outcome
        .logs
        .iter()
        .filter(|line| line.contains("neo-runtime dropped"))
        .collect();
    assert_eq!(notes.len(), 1, "{:#?}", outcome.logs);
    let dropped = 24 - count;
    let unit = if dropped == 1 { "event" } else { "events" };
    assert!(
        notes[0].contains(&format!("dropped {dropped} {unit}:")),
        "{notes:?}"
    );
}
