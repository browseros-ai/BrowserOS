//! Runner N (R13): Playwright scripts as agents write them, run by the real prelude in QuickJS
//! (`tests/support/js_runner.rs`), with every op going through `Playwright::invoke`. This is the
//! end-to-end check of the wire contract: wire-examples.json proves the inputs decode, and these
//! scripts prove the outputs read back as Playwright's values: navigation responses, reads, the
//! default and v1 snapshot texts and refs from them, the frame tree, popups through a wait and
//! through the event pump, and page handles that keep their identity.
//!
//! The offline test needs no browser. The live test needs a throwaway BrowserOS with CDP on
//! `BROWSEROS_CDP_PORT` (default 9005): `cargo test -p neo-playwright --test prelude_live
//! -- --ignored`. It serves its fixtures from `tests/data/prelude/` and closes the tabs it opens.

#[path = "support/fixture_server.rs"]
#[expect(
    dead_code,
    reason = "the nested-frame fixture and scripts serve frames_live"
)]
mod fixture_server;
#[path = "support/js_runner.rs"]
mod js_runner;
#[path = "support/live_run.rs"]
#[expect(
    dead_code,
    reason = "Runner N drives pages through scripts, not through live_run's direct helpers"
)]
mod live_run;

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;
use std::{env, fs, mem};

use browseros_core::TabId;
use fixture_server::{FixtureServer, Page};
use futures_util::future::BoxFuture;
use js_runner::OpHost;
use live_run::{LiveRun, TestPage, TestResult};
use neo_playwright::{ErrorCode, PlaywrightError};
use serde_json::{Value, json};
use tokio::sync::Semaphore;
use tokio::time;

/// The goldens recorded from playwright-core 1.64.1: each expression's selector and `toString()`.
const SELECTOR_GOLDENS: &str =
    include_str!("../../neo-playwright-js/tests/data/selector-goldens.json");

/// One page-opening test at a time where a test waits on a context-wide event: the context lists
/// every tab of the browser, so another test's new page would answer `context.once('page')`.
static ALONE: Semaphore = Semaphore::const_new(1);

/// A host for scripts that must not reach Rust: builders are values and send nothing.
struct NoOps;

/// Component realms share production's globals, so URL revival and predicates cannot silently
/// fall back to a test-only fake while the real server uses a different constructor.
#[tokio::test]
async fn tests_that_component_realms_use_the_production_web_globals() -> TestResult {
    let host: Arc<dyn OpHost> = Arc::new(NoOps);
    let result = js_runner::run_script(
        &host,
        &json!({ "page": null }),
        "const u = new URL('/p?a=1', 'https://example.test/');
         u.searchParams.append('b', 'x y');
         return [u instanceof URL, u.searchParams instanceof URLSearchParams,
                 u.href, atob(btoa('binary'))];",
    )
    .await?;
    assert_eq!(
        result,
        json!([true, true, "https://example.test/p?a=1&b=x+y", "binary"])
    );
    Ok(())
}

impl OpHost for NoOps {
    fn invoke<'a>(
        &'a self,
        name: &'a str,
        _input: Value,
    ) -> BoxFuture<'a, Result<Value, PlaywrightError>> {
        Box::pin(async move {
            Err(PlaywrightError::new(
                ErrorCode::Internal,
                format!("{name}: this script was to send no op"),
            ))
        })
    }

    fn gesture(
        &self,
        _page: TestPage,
        _expression: String,
    ) -> BoxFuture<'_, Result<Value, String>> {
        Box::pin(async { Err("this script was to touch no page".to_owned()) })
    }
}

#[tokio::test]
async fn tests_that_the_prelude_builds_playwrights_locators_in_quickjs() -> TestResult {
    let goldens: Value = serde_json::from_str(SELECTOR_GOLDENS)?;
    let cases = goldens
        .get("cases")
        .and_then(Value::as_array)
        .ok_or("the goldens list their cases")?;
    let codes: Vec<&Value> = cases.iter().filter_map(|case| case.get("code")).collect();
    let host: Arc<dyn OpHost> = Arc::new(NoOps);
    let init = json!({ "page": { "$neo": "page", "tab": 1007 } });
    // A direct eval, so each golden sees the agent's `page` parameter.
    let script = format!(
        "const built = {codes}.map((code) => {{\n\
           const locator = eval(code)\n\
           return {{ code, selector: locator._selector ?? locator._frameSelector, toString: String(locator) }}\n\
         }})\n\
         const go = page.getByRole('button', {{ name: 'Go' }})\n\
         return {{ built, go: {{ selector: go._selector, toString: go.toString() }} }}",
        codes = serde_json::to_string(&codes)?,
    );
    let result = js_runner::run_script(&host, &init, &script).await?;
    let expected: Vec<Value> = cases
        .iter()
        .map(|case| {
            json!({ "code": case["code"], "selector": case["selector"], "toString": case["toString"] })
        })
        .collect();
    assert!(expected.len() >= 60, "the goldens cover the locator API");
    assert_eq!(
        result["built"],
        Value::Array(expected),
        "every golden, in the shipping engine"
    );
    assert_eq!(
        result["go"],
        json!({ "selector": "internal:role=button[name=\"Go\"i]",
                "toString": "getByRole('button', { name: 'Go' })" })
    );
    Ok(())
}

#[tokio::test]
async fn tests_that_an_uncaught_expect_failure_settles_as_its_ops_failure() -> TestResult {
    let host: Arc<dyn OpHost> = Arc::new(NoOps);
    let init = json!({ "page": { "$neo": "page", "tab": 1007 } });
    let script = "await expect(page.locator('#total')).toHaveText('nope', { timeout: 50 })";
    let settled = js_runner::run_script(&host, &init, script).await;
    assert_eq!(
        settled,
        Err(
            "the script threw the error of an op: internal: expect.locator: this script was to \
             send no op"
                .to_owned()
        ),
        "the bootstrap knows the op's own error, so the run reports the failure the door recorded"
    );
    Ok(())
}

/// The run page a seeded run starts on.
fn seeded() -> Value {
    json!({ "page": { "$neo": "page", "tab": 1007 } })
}

/// FR/RC: a refusal the script leaves uncaught settles the run as the refusal's op, whose
/// failure the door recorded with its code, for an async member and a sync one alike.
#[tokio::test]
async fn tests_that_an_uncaught_refusal_settles_as_its_refusal_op() -> TestResult {
    let host: Arc<dyn OpHost> = Arc::new(NoOps);
    for (script, refused) in [
        (
            "await page.route('**', () => {})",
            "page.route: network interception is not available in Neo",
        ),
        (
            "browser.browserType()",
            "browser.browserType: Neo drives its own BrowserOS; there is no browser type to launch",
        ),
    ] {
        assert_eq!(
            js_runner::run_script(&host, &seeded(), script).await,
            Err(format!(
                "the script threw the error of an op: method_unavailable: {refused}"
            )),
            "{script}"
        );
    }
    Ok(())
}

/// A host that records every op it is sent and answers only `page.frames` (with `frames`, or
/// refused as an owner's takeover refuses it) and `page.title`.
struct Recording {
    frames: Option<Value>,
    sent: Mutex<Vec<String>>,
}

impl Recording {
    fn answering_frames(frames: Option<Value>) -> Arc<Self> {
        Arc::new(Self {
            frames,
            sent: Mutex::default(),
        })
    }

    fn sent(&self) -> Vec<String> {
        self.sent
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl OpHost for Recording {
    fn invoke<'a>(
        &'a self,
        name: &'a str,
        _input: Value,
    ) -> BoxFuture<'a, Result<Value, PlaywrightError>> {
        Box::pin(async move {
            self.sent
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(name.to_owned());
            match (name, &self.frames) {
                ("page.frames", Some(frames)) => Ok(frames.clone()),
                ("page.title", _) => Ok(json!("Title")),
                _ => Err(PlaywrightError::new(
                    ErrorCode::Unsupported,
                    format!("{name}: the owner has control of this tab"),
                )),
            }
        })
    }

    // This mock serves page payloads, so it supplies the identity facts that the real
    // door observes with them. Otherwise the prelude must reject their unstamped state.
    fn page_facts(&self, tabs: Vec<TabId>) -> BoxFuture<'_, Vec<Value>> {
        Box::pin(async move {
            tabs.into_iter()
                .map(|tab| json!({ "tab": tab, "closed": false }))
                .collect()
        })
    }

    fn gesture(
        &self,
        _page: TestPage,
        _expression: String,
    ) -> BoxFuture<'_, Result<Value, String>> {
        Box::pin(async { Err("this script was to touch no page".to_owned()) })
    }
}

/// FR-1: a run on a known tab reads the tab's frames once, before the script's first statement,
/// so `page.frames()` lists its iframes with no op of the script's own.
#[tokio::test]
async fn tests_that_a_run_on_a_known_tab_reads_its_frames_before_its_first_statement() -> TestResult
{
    let recording = Recording::answering_frames(Some(json!({ "frames": [
        { "id": "main", "parentId": null, "name": "", "url": "https://a.example/" },
        { "id": "same", "parentId": "main", "name": "same", "url": "https://a.example/child" },
    ] })));
    let host: Arc<dyn OpHost> = Arc::clone(&recording) as Arc<dyn OpHost>;
    let names = js_runner::run_script(
        &host,
        &seeded(),
        "return page.frames().map((frame) => frame.name())",
    )
    .await?;
    assert_eq!(names, json!(["", "same"]));
    assert_eq!(recording.sent(), ["page.frames"], "one read, at the start");
    Ok(())
}

