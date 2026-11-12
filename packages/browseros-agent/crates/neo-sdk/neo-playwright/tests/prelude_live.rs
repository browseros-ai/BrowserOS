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

use std::collections::HashMap;
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

/// The goldens recorded from playwright-core 1.64.1: each expression's selector and `toString()`.
const SELECTOR_GOLDENS: &str =
    include_str!("../../neo-playwright-js/tests/data/selector-goldens.json");

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
