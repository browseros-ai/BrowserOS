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
use std::{env, fs};

use browseros_core::TabId;
use fixture_server::{FixtureServer, Page};
use futures_util::future::BoxFuture;
use js_runner::OpHost;
use live_run::{LiveRun, TestPage, TestResult};
use neo_playwright::{ErrorCode, PlaywrightError};
use serde_json::{Value, json};
use tokio::sync::Semaphore;

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
        first.starts_with("# full snapshot: no earlier snapshot of this page\n- main [ref="),
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
        text("otherFirst").starts_with("# full snapshot: no earlier snapshot of this page\n"),
        "another tab has no baseline of its own yet"
    );
    assert_eq!(
        text("again"),
        text("collapsed"),
        "the same run's snapshots stay pending: this one still compares against the first \
         run's text, and the other tab's snapshot left it alone"
    );
    assert!(
        text("navigated").starts_with("# full snapshot: no earlier snapshot of this page\n"),
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
           return {{ plain, interactive, json }}\n\
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
    Ok(())
}

/// Snapshot v1's clickable marking (design section 4), as twins: each element under `listeners`
/// has only a click listener and the default cursor; its twin under `pointers` has a real
/// `cursor: pointer` and no listener. The cases: a `div` with text, an empty tile, a `div`
/// around a named icon, a shadow `div` whose named icon is slotted into it (CSS inheritance
/// follows the slot), a `div` wrapped in another together with a note, and a `div` that is its
/// wrapper's only child (which upstream's distiller inlines into the wrapper, real cursor or not;
/// design.md section 4, what the engine still loses). A container whose listener only relays its buttons' clicks
/// follows. The listeners are the page's own script.
const CLICKABLE_PAGE: &str = r#"<!doctype html><html><head><title>Clickable</title></head><body>
<main>
<nav aria-label="listeners">
<div class="menu">Open menu</div>
<div class="tile" style="width: 40px; height: 40px"></div>
<div class="icon"><img alt="Settings" width="16" height="16" src="data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='16' height='16'/%3E"></div>
<icon-box data-hit><img alt="Slotted" width="16" height="16" src="data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='16' height='16'/%3E"></icon-box>
<div><div class="inner">Wrapped with</div><span>a note</span></div>
<div><div class="inner">Wrapped</div></div>
</nav>
<nav aria-label="pointers">
<div style="cursor: pointer">Open menu</div>
<div style="width: 40px; height: 40px; cursor: pointer"></div>
<div style="cursor: pointer"><img alt="Settings" width="16" height="16" src="data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='16' height='16'/%3E"></div>
<icon-box data-pointer><img alt="Slotted" width="16" height="16" src="data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='16' height='16'/%3E"></icon-box>
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
for (const element of listeners.querySelectorAll('.menu, .tile, .icon, .inner'))
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
/// element prints exactly as its twin with a real `cursor: pointer` does, and a container that
/// relays its buttons' clicks prints no line of its own. Under the spec harness's conformance
/// switch the same page prints Playwright's own text: the listener-only elements have no mark.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_listener_only_elements_print_as_their_pointer_twins_and_the_switch_leaves_them()
-> TestResult {
    let _alone = ALONE.acquire().await?;
    let page = Page::html(CLICKABLE_PAGE).with_header("cache-control", "no-store");
    let server =
        FixtureServer::start(HashMap::from([("/clickable.html".to_owned(), page)])).await?;
    let url = server.url("127.0.0.1", "/clickable.html");
    let run = Arc::new(LiveRun::connect().await?);
    let host: Arc<dyn OpHost> = Arc::clone(&run) as Arc<dyn OpHost>;
    let init = json!({ "page": null });
    let v1 = js_runner::run_script(&host, &init, &clickable_script(&url)).await?;
    let plain = v1["plain"].as_str().ok_or("the v1 text")?;
    let twins = [
        "- generic [ref] [cursor=pointer]: Open menu",
        "- generic [ref] [cursor=pointer]",
        "- img \"Settings\" [ref] [cursor=pointer]",
        "- img \"Slotted\" [ref] [cursor=pointer]",
        "- generic [ref] [cursor=pointer]: Wrapped with",
        "- text: a note Wrapped",
    ];
    assert_eq!(block(plain, "listeners"), twins, "{plain}");
    assert_eq!(block(plain, "pointers"), twins, "{plain}");
    assert!(
        plain.ends_with(
            "  - button \"Archive\" [ref]\n  - button \"Delete\" [ref]\n  - text: Plain text"
        ),
        "the relaying container prints no line of its own: {plain}"
    );
    assert_eq!(v1["plain"], v1["ai"], "no mode is mode 'ai'");
    assert_eq!(v1["after"], "Menu open", "a click through the printed ref");

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

/// Every ref a v1 text prints.
fn printed_refs(text: &str) -> HashSet<String> {
    text.split("[ref=")
        .skip(1)
        .filter_map(|rest| rest.split(']').next())
        .map(str::to_owned)
        .collect()
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
    let script = format!(
        "try {{\n\
           await page.goto({url})\n\
           return {{ plain: await page.ariaSnapshot(), interactive: await page.ariaSnapshot({{ text: false }}) }}\n\
         }} finally {{\n\
           await page.close()\n\
         }}",
        url = json!(server.url("127.0.0.1", "/marking.html")),
    );
    let run = Arc::new(LiveRun::connect().await?);
    let init = json!({ "page": null });
    let unmarked: Arc<dyn OpHost> = Arc::new(Unmarked(Arc::clone(&run)));
    let unmarked = js_runner::run_script(&unmarked, &init, &script).await?;
    let marked: Arc<dyn OpHost> = run;
    let marked = js_runner::run_script(&marked, &init, &script).await?;
    for form in ["plain", "interactive"] {
        let before = unmarked[form].as_str().ok_or("the unmarked text")?;
        let after = marked[form].as_str().ok_or("the marked text")?;
        let lost: Vec<_> = printed_refs(before)
            .difference(&printed_refs(after))
            .cloned()
            .collect();
        assert!(
            lost.is_empty(),
            "{form}: marking removed {lost:?}\n--- unmarked\n{before}\n--- marked\n{after}"
        );
    }
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