/// A4: a run whose page has no tab yet reads no frames, so a run that never touches `page`
/// opens no tab, and a refusal it meets first is refused by the door alone. The refusal's code
/// is read from the run's settled error, the door's record of it: the error object has the
/// door's fields only after the script next yields to Rust.
#[tokio::test]
async fn tests_that_an_unopened_run_page_opens_no_tab() -> TestResult {
    let recording = Recording::answering_frames(None);
    let host: Arc<dyn OpHost> = Arc::clone(&recording) as Arc<dyn OpHost>;
    let settled = js_runner::run_script(
        &host,
        &json!({ "page": null }),
        "if (page.frames().length !== 1) throw new Error('the unopened page lists one frame')\n\
         await page.route('**', () => {})",
    )
    .await;
    assert_eq!(
        settled,
        Err(
            "the script threw the error of an op: method_unavailable: page.route: network \
             interception is not available in Neo"
                .to_owned()
        ),
        "the door recorded the refusal with its code"
    );
    assert_eq!(
        recording.sent(),
        Vec::<String>::new(),
        "no op reached the host"
    );
    Ok(())
}

/// A4: a frames read the door refuses, as under the owner's takeover, keeps the main frame alone,
/// and the run still starts; its own first op meets the refusal itself.
#[tokio::test]
async fn tests_that_a_refused_frames_read_still_starts_the_run() -> TestResult {
    let recording = Recording::answering_frames(None);
    let host: Arc<dyn OpHost> = Arc::clone(&recording) as Arc<dyn OpHost>;
    let outcome = js_runner::run_script(
        &host,
        &seeded(),
        "return { frames: page.frames().length, title: await page.title() }",
    )
    .await?;
    assert_eq!(outcome, json!({ "frames": 1, "title": "Title" }));
    assert_eq!(recording.sent(), ["page.frames", "page.title"]);
    Ok(())
}

#[tokio::test]
async fn tests_that_context_pages_lists_the_tabs_the_run_init_names() -> TestResult {
    let host: Arc<dyn OpHost> = Arc::new(NoOps);
    let init = json!({
        "page": { "$neo": "page", "tab": 1007 },
        "pages": [
            { "id": 7, "tab": 1007, "url": "https://shop.example/cart", "title": "Cart", "closed": false },
            { "id": 9, "tab": 1009, "url": "https://mail.example/", "title": "Mail", "closed": false },
        ],
    });
    let script =
        "return context.pages().map((p) => ({ url: p.url(), brand: p.toJSON(), run: p === page }))";
    let listed = js_runner::run_script(&host, &init, script).await?;
    assert_eq!(
        listed,
        json!([
            { "url": "https://shop.example/cart", "brand": { "$neo": "page", "tab": 1007 }, "run": true },
            { "url": "https://mail.example/", "brand": { "$neo": "page", "tab": 1009 }, "run": false },
        ]),
        "every tab the init lists, with no op sent"
    );
    Ok(())
}

#[tokio::test]
async fn tests_that_buffer_text_of_megabytes_fits_in_claw_sdks_heap() -> TestResult {
    // Each size is past where the codec ran out of claw-sdk's 64 MiB while it gathered text as
    // an array of numbers: base64, hex and UTF-8 encoding from 2 MiB, UTF-8 decoding from 3 MiB.
    let script = "const bytes = Buffer.alloc(3 * 1024 * 1024)\n\
                  return {\n\
                    base64: bytes.toString('base64').length,\n\
                    hex: bytes.toString('hex').length,\n\
                    utf8: bytes.toString('utf8').length,\n\
                    fromUtf8: Buffer.from('\u{e9}\u{1d11e}x'.repeat(512 * 1024)).length,\n\
                  }";
    let host: Arc<dyn OpHost> = Arc::new(NoOps);
    let init = json!({ "page": { "$neo": "page", "tab": 1007 } });
    let lengths = js_runner::run_script(&host, &init, script).await?;
    assert_eq!(
        lengths,
        json!({ "base64": 4_194_304, "hex": 6_291_456, "utf8": 3_145_728, "fromUtf8": 3_670_016 }),
        "each text of 3 MiB of bytes, under js_runner's 64 MiB heap"
    );
    Ok(())
}

/// A host that takes `locator.setInputFiles` and keeps each payload it was sent.
#[derive(Default)]
struct Uploads(Mutex<Vec<Value>>);

impl OpHost for Uploads {
    fn invoke<'a>(
        &'a self,
        name: &'a str,
        input: Value,
    ) -> BoxFuture<'a, Result<Value, PlaywrightError>> {
        Box::pin(async move {
            if name != "locator.setInputFiles" {
                return NoOps.invoke(name, input).await;
            }
            let payloads = input.pointer("/files/payloads").cloned();
            let mut sent = self.0.lock().unwrap_or_else(PoisonError::into_inner);
            sent.push(payloads.unwrap_or(Value::Null));
            Ok(Value::Null)
        })
    }

    fn gesture(&self, page: TestPage, expression: String) -> BoxFuture<'_, Result<Value, String>> {
        NoOps.gesture(page, expression)
    }
}

#[tokio::test]
async fn tests_that_a_payload_under_neos_limit_reaches_the_op_whole_in_claw_sdks_heap() -> TestResult
{
    // 10 MiB less a byte of 'neo', beside 8 MiB the script holds, under js_runner's 64 MiB heap;
    // then 10 MiB, which Neo refuses before it sends anything.
    let script = "const held = Buffer.alloc(8 * 1024 * 1024, 'x')\n\
                  await page.setInputFiles('#f', { name: 'big.bin', buffer: Buffer.alloc(10 * 1024 * 1024 - 1, 'neo') })\n\
                  try {\n\
                    await page.setInputFiles('#f', { name: 'big.bin', buffer: Buffer.alloc(10 * 1024 * 1024) })\n\
                    return 'sent'\n\
                  } catch (error) {\n\
                    return [error.name, error.message, held.length]\n\
                  }";
    let uploads = Arc::new(Uploads::default());
    let host: Arc<dyn OpHost> = Arc::<Uploads>::clone(&uploads);
    let init = json!({ "page": { "$neo": "page", "tab": 1007 } });
    let refused = js_runner::run_script(&host, &init, script).await?;
    assert_eq!(
        refused,
        json!([
            "Error",
            "Cannot set buffer larger than 10Mb in Neo, please write it to a file and pass its \
             path instead.",
            8_388_608
        ]),
        "10 MiB is refused with upstream's advice"
    );
    let sent = uploads
        .0
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    // 'neo' is 'bmVv' in base64, so the 3 495 253 copies of it are that many of 'bmVv'.
    let whole = json!([[{
        "name": "big.bin",
        "mimeType": "application/octet-stream",
        "buffer": "bmVv".repeat(3_495_253),
    }]]);
    assert!(
        Value::Array(sent) == whole,
        "the payload under the limit reached the op whole, and the one at it never left"
    );
    Ok(())
}

/// The fixtures under `/prelude/`.
fn pages() -> HashMap<String, Page> {
    [
        ("page", include_str!("data/prelude/page.html")),
        ("side", include_str!("data/prelude/side.html")),
        ("popup", include_str!("data/prelude/popup.html")),
    ]
    .into_iter()
    .map(|(name, body)| {
        let page = Page::html(body).with_header("cache-control", "no-store");
        (format!("/prelude/{name}.html"), page)
    })
    .collect()
}

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_playwright_script_runs_through_the_prelude_on_a_live_browser() -> TestResult {
    let _alone = ALONE.acquire().await?;
    let server = FixtureServer::start(pages()).await?;
    let url = |name: &str| server.url("127.0.0.1", &format!("/prelude/{name}.html"));
    let run = Arc::new(LiveRun::connect().await?);
    let host: Arc<dyn OpHost> = run;
    // No tab yet, as for a new task: the door opens the run's tab for its first op.
    let init = json!({ "page": null });
    let script = format!(
        "try {{\n\
           const response = await page.goto({page})\n\
           const go = await page.getByRole('button', {{ name: 'Go' }}).textContent()\n\
           const items = await page.locator('li').count()\n\
           const snapshot = await page.ariaSnapshot({{ mode: 'default' }})\n\
           const aiSnapshot = await page.ariaSnapshot({{ mode: 'ai' }})\n\
           const unnamed = await page.ariaSnapshot()\n\
           const refs = [...aiSnapshot.matchAll(/\\[ref=([^\\]]+)\\]/g)].map((match) => match[1])\n\
           const inFrame = (nodes) => {{\n\
             for (const node of nodes) {{\n\
               if (typeof node === 'string') continue\n\
               if (node.text === 'side') return node.ref\n\
               const found = inFrame(node.children ?? [])\n\
               if (found) return found\n\
             }}\n\
           }}\n\
           const sideRef = inFrame(await page.ariaSnapshotJSON({{ mode: 'ai' }}))\n\
           const byRef = [await page.getByRef(refs[0]).textContent(), await page.getByRef(sideRef).textContent()]\n\
           const frames = page.frames().map((frame) => ({{\n\
             name: frame.name(), url: frame.url(), parent: frame.parentFrame()?.name() ?? null,\n\
           }}))\n\
           const fromPage = new Promise((resolve) => page.once('popup', resolve))\n\
           const fromContext = new Promise((resolve) => context.once('page', resolve))\n\
           const opened = page.waitForEvent('popup')\n\
           const failed = await __neo_test_gesture(page._tab, \"void window.open('/prelude/popup.html')\")\n\
           if (failed) throw new Error(failed)\n\
           const popup = await opened\n\
           const pumped = (await fromPage) === popup && (await fromContext) === popup\n\
           const popupUrl = popup.url()\n\
           const opener = (await popup.opener()) === page\n\
           const listed = context.pages().includes(popup)\n\
           await popup.close()\n\
           await page.goto({side})\n\
           return {{\n\
             status: response.status(), ok: response.ok(), statusText: response.statusText(),\n\
             go, items, snapshot, ai: aiSnapshot.replace(/\\[ref=[^\\]]+\\]/g, '[ref]'), unnamedIsAi: unnamed === aiSnapshot, byRef, frames, popupUrl, pumped, opener, listed, popupClosed: popup.isClosed(),\n\
             after: page.frames().map((frame) => frame.url()), url: page.url(),\n\
           }}\n\
         }} finally {{\n\
           await page.close()\n\
         }}",
        page = json!(url("page")),
        side = json!(url("side")),
    );
    let result = js_runner::run_script(&host, &init, &script).await?;
    assert_eq!(
        result,
        json!({
            "status": 200, "ok": true, "statusText": "OK",
            "go": "Go",
            "items": 3,
            "snapshot": "- button \"Go\"\n- button \"Stop\"\n- list:\n  - listitem: One\n  \
                         - listitem: Two\n  - listitem: Three\n- iframe",
            // Refs differ per process, so they are compared as `[ref]`. The v1 text prints refs
            // only on what can be acted on or scoped to; a button's is read back through
            // getByRef, and so is one inside the iframe, taken from the JSON tree.
            "ai": "- button \"Go\" [ref]\n- button \"Stop\" [ref]\n- text: One Two Three\n\
                   - iframe [ref]:\n  - text: side",
            "unnamedIsAi": true,
            "byRef": ["Go", "side"],
            "frames": [
                { "name": "", "url": url("page"), "parent": null },
                { "name": "side", "url": url("side"), "parent": "" },
            ],
            "popupUrl": url("popup"),
            "pumped": true,
            "opener": true,
            "listed": true,
            "popupClosed": true,
            "after": [url("side")],
            "url": url("side"),
        }),
        "each answer is what Playwright gives"
    );
    Ok(())
}

/// `ariaSnapshot({ delta: true })` on a real engine, across fresh realms on one host as runs
/// are, each ended as the run service ends it (`LiveRun::end_run`): the first snapshot is whole,
/// an unchanged one says so, a collapsed comment lists only its lines, a locator snapshot in
/// between leaves the baseline, another tab has its own, a delta later in the same run still
/// compares against the baseline from the run's start, a new document starts over, and a full
/// snapshot that could have been a delta is counted for the server's tip.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_delta_snapshot_lists_what_changed_on_a_live_browser() -> TestResult {
    let page = Page::html(include_str!("data/prelude/delta.html"))
        .with_header("cache-control", "no-store");
    let server =
        FixtureServer::start(HashMap::from([("/prelude/delta.html".to_owned(), page)])).await?;
    let url = json!(server.url("127.0.0.1", "/prelude/delta.html"));
    let run = Arc::new(LiveRun::connect().await?);
    let host: Arc<dyn OpHost> = Arc::clone(&run) as Arc<dyn OpHost>;
    let opened = js_runner::run_script(
        &host,
        &json!({ "page": null }),
        &format!(
            "await page.goto({url})\n\
             return {{ first: await page.ariaSnapshot({{ delta: true }}), tab: page.toJSON().tab }}"
        ),
    )
    .await?;
    // The run returned its snapshot, so its text is the tab's baseline from here on.
    run.end_run(Some(&opened));
    let tab = opened["tab"].as_i64().ok_or("the page's tab")?;
    let first = opened["first"].as_str().ok_or("the first snapshot")?;
    assert!(
        first.starts_with("# whole page: no earlier snapshot of this page\n- main [ref="),
        "{first}"
    );
    // A second run is a fresh realm on the same tab: the baseline is the host's.
    let script = format!(
        "try {{\n\
           const unchanged = await page.ariaSnapshot({{ delta: true }})\n\
           await page.getByRole('article').ariaSnapshot()\n\
           await page.getByRole('button', {{ name: 'Collapse' }}).click()\n\
           const collapsed = await page.ariaSnapshot({{ delta: true }})\n\
           const full = await page.ariaSnapshot()\n\
           const other = await context.newPage()\n\
           await other.goto({url})\n\
           const otherFirst = await other.ariaSnapshot({{ delta: true }})\n\
           await other.close()\n\
           const again = await page.ariaSnapshot({{ delta: true }})\n\
           await page.goto({url})\n\
           const navigated = await page.ariaSnapshot({{ delta: true }})\n\
           return {{ unchanged, collapsed, full, otherFirst, again, navigated }}\n\
         }} finally {{\n\
           await page.close()\n\
         }}"
    );
    let result = js_runner::run_script(
        &host,
        &json!({ "page": { "$neo": "page", "tab": tab } }),
        &script,
    )
    .await?;
    let text = |key: &str| result[key].as_str().unwrap_or_default().to_owned();
    assert_eq!(
        text("unchanged"),
        "# no change since your last snapshot of this tab"
    );
    assert_eq!(
        without_refs(&text("collapsed")),
        "# changes since your last snapshot of this tab: 1 added, 3 removed\n\
         @@ article \"First comment\" [ref]\n \
         - main [ref]:\n   \
         - heading \"Thread\" [level=1]\n   \
         - article \"First comment\" [ref]:\n\
         -    - button \"Collapse\" [expanded] [ref]\n\
         -    - link \"Reply one\" [ref]\n\
         -    - link \"Reply two\" [ref]\n\
         +    - button \"Expand\" [active] [ref]\n   \
         - link \"Story 0\" [ref]\n   \
         - link \"Story 1\" [ref]\n   \
         - link \"Story 2\" [ref]",
        "the collapse, and nothing else: the clicked button also has focus now"
    );
    assert!(
        text("full").starts_with("- main [ref="),
        "without delta the text is whole"
    );
    assert!(
        text("otherFirst").starts_with("# whole page: no earlier snapshot of this page\n"),
        "another tab has no baseline of its own yet"
    );
    assert_eq!(
        text("again"),
        text("collapsed"),
        "the same run's snapshots stay pending: this one still compares against the first \
         run's text, and the other tab's snapshot left it alone"
    );
    assert!(
        text("navigated").starts_with("# whole page: no earlier snapshot of this page\n"),
        "a new document starts over: {}",
        text("navigated")
    );
    assert_eq!(
        run.full_snapshot_repeats(),
        [TabId(tab)],
        "the full snapshot while a baseline was stored is the one a tip is for"
    );
    Ok(())
}

/// The live browser, with each whole-page snapshot's reply held for `hold` after the browser
/// answered, as Rust's reply path held run 2's in the owner's export of 12 November (the
/// event-ordering wait after the handler, up to two seconds, and the reply's observation). Every
/// op name is recorded.
struct LateTrees {
    run: Arc<LiveRun>,
    hold: Mutex<Duration>,
    sent: Mutex<Vec<String>>,
}

impl LateTrees {
    fn new(run: Arc<LiveRun>) -> Arc<Self> {
        Arc::new(Self {
            run,
            hold: Mutex::new(Duration::ZERO),
            sent: Mutex::default(),
        })
    }

    fn hold(&self, hold: Duration) {
        *self.hold.lock().unwrap_or_else(PoisonError::into_inner) = hold;
    }

    /// The op names sent since the last call.
    fn sent(&self) -> Vec<String> {
        mem::take(&mut *self.sent.lock().unwrap_or_else(PoisonError::into_inner))
    }
}

impl OpHost for LateTrees {
    fn invoke<'a>(
        &'a self,
        name: &'a str,
        input: Value,
    ) -> BoxFuture<'a, Result<Value, PlaywrightError>> {
        Box::pin(async move {
            self.sent
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(name.to_owned());
            let answer = OpHost::invoke(&*self.run, name, input).await;
            if name == "page.ariaSnapshot" && answer.is_ok() {
                let hold = *self.hold.lock().unwrap_or_else(PoisonError::into_inner);
                time::sleep(hold).await;
            }
            answer
        })
    }

    fn event_head(&self) -> u64 {
        OpHost::event_head(&*self.run)
    }

    fn page_facts(&self, tabs: Vec<TabId>) -> BoxFuture<'_, Vec<Value>> {
        OpHost::page_facts(&*self.run, tabs)
    }

    fn gesture(&self, page: TestPage, expression: String) -> BoxFuture<'_, Result<Value, String>> {
        OpHost::gesture(&*self.run, page, expression)
    }
}

/// Run 2 of the owner's export of 12 November: the browser took a whole-page snapshot in time,
/// its reply reached the realm after the call's deadline, and the call threw `TimeoutError` with
/// no op behind it, which the run reported as the script's own error. On a real engine, with the
/// reply held 1.3 s past the browser's answer and a 1 s timeout, under the committed choice (A,
/// `snapshot-late.ts`): the call returns the snapshot, records no baseline (no
/// `page.snapshotPending`), and the next run's delta is the whole page. Through the raw ops, a
/// record naming a held call whose 1 s passed is refused by Rust's own clock as that call's
/// timeout, naming the phase (the decision B would surface), and the same record naming a call
/// with 10 s is taken: no script can make Rust give that timeout for a call whose time had not
/// passed. A slow page alone does not cross: Rust's own op shares the call's timeout and times
/// out first, which is the verdict that governs a tree that never came.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_snapshot_whose_reply_lands_after_its_deadline_is_returned_on_a_live_browser()
-> TestResult {
    let page = Page::html(include_str!("data/prelude/delta.html"))
        .with_header("cache-control", "no-store");
    let server =
        FixtureServer::start(HashMap::from([("/prelude/delta.html".to_owned(), page)])).await?;
    let url = json!(server.url("127.0.0.1", "/prelude/delta.html"));
    let run = Arc::new(LiveRun::connect().await?);
    let late = LateTrees::new(Arc::clone(&run));
    let host: Arc<dyn OpHost> = Arc::clone(&late) as Arc<dyn OpHost>;

    late.hold(Duration::from_millis(1_300));
    let opened = js_runner::run_script(
        &host,
        &json!({ "page": null }),
        &format!(
            "await page.goto({url})\n\
             const started = Date.now()\n\
             const text = await page.ariaSnapshot({{ timeout: 1000 }})\n\
             const took = Date.now() - started\n\
             // A text the run never returns, so the host never makes it the tab's baseline.\n\
             const PROBE = 'a probe text this run never returns'\n\
             const op = async (name, input) => JSON.parse(await __neo_invoke(name, JSON.stringify(input), ''))\n\
             const record = async (timeoutMs) => {{\n\
               const tree = await op('page.ariaSnapshot', {{ call: {{ api: 'page.ariaSnapshot', timeoutMs }}, page: page.toJSON(), mode: 'ai' }})\n\
               const reply = await op('page.snapshotPending', {{ call: {{ api: 'page.ariaSnapshot', timeoutMs: 0 }}, page: page.toJSON(),\n\
                 document: tree.ok.document, snapshotCall: tree.ok.snapshotCall, text: PROBE,\n\
                 form: {{ text: 'kept', urls: false, maxTokens: 8000 }}, delta: false }})\n\
               return reply.err ? [reply.err.code, reply.err.message] : 'recorded'\n\
             }}\n\
             return {{ text, took, tab: page.toJSON().tab, late: await record(1000), inTime: await record(10000) }}"
        ),
    )
    .await?;
    run.end_run(Some(&opened));
    let tab = opened["tab"].as_i64().ok_or("the page's tab")?;
    let text = opened["text"].as_str().ok_or("the late snapshot")?;
    assert!(text.starts_with("- main [ref="), "the snapshot: {text}");
    assert!(
        opened["took"].as_i64().is_some_and(|took| took >= 1_000),
        "the reply crossed the deadline: {opened}"
    );
    let sent = late.sent();
    assert_eq!(
        sent.iter()
            .filter(|name| name.as_str() == "page.snapshotPending")
            .count(),
        2,
        "the prelude recorded no baseline for the late text; both pending ops are the script's \
         own: {sent:?}"
    );
    assert_eq!(
        opened["late"][0],
        json!("wait_timeout"),
        "Rust refused a record for a call whose 1 s had passed: {opened}"
    );
    assert!(
        opened["late"][1]
            .as_str()
            .is_some_and(|message| message.starts_with(
                "page.ariaSnapshot: Timeout 1000ms exceeded after the snapshot was taken, while \
             recording its delta baseline."
            )),
        "naming the phase: {opened}"
    );
    assert_eq!(
        opened["inTime"],
        json!("recorded"),
        "the same record for a call with 10 s is taken: Rust holds each call to its own deadline"
    );

    // The next runs, on time: the late text was never a baseline, so the first delta is whole;
    // that one was given in time and is the baseline for the next.
    late.hold(Duration::ZERO);
    let at_tab = json!({ "page": { "$neo": "page", "tab": tab } });
    let first = js_runner::run_script(
        &host,
        &at_tab,
        "return await page.ariaSnapshot({ delta: true })",
    )
    .await?;
    run.end_run(Some(&first));
    let first = first.as_str().unwrap_or_default();
    assert!(
        first.starts_with("# whole page: no earlier snapshot of this page\n- main [ref="),
        "{first}"
    );
    assert!(
        late.sent()
            .iter()
            .any(|name| name == "page.snapshotPending"),
        "an in-time snapshot records its baseline"
    );
    let again = js_runner::run_script(
        &host,
        &at_tab,
        "try { return await page.ariaSnapshot({ delta: true }) } finally { await page.close() }",
    )
    .await?;
    assert_eq!(
        again,
        json!("# no change since your last snapshot of this tab")
    );
    Ok(())
}

/// `text` with every `[ref=…]` printed as `[ref]`: refs differ per process.
fn without_refs(text: &str) -> String {
    text.split("[ref=")
        .enumerate()
        .map(|(index, part)| match (index, part.split_once(']')) {
            (0, _) | (_, None) => part.to_owned(),
            (_, Some((_, rest))) => format!("[ref]{rest}"),
        })
        .collect()
}

/// The v1 format's test page: a block notice beside a button, a form error, prices beside
/// buttons and a dialog, the cases a list of controls alone would leave unexplained.
const BLIND_PAGE: &str = include_str!("data/snapshot-v1/blind.html");
/// What `ariaSnapshot()` prints for it: the text form, since the page is small.
const BLIND_DEFAULT: &str = include_str!("data/snapshot-v1/blind.v1-default.txt");
/// What `ariaSnapshot({ text: false })` prints for it.
const BLIND_INTERACTIVE: &str = include_str!("data/snapshot-v1/blind.v1-interactive.txt");
/// What Playwright 1.64.1's own `ariaSnapshot({ mode: 'ai' })` printed for it, and so what
/// Neo's `mode: 'full'` prints: the page has no element that only clickable marking finds.
const BLIND_PLAYWRIGHT_AI: &str = include_str!("data/snapshot-v1/blind.playwright-ai.yml");
/// The snapshot op's `{ json, partial }` for the page, which the prelude's renderer tests read
/// (neo-playwright-js/tests/snapshot-v1.test.ts). From this crate's manifest directory.
const BLIND_RESULT: &str = "tests/data/snapshot-v1/blind.json";

/// The v1 text on a real engine, page and `run`: the renderer's unit tests read the JSON this
/// test pins, so together they show the fixture's expected text is what a live page prints.
/// `NEO_RECORD_SNAPSHOT_V1=1` rewrites the JSON instead of comparing it. The ref numbers are
/// those of the first snapshot of a freshly loaded page.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_the_blind_page_prints_the_v1_text_on_a_live_browser() -> TestResult {
    let page = Page::html(BLIND_PAGE).with_header("cache-control", "no-store");
    let server = FixtureServer::start(HashMap::from([(
        "/snapshot-v1/blind.html".to_owned(),
        page,
    )]))
    .await?;
    let run = Arc::new(LiveRun::connect().await?);
    let host: Arc<dyn OpHost> = run;
    let script = format!(
        "try {{\n\
           await page.goto({url})\n\
           const plain = await page.ariaSnapshot()\n\
           const interactive = await page.ariaSnapshot({{ text: false }})\n\
           const json = await page.ariaSnapshotJSON({{ mode: 'ai' }})\n\
           const full = await page.ariaSnapshot({{ mode: 'full' }})\n\
           return {{ plain, interactive, json, full }}\n\
         }} finally {{\n\
           await page.close()\n\
         }}",
        url = json!(server.url("127.0.0.1", "/snapshot-v1/blind.html")),
    );
    let result = js_runner::run_script(&host, &json!({ "page": null }), &script).await?;
    let recorded = json!({ "json": result["json"], "partial": [] });
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(BLIND_RESULT);
    if env::var("NEO_RECORD_SNAPSHOT_V1").is_ok_and(|record| record == "1") {
        fs::write(
            &path,
            format!("{}\n", serde_json::to_string_pretty(&recorded)?),
        )?;
    } else {
        let committed: Value = serde_json::from_str(&fs::read_to_string(&path)?)?;
        assert_eq!(
            recorded, committed,
            "the engine still gives the committed tree"
        );
    }
    assert_eq!(
        result["plain"], BLIND_DEFAULT,
        "the default is the text form"
    );
    assert_eq!(result["interactive"], BLIND_INTERACTIVE, "text: false");
    assert_eq!(
        result["full"], BLIND_PLAYWRIGHT_AI,
        "mode 'full' is Playwright's own ai text"
    );
    Ok(())
}

/// `mode: 'full'`'s test page (design section 13): a region holding a plain `div` (no listener,
/// no pointer cursor, so the v1 text prints it with no ref) and a `div` with only a click
/// listener (a ref only through clickable marking); 600 orders in `main`, so the whole page is
/// over the default cap; a footer after it, which the cut line offers; and a dialog at the end of
/// the document, which a cut must not hide.
fn full_mode_page() -> String {
    let orders = (0..600)
        .map(|n| format!("<li>Order {n} <button>View {n}</button></li>"))
        .collect::<Vec<_>>()
        .concat();
    format!(
        r##"<!doctype html><html><head><title>Full</title>
<style>#hover:hover {{ outline: 2px solid red }}</style></head><body>
<nav aria-label="Top"><a href="#home">Home</a> <a href="#about">About</a></nav>
<section aria-label="Tools"><div id="hover">Hover me</div><div class="menu">Open menu</div></section>
<main><h1>Orders</h1><ul>{orders}</ul></main>
<footer><a href="#contact">Contact</a></footer>
<div role="dialog" aria-label="Cookies"><p>We use cookies.</p><button>Accept</button></div>
<script>document.querySelector('.menu').addEventListener('click', () => {{}})</script>
</body></html>"##
    )
}

/// `ariaSnapshot({ mode: 'full' })` on a real engine, page and locator (design section 13): it
/// prints every ref the v1 text prints in either form (the marked `div` included), the whole page
/// is cut near 8,000 tokens with the dialog kept, a section named by the v1 text comes back whole,
/// a ref only the full text prints can be hovered, and the snapshot counts as the run's snapshot
/// of a tab it opened, so a stale ref after it gets the ordinary message. A second run shows it
/// is never a delta baseline and never counted for the server's tip.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_mode_full_prints_every_ref_capped_on_a_live_browser() -> TestResult {
    let page = Page::html(&full_mode_page()).with_header("cache-control", "no-store");
    let server =
        FixtureServer::start(HashMap::from([("/prelude/full.html".to_owned(), page)])).await?;
    let url = json!(server.url("127.0.0.1", "/prelude/full.html"));
    let run = Arc::new(LiveRun::connect().await?);
    let host: Arc<dyn OpHost> = Arc::clone(&run) as Arc<dyn OpHost>;
    let script = format!(
        "const stale = () => page.getByRef('e9999').textContent({{ timeout: 2000 }}).then(() => null, (error) => error.message)\n\
         await page.goto({url})\n\
         const before = await stale()\n\
         const full = await page.ariaSnapshot({{ mode: 'full' }})\n\
         const after = await stale()\n\
         const text = await page.ariaSnapshot({{ text: true, maxTokens: 100000 }})\n\
         const interactive = await page.ariaSnapshot({{ text: false, maxTokens: 100000 }})\n\
         const uncut = await page.ariaSnapshot({{ mode: 'full', maxTokens: 100000 }})\n\
         const plain = await page.ariaSnapshot()\n\
         const tools = plain.match(/- region \"Tools\" \\[ref=([^\\]]+)\\]/)?.[1]\n\
         const section = await page.getByRef(tools).ariaSnapshot({{ mode: 'full' }})\n\
         const hoverRef = section.match(/- generic \\[ref=([^\\]]+)\\]: Hover me/)?.[1] ?? null\n\
         if (hoverRef) await page.getByRef(hoverRef).hover()\n\
         const hovered = await page.evaluate(() => document.querySelector('#hover').matches(':hover'))\n\
         return {{ before, after, full, text, interactive, uncut, plain, section, hoverRef, hovered, tab: page.toJSON().tab }}"
    );
    let result = js_runner::run_script(&host, &json!({ "page": null }), &script).await?;
    run.end_run(Some(&result));
    let text = |key: &str| result[key].as_str().unwrap_or_default().to_owned();

    assert!(
        text("before").contains("e9999 is not from this tab, which this run opened"),
        "no snapshot yet on the tab this run opened: {}",
        text("before")
    );
    assert!(
        text("after").contains("e9999 is not on the page any more; take a new snapshot"),
        "a full-mode snapshot is the run's snapshot of the tab: {}",
        text("after")
    );

    assert_full_text(&result)?;

    let section = text("section");
    assert!(
        section.starts_with("- region \"Tools\" [ref=") && !section.contains("# cut: "),
        "the section named by the v1 text comes back whole: {section}"
    );
    let hover_ref = result["hoverRef"].as_str().ok_or("the plain div's ref")?;
    assert!(
        !printed_refs(&text("text")).contains(hover_ref),
        "the plain div has no ref in the v1 text"
    );
    assert_eq!(
        result["hovered"], true,
        "the full text's ref hovers the div"
    );

    let tab = result["tab"].as_i64().ok_or("the page's tab")?;
    assert_full_mode_leaves_the_baseline(&run, &host, tab).await
}

/// The first run's full texts: the uncut one prints every ref the v1 forms print (the marked
/// `div`'s too); the one cut at the default cap keeps the uncut text's top lines, offers the
/// next section in mode full, and still prints the dialog after the cut line.
fn assert_full_text(result: &Value) -> TestResult {
    let text = |key: &str| result[key].as_str().unwrap_or_default().to_owned();
    let uncut = text("uncut");
    assert!(!uncut.contains("# cut: "), "{uncut}");
    assert!(
        uncut
            .lines()
            .any(|line| line.contains("[cursor=pointer]: Open menu") && line.contains("[ref=")),
        "the listener-only div keeps its ref through clickable marking: {uncut}"
    );
    let full_refs = printed_refs(&uncut);
    for form in ["text", "interactive", "plain"] {
        let missing: Vec<String> = printed_refs(&text(form))
            .difference(&full_refs)
            .cloned()
            .collect();
        assert!(
            missing.is_empty(),
            "{form} prints refs the full text lacks: {missing:?}"
        );
    }

    let full = text("full");
    let (kept, rest) = full
        .split_once("\n# cut: ")
        .ok_or("the whole page is cut")?;
    assert!(
        kept.len() <= 32_000 && kept.len() > 31_000,
        "cut near 8,000 tokens: {} characters",
        kept.len()
    );
    assert!(
        uncut.starts_with(kept),
        "the lines above the cut are the uncut text's"
    );
    let mut after_cut = rest.lines();
    let cut_line = after_cut.next().unwrap_or_default();
    assert!(
        cut_line.contains(".ariaSnapshot({ mode: 'full' }) shows the contentinfo section"),
        "the offer keeps the mode: {cut_line}"
    );
    assert!(
        after_cut
            .next()
            .is_some_and(|line| line.starts_with("- dialog \"Cookies\" [ref=")),
        "the dialog past the cut still prints, at the left edge: {rest}"
    );
    Ok(())
}

/// A second run on the tab: a full-mode snapshot between the stored v1 text and a delta changes
/// nothing, so the delta finds no change, and it is never counted for the server's delta tip.
async fn assert_full_mode_leaves_the_baseline(
    run: &LiveRun,
    host: &Arc<dyn OpHost>,
    tab: i64,
) -> TestResult {
    let second = js_runner::run_script(
        host,
        &json!({ "page": { "$neo": "page", "tab": tab } }),
        "try {\n\
           const full = await page.ariaSnapshot({ mode: 'full' })\n\
           return { full, delta: await page.ariaSnapshot({ delta: true }) }\n\
         } finally {\n\
           await page.close()\n\
         }",
    )
    .await?;
    assert_eq!(
        second.get("delta").and_then(Value::as_str),
        Some("# no change since your last snapshot of this tab"),
        "the full text in between changed nothing"
    );
    assert_eq!(
        run.full_snapshot_repeats(),
        Vec::<TabId>::new(),
        "a full-mode snapshot is never counted for the delta tip"
    );
    Ok(())
}

/// Snapshot v1's clickable marking (design section 4), as twins: each element under `listeners`
/// has only a click listener and the default cursor; its twin under `pointers` has a real
/// `cursor: pointer` and no listener. The cases: a `div` with text, an empty tile, a `div`
/// around a named icon, a shadow `div` whose named icon is slotted into it (CSS inheritance
/// follows the slot), a `<label>` around a named icon (its real cursor is the browser
/// stylesheet's `default`, which the icon inherits), a `div` around a paragraph, a `div` wrapped
/// in another together with a note, and a `div` that is its wrapper's only child (which
/// upstream's distiller inlines into the wrapper, real cursor or not; design.md section 4, what
/// the engine still loses). A container whose listener only relays its buttons' clicks follows.
/// The listeners are the page's own script.
const CLICKABLE_PAGE: &str = r#"<!doctype html><html><head><title>Clickable</title></head><body>
<main>
<nav aria-label="listeners">
<div class="menu">Open menu</div>
<div class="tile" style="width: 40px; height: 40px"></div>
<div class="icon"><img alt="Settings" width="16" height="16" src="data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='16' height='16'/%3E"></div>
<icon-box data-hit><img alt="Slotted" width="16" height="16" src="data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='16' height='16'/%3E"></icon-box>
<label class="label"><img alt="Labelled" width="16" height="16" src="data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='16' height='16'/%3E"></label>
<div class="para"><p>Para text</p></div>
<div><div class="inner">Wrapped with</div><span>a note</span></div>
<div><div class="inner">Wrapped</div></div>
</nav>
<nav aria-label="pointers">
<div style="cursor: pointer">Open menu</div>
<div style="width: 40px; height: 40px; cursor: pointer"></div>
<div style="cursor: pointer"><img alt="Settings" width="16" height="16" src="data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='16' height='16'/%3E"></div>
<icon-box data-pointer><img alt="Slotted" width="16" height="16" src="data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='16' height='16'/%3E"></icon-box>
<label style="cursor: pointer"><img alt="Labelled" width="16" height="16" src="data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='16' height='16'/%3E"></label>
<div style="cursor: pointer"><p>Para text</p></div>
<div><div style="cursor: pointer">Wrapped with</div><span>a note</span></div>
<div><div style="cursor: pointer">Wrapped</div></div>
</nav>
<div id="relay"><button>Archive</button><button>Delete</button></div>
<p>Plain text</p>
</main>
<script>
customElements.define('icon-box', class extends HTMLElement {
  constructor() {
    super()
    const style = this.hasAttribute('data-pointer') ? ' style="cursor: pointer"' : ''
    this.attachShadow({ mode: 'open' }).innerHTML = `<div${style}><slot></slot></div>`
    if (this.hasAttribute('data-hit'))
      this.shadowRoot.firstElementChild.addEventListener('click', () => {})
  }
})
const listeners = document.querySelector('nav[aria-label="listeners"]')
for (const element of listeners.querySelectorAll('.menu, .tile, .icon, .label, .para, .inner'))
  element.addEventListener('click', () => {})
listeners.querySelector('.menu').addEventListener('click', (event) => { event.currentTarget.textContent = 'Menu open' })
document.getElementById('relay').addEventListener('click', () => {})
</script>
</body></html>"#;

/// The script both runs share: the v1 text (or, under the switch, Playwright's), the ai text, and
/// a click through the listener menu's printed ref.
fn clickable_script(url: &str) -> String {
    format!(
        "try {{\n\
           await page.goto({url})\n\
           const plain = await page.ariaSnapshot()\n\
           const ai = await page.ariaSnapshot({{ mode: 'ai' }})\n\
           const menu = ai.match(/- generic \\[ref=([^\\]]+)\\][^\\n]*: Open menu/)?.[1] ?? null\n\
           if (menu) await page.getByRef(menu).click()\n\
           const after = menu ? await page.getByRef(menu).textContent() : null\n\
           const refless = (text) => text.replace(/\\[ref=[^\\]]+\\]/g, '[ref]')\n\
           return {{ plain: refless(plain), ai: refless(ai), after }}\n\
         }} finally {{\n\
           await page.close()\n\
         }}",
        url = json!(url),
    )
}

/// The lines under the navigation named `name`, without their indent.
fn block(text: &str, name: &str) -> Vec<String> {
    let header = format!("- navigation \"{name}\" [ref]:");
    let mut lines = text.lines().skip_while(|line| line.trim() != header);
    let Some(header) = lines.next() else {
        return Vec::new();
    };
    let indent = header.len() - header.trim_start().len() + 2;
    lines
        .take_while(|line| line.len() - line.trim_start().len() >= indent)
        .map(|line| line.trim().to_owned())
        .collect()
}

/// A `div` with only a click listener and the default cursor keeps its ref, printed with
/// `[cursor=pointer]`, and a click through that ref reaches its listener; every listener-only
/// element prints exactly as its twin with a real `cursor: pointer` does, also on the same page
/// under `body { cursor: default }`, and a container that relays its buttons' clicks prints no
/// line of its own. Under the spec harness's conformance switch the same page prints
/// Playwright's own text: the listener-only elements have no mark.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_listener_only_elements_print_as_their_pointer_twins_and_the_switch_leaves_them()
-> TestResult {
    let _alone = ALONE.acquire().await?;
    let page = |html: &str| Page::html(html).with_header("cache-control", "no-store");
    // Every element of this copy reads `default` as its real cursor, a target included, so what
    // a target holds inherits `default`, not `auto`.
    let default_cursor =
        CLICKABLE_PAGE.replace("</head>", "<style>body { cursor: default }</style></head>");
    let server = FixtureServer::start(HashMap::from([
        ("/clickable.html".to_owned(), page(CLICKABLE_PAGE)),
        ("/clickable-default.html".to_owned(), page(&default_cursor)),
    ]))
    .await?;
    let url = server.url("127.0.0.1", "/clickable.html");
    let run = Arc::new(LiveRun::connect().await?);
    let host: Arc<dyn OpHost> = Arc::clone(&run) as Arc<dyn OpHost>;
    let init = json!({ "page": null });
    let twins = [
        "- generic [ref] [cursor=pointer]: Open menu",
        "- generic [ref] [cursor=pointer]",
        "- img \"Settings\" [ref] [cursor=pointer]",
        "- img \"Slotted\" [ref] [cursor=pointer]",
        "- img \"Labelled\" [ref] [cursor=pointer]",
        "- paragraph [ref] [cursor=pointer]: Para text",
        "- generic [ref] [cursor=pointer]: Wrapped with",
        "- text: a note Wrapped",
    ];
    for path in ["/clickable.html", "/clickable-default.html"] {
        let script = clickable_script(&server.url("127.0.0.1", path));
        let v1 = js_runner::run_script(&host, &init, &script).await?;
        let plain = v1["plain"].as_str().ok_or("the v1 text")?;
        assert_eq!(block(plain, "listeners"), twins, "{path}: {plain}");
        assert_eq!(block(plain, "pointers"), twins, "{path}: {plain}");
        assert!(
            plain.ends_with(
                "  - button \"Archive\" [ref]\n  - button \"Delete\" [ref]\n  - text: Plain text"
            ),
            "{path}: the relaying container prints no line of its own: {plain}"
        );
        assert_eq!(v1["plain"], v1["ai"], "{path}: no mode is mode 'ai'");
        assert_eq!(
            v1["after"], "Menu open",
            "{path}: a click through the printed ref"
        );
    }

    let upstream: Arc<dyn OpHost> = Arc::new(Upstream(Arc::clone(&run)));
    let upstream = js_runner::run_script(&upstream, &init, &clickable_script(&url)).await?;
    let ai = upstream["ai"].as_str().ok_or("Playwright's ai text")?;
    let listeners = ai
        .split("- navigation \"pointers\"")
        .next()
        .ok_or("the listeners' half")?;
    assert!(
        listeners.contains("- generic [ref]: Open menu") && !listeners.contains("[cursor=pointer]"),
        "under the switch no listener-only element is marked: {ai}"
    );
    assert!(
        ai.contains("- generic [ref] [cursor=pointer]: Open menu"),
        "the real pointer twin keeps upstream's own mark: {ai}"
    );
    Ok(())
}

/// The live browser as the spec harness hosts it: the same ops, with the conformance switch set
/// in the run's realm.
struct Upstream(Arc<LiveRun>);

impl OpHost for Upstream {
    fn invoke<'a>(
        &'a self,
        name: &'a str,
        input: Value,
    ) -> BoxFuture<'a, Result<Value, PlaywrightError>> {
        OpHost::invoke(&*self.0, name, input)
    }

    fn event_head(&self) -> u64 {
        OpHost::event_head(&*self.0)
    }

    fn page_facts(&self, tabs: Vec<TabId>) -> BoxFuture<'_, Vec<Value>> {
        OpHost::page_facts(&*self.0, tabs)
    }

    fn gesture(&self, page: TestPage, expression: String) -> BoxFuture<'_, Result<Value, String>> {
        OpHost::gesture(&*self.0, page, expression)
    }

    fn upstream_snapshots(&self) -> bool {
        true
    }
}

/// A page with two iframes, each holding a listener-only `div`: one on the page's own site,
/// which shares the page's renderer and CDP session (its document is found inside the session's
/// capture through its `<iframe>`), and one on `localhost`, another site, which runs out of
/// process with a session of its own. The cross-site frame's address names the page's own port.
const FRAMED_PAGE: &str = r#"<!doctype html><html><head><title>Framed</title></head><body>
<main>
<iframe title="same" src="/frame-same.html"></iframe>
<iframe title="cross" id="cross"></iframe>
</main>
<script>
document.getElementById('cross').src = `http://localhost:${location.port}/frame-cross.html`
</script>
</body></html>"#;

/// One frame's page: a listener-only `div` named `menu` beside its twin with a real
/// `cursor: pointer`. Not alone: the engine inlines a frame body's only child into the frame's
/// line whatever its cursor (design.md section 11, what the engine still loses).
fn frame_page(menu: &str) -> String {
    format!(
        "<!doctype html><html><body><div class=\"menu\">{menu}</div>\n\
         <div style=\"cursor: pointer\">{menu} twin</div>\n\
         <script>document.querySelector('.menu').addEventListener('click', () => {{}})</script>\n\
         </body></html>"
    )
}

/// Marking reads every frame the snapshot does: a listener-only element in a same-site iframe,
/// and one in a cross-site iframe, both print with a ref and `[cursor=pointer]`, as their
/// real-pointer twins beside them do. A frame whose
/// read fails is snapshotted without its marks and the snapshot still succeeds, so only the
/// marks show that each frame's document was found.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_listener_only_elements_in_same_and_cross_site_frames_are_marked() -> TestResult
{
    let _alone = ALONE.acquire().await?;
    let page = |html: &str| Page::html(html).with_header("cache-control", "no-store");
    let server = FixtureServer::start(HashMap::from([
        ("/framed.html".to_owned(), page(FRAMED_PAGE)),
        (
            "/frame-same.html".to_owned(),
            page(&frame_page("Same-site menu")),
        ),
        (
            "/frame-cross.html".to_owned(),
            page(&frame_page("Cross-site menu")),
        ),
    ]))
    .await?;
    let script = format!(
        "try {{\n\
           await page.goto({url})\n\
           const frames = page.frames().map((frame) => frame.url())\n\
           const text = (await page.ariaSnapshot()).replace(/\\[ref=[^\\]]+\\]/g, '[ref]')\n\
           return {{ frames, text }}\n\
         }} finally {{\n\
           await page.close()\n\
         }}",
        url = json!(server.url("127.0.0.1", "/framed.html")),
    );
    let run: Arc<dyn OpHost> = Arc::new(LiveRun::connect().await?);
    let framed = js_runner::run_script(&run, &json!({ "page": null }), &script).await?;
    assert_eq!(
        framed["frames"],
        json!([
            server.url("127.0.0.1", "/framed.html"),
            server.url("127.0.0.1", "/frame-same.html"),
            server.url("localhost", "/frame-cross.html"),
        ]),
        "both frames loaded"
    );
    let text = framed["text"].as_str().ok_or("the v1 text")?;
    for menu in ["Same-site menu", "Cross-site menu"] {
        for name in [menu.to_owned(), format!("{menu} twin")] {
            let line = format!("- generic [ref] [cursor=pointer]: {name}");
            assert!(
                text.lines().any(|printed| printed.trim() == line),
                "{name} is marked: {text}"
            );
        }
    }
    Ok(())
}

/// The fallback markup of `FALLBACK_PAGE`'s second iframe, as the HTML parser keeps it: raw text.
const FALLBACK_MARKUP: &str = "<p>Your browser does not support iframes.</p>";

/// The page of the ruling on an iframe's fallback markup (design section 13, found after
/// #3521): the same cross-site card frame twice, the first iframe written empty and the second
/// with fallback markup between its tags, as the card frames of a real donation page carry it.
/// The frames are on `localhost`, another site, so each runs out of process with a session of
/// its own; their address names the page's own port.
const FALLBACK_PAGE: &str = r#"<!doctype html><html><head><title>Fallback</title></head><body>
<main>
<iframe id="plain"></iframe>
<iframe id="fallback"><p>Your browser does not support iframes.</p></iframe>
</main>
<script>
for (const id of ['plain', 'fallback'])
  document.getElementById(id).src = `http://localhost:${location.port}/card.html`
</script>
</body></html>"#;

/// The card frame: the field and the button the default text prints with refs.
const CARD_FRAME: &str = r#"<!doctype html><html><body>
<input aria-label="Card number"> <button>Pay</button>
</body></html>"#;

/// An iframe whose tag holds fallback markup, on a real engine: the engine reads the markup as
/// the iframe node's `text` and Rust stitches the frame's content under the same node. Neo's
/// formats print that frame with its refs and never the markup: `mode: 'full'` prints every ref
/// the default text prints, in both forms, the second card field's included. Under the spec
/// harness's conformance switch the page prints Playwright's own text: the fallback line, and
/// no second frame.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_an_iframe_with_fallback_markup_prints_its_frame_and_the_switch_keeps_the_markup()
-> TestResult {
    let page = |html: &str| Page::html(html).with_header("cache-control", "no-store");
    let server = FixtureServer::start(HashMap::from([
        ("/fallback.html".to_owned(), page(FALLBACK_PAGE)),
        ("/card.html".to_owned(), page(CARD_FRAME)),
    ]))
    .await?;
    let url = json!(server.url("127.0.0.1", "/fallback.html"));
    let neo_script = format!(
        "try {{\n\
           await page.goto({url})\n\
           const frames = page.frames().map((frame) => frame.url())\n\
           const json = await page.ariaSnapshotJSON({{ mode: 'ai' }})\n\
           const full = await page.ariaSnapshot({{ mode: 'full', maxTokens: 100000 }})\n\
           const plain = await page.ariaSnapshot()\n\
           const text = await page.ariaSnapshot({{ text: true }})\n\
           const interactive = await page.ariaSnapshot({{ text: false }})\n\
           return {{ frames, json, full, plain, text, interactive }}\n\
         }} finally {{\n\
           await page.close()\n\
         }}"
    );
    let run = Arc::new(LiveRun::connect().await?);
    let host: Arc<dyn OpHost> = Arc::clone(&run) as Arc<dyn OpHost>;
    let init = json!({ "page": null });
    let neo = js_runner::run_script(&host, &init, &neo_script).await?;
    let card = server.url("localhost", "/card.html");
    assert_eq!(
        neo["frames"],
        json!([server.url("127.0.0.1", "/fallback.html"), card, card]),
        "both card frames loaded"
    );
    assert_fallback_is_read_into_the_tree(&neo["json"])?;

    let text = |key: &str| neo[key].as_str().unwrap_or_default().to_owned();
    let card_fields = |printed: &str| {
        printed
            .lines()
            .filter(|line| {
                line.trim_start()
                    .starts_with("- textbox \"Card number\" [ref=")
            })
            .count()
    };
    let full = text("full");
    assert_eq!(
        card_fields(&full),
        2,
        "mode 'full' prints both frames' card field with a ref: {full}"
    );
    let full_refs = printed_refs(&full);
    for form in ["full", "plain", "text", "interactive"] {
        let printed = text(form);
        assert!(
            !printed.contains("Your browser does not support iframes"),
            "{form} prints no fallback markup: {printed}"
        );
        let missing: Vec<String> = printed_refs(&printed)
            .difference(&full_refs)
            .cloned()
            .collect();
        assert!(
            missing.is_empty(),
            "{form} prints refs the full text lacks: {missing:?}"
        );
    }
    assert_eq!(card_fields(&text("plain")), 2, "the default prints both");

    let upstream: Arc<dyn OpHost> = Arc::new(Upstream(Arc::clone(&run)));
    let switched = js_runner::run_script(
        &upstream,
        &init,
        &format!(
            "try {{\n\
               await page.goto({url})\n\
               return await page.ariaSnapshot({{ mode: 'ai' }})\n\
             }} finally {{\n\
               await page.close()\n\
             }}"
        ),
    )
    .await?;
    let ai = switched.as_str().ok_or("Playwright's ai text")?;
    let fallback_line = ai
        .lines()
        .filter(|line| line.trim_start().starts_with("- iframe [ref="))
        .nth(1)
        .unwrap_or_default();
    assert!(
        fallback_line.ends_with(&format!("]: {FALLBACK_MARKUP}")),
        "under the switch the second iframe is Playwright's fallback line: {ai}"
    );
    assert_eq!(
        card_fields(ai),
        1,
        "under the switch only the first frame prints, as in Playwright: {ai}"
    );
    Ok(())
}

/// The cause, read from the engine's stitched tree: the first iframe has no `text`, the second
/// has the fallback markup as its `text`, and both hold their frame's content as children.
fn assert_fallback_is_read_into_the_tree(json: &Value) -> TestResult {
    let mut iframes = Vec::new();
    // Depth first in document order: children go on reversed, so the first comes off first.
    let mut pending: Vec<&Value> = json.as_array().into_iter().flatten().rev().collect();
    while let Some(node) = pending.pop() {
        if node["role"] == "iframe" {
            iframes.push(node);
        }
        pending.extend(node["children"].as_array().into_iter().flatten().rev());
    }
    let [plain, fallback] = iframes.as_slice() else {
        return Err(format!("two iframe nodes: {json}").into());
    };
    assert_eq!(plain.get("text"), None, "the empty iframe has no text");
    assert_eq!(
        fallback["text"], FALLBACK_MARKUP,
        "the engine reads the fallback markup as the iframe's text"
    );
    for iframe in [plain, fallback] {
        assert!(
            iframe["children"]
                .as_array()
                .is_some_and(|children| !children.is_empty()),
            "Rust stitches the frame under the iframe: {iframe}"
        );
    }
    Ok(())
}

/// Containers clickable marking must never pass as targets, as the v1 text prints them: a
/// `tabindex` pane of controls and pointer rows, a `tabindex` box of pointer rows, a `tabindex`
/// list of listener-only items, a listener card holding a note and a text-only `<article>` (a section the v1
/// text prints with its ref only with its text), and a `tabindex` card in a shadow root whose
/// slot holds assigned text over a fallback button (the engine never shows the fallback), and a
/// `tabindex` box holding a text-only region whose `role` is a token list (`region group`).
const MARKING_PAGE: &str = r##"<!doctype html><html><head><title>Marking</title></head><body>
<main>
<div tabindex="0"><a href="#docs">Docs</a> <button>Save</button>
<div style="cursor: pointer">Row one</div><div style="width: 20px; height: 20px; cursor: pointer"></div></div>
<div tabindex="0"><div style="cursor: pointer">Pointer row A</div><div style="cursor: pointer">Pointer row B</div></div>
<ul tabindex="0"><li><div class="item">Item A</div></li><li><div class="item">Item B</div></li></ul>
<div class="story"><span>Read on</span><article><p>Story text</p></article></div>
<div tabindex="0"><span>Note</span><div role="region group" aria-label="Notes">Region text</div></div>
<text-card>Card text</text-card>
<div class="menu">Open menu</div>
</main>
<script>
customElements.define('text-card', class extends HTMLElement {
  constructor() {
    super()
    this.attachShadow({ mode: 'open' }).innerHTML = '<div tabindex="0"><slot><button>Fallback</button></slot></div>'
  }
})
for (const element of document.querySelectorAll('.item, .story, .menu'))
  element.addEventListener('click', () => {})
</script>
</body></html>"##;

/// The live browser with marking switched off: the same ops without `clickable`, so the prelude
/// renders the same v1 text from an unmarked snapshot.
struct Unmarked(Arc<LiveRun>);

impl OpHost for Unmarked {
    fn invoke<'a>(
        &'a self,
        name: &'a str,
        mut input: Value,
    ) -> BoxFuture<'a, Result<Value, PlaywrightError>> {
        if let Some(input) = input.as_object_mut() {
            input.remove("clickable");
        }
        OpHost::invoke(&*self.0, name, input)
    }

    fn event_head(&self) -> u64 {
        OpHost::event_head(&*self.0)
    }

    fn page_facts(&self, tabs: Vec<TabId>) -> BoxFuture<'_, Vec<Value>> {
        OpHost::page_facts(&*self.0, tabs)
    }

    fn gesture(&self, page: TestPage, expression: String) -> BoxFuture<'_, Result<Value, String>> {
        OpHost::gesture(&*self.0, page, expression)
    }
}

/// Every ref a snapshot text prints, v1 or full.
fn printed_refs(text: &str) -> HashSet<String> {
    text.split("[ref=")
        .skip(1)
        .filter_map(|rest| rest.split(']').next())
        .map(str::to_owned)
        .collect()
}

/// The v1 text of a fresh copy of the page at `url` in both forms, first with marking switched
/// off and then with it on, and fails when marking removed a ref: every ref the unmarked text
/// prints, in either form, must be printed by the marked text of the same form. A fresh copy
/// mints the same refs, since marking changes no element's ref, only which ones print.
async fn unmarked_then_marked(url: &str) -> TestResult<(Value, Value)> {
    let script = format!(
        "try {{\n\
           await page.goto({url})\n\
           return {{ plain: await page.ariaSnapshot(), interactive: await page.ariaSnapshot({{ text: false }}) }}\n\
         }} finally {{\n\
           await page.close()\n\
         }}",
        url = json!(url),
    );
    let run = Arc::new(LiveRun::connect().await?);
    let init = json!({ "page": null });
    let unmarked: Arc<dyn OpHost> = Arc::new(Unmarked(Arc::clone(&run)));
    let unmarked = js_runner::run_script(&unmarked, &init, &script).await?;
    let marked: Arc<dyn OpHost> = run;
    let marked = js_runner::run_script(&marked, &init, &script).await?;
    for form in ["plain", "interactive"] {
        let before = unmarked
            .get(form)
            .and_then(Value::as_str)
            .ok_or("the unmarked text")?;
        let after = marked
            .get(form)
            .and_then(Value::as_str)
            .ok_or("the marked text")?;
        let lost: Vec<_> = printed_refs(before)
            .difference(&printed_refs(after))
            .cloned()
            .collect();
        assert!(
            lost.is_empty(),
            "{form}: marking removed {lost:?}\n--- unmarked\n{before}\n--- marked\n{after}"
        );
    }
    Ok((unmarked, marked))
}

/// Marking never removes a ref the v1 text prints: on a fresh copy of the same page, every ref
/// the unmarked text prints, in both forms, is printed by the marked text too. The article's
/// section line survives, the containers stay unmarked, and the listener items, the menu and the
/// shadow card whose slot holds text are marked.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_marking_never_removes_a_ref_the_v1_text_prints() -> TestResult {
    let _alone = ALONE.acquire().await?;
    let page = Page::html(MARKING_PAGE).with_header("cache-control", "no-store");
    let server = FixtureServer::start(HashMap::from([("/marking.html".to_owned(), page)])).await?;
    let (_, marked) = unmarked_then_marked(&server.url("127.0.0.1", "/marking.html")).await?;
    let plain = marked["plain"].as_str().ok_or("the marked text")?;
    assert!(
        plain.contains("- article [ref="),
        "the article keeps its line: {plain}"
    );
    assert!(
        plain.contains("- region \"Notes\" [ref="),
        "the region named by its first role token keeps its line: {plain}"
    );
    for marked_line in ["] [cursor=pointer]: Item A", "] [cursor=pointer]: Item B"] {
        assert!(plain.contains(marked_line), "{marked_line}: {plain}");
    }
    assert!(plain.contains("] [cursor=pointer]: Open menu"), "{plain}");
    assert!(
        plain.contains("] [cursor=pointer]: Card text"),
        "the shadow card whose slot holds text is a leaf: {plain}"
    );
    Ok(())
}

/// `tabindex` wrappers, each in a navigation of its own so its lines read alone (design.md
/// section 4, "A hidden control does not make a container"). The first four hold a control the
/// engine does not draw (`display: none`, a drop zone's hidden file input, a zero-size checkbox,
/// a hidden `nav`), so the wrapper is the only thing to click. The last three hold one it does
/// draw: a button, a 1-pixel screen-reader checkbox, and an `aria-hidden` button, which ai mode
/// prints with its ref.
const HIDDEN_CONTROLS_PAGE: &str = r##"<!doctype html><html><head><title>Hidden controls</title></head><body>
<main>
<nav aria-label="display none"><div tabindex="0">Open menu<button style="display: none">Hidden</button></div></nav>
<nav aria-label="drop zone"><div tabindex="0">Drop files here<input type="file" style="display: none"></div></nav>
<nav aria-label="zero size"><div tabindex="0"><input type="checkbox" style="opacity: 0; width: 0; height: 0; margin: 0; padding: 0; border: 0"><span>Notifications</span></div></nav>
<nav aria-label="hidden nav"><div tabindex="0">Hidden section<nav style="display: none"><a href="#docs">Docs</a></nav></div></nav>
<nav aria-label="drawn control"><div tabindex="0">Visible control <button>Save</button></div></nav>
<nav aria-label="screen reader"><div tabindex="0"><input type="checkbox" style="position: absolute; width: 1px; height: 1px; overflow: hidden; clip: rect(0 0 0 0)"><span>Subscribe</span></div></nav>
<nav aria-label="aria hidden"><div tabindex="0">Card text <button aria-hidden="true" tabindex="-1">x</button></div></nav>
</main>
</body></html>"##;

/// Marking's guard counts only what the engine draws: a wrapper whose only control is not drawn
/// goes from a text line to a marked line, and a wrapper around a drawn control, a 1-pixel
/// checkbox or a drawn `aria-hidden` button stays exactly as unmarked. No ref is lost either way.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_control_the_engine_does_not_draw_does_not_make_a_container() -> TestResult {
    let _alone = ALONE.acquire().await?;
    let page = Page::html(HIDDEN_CONTROLS_PAGE).with_header("cache-control", "no-store");
    let server = FixtureServer::start(HashMap::from([("/hidden.html".to_owned(), page)])).await?;
    let (unmarked, marked) = unmarked_then_marked(&server.url("127.0.0.1", "/hidden.html")).await?;
    let unmarked = without_refs(unmarked["plain"].as_str().ok_or("the unmarked text")?);
    let marked = without_refs(marked["plain"].as_str().ok_or("the marked text")?);
    for (name, text) in [
        ("display none", "Open menu"),
        ("drop zone", "Drop files here"),
        ("zero size", "Notifications"),
        ("hidden nav", "Hidden section"),
    ] {
        assert_eq!(
            block(&unmarked, name),
            [format!("- text: {text}")],
            "{name}, unmarked: {unmarked}"
        );
        assert_eq!(
            block(&marked, name),
            [format!("- generic [ref] [cursor=pointer]: {text}")],
            "{name}, marked: {marked}"
        );
    }
    for name in ["drawn control", "screen reader", "aria hidden"] {
        let lines = block(&marked, name);
        assert!(
            !lines.is_empty() && lines == block(&unmarked, name),
            "{name} stays as unmarked:\n--- unmarked\n{unmarked}\n--- marked\n{marked}"
        );
        assert!(
            lines.iter().all(|line| !line.contains("[cursor=pointer]")),
            "{name}: {marked}"
        );
    }
    Ok(())
}
