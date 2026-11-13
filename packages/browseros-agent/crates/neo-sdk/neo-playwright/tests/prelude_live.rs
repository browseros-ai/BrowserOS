//! Runner N (R13): Playwright scripts as agents write them, run by the real prelude in QuickJS
//! (`tests/support/js_runner.rs`), with every op going through `Playwright::invoke`. This is the
//! end-to-end check of the wire contract: wire-examples.json proves the inputs decode, and these
//! scripts prove the outputs read back as Playwright's values: navigation responses, reads, the
//! default and v1 snapshot texts and refs from them, the frame tree, popups through a wait and
//! through the event pump, and page handles that keep their identity.
//!
//! The offline test needs no browser. The live test needs a throwaway BrowserOS with CDP on
//! `BROWSEROS_CDP_PORT` (default 9005): `bun run test:browser -- cargo test -p neo-playwright --test prelude_live
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

/// A ref's caller crosses a run boundary, while its node stays in the document's isolated
/// world. The child is on another site so the saved parent path must survive an OOPIF jump.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_locator_refs_keep_node_identity_across_runs_and_cross_site_frames() -> TestResult
{
    let _alone = ALONE.acquire().await?;
    let child = r#"<input aria-label="Verification code" style="width:0;height:0;padding:0;border:0;position:absolute">
        <button style="pointer-events:none" onclick="this.dataset.clicked='yes'">Frame action</button>"#;
    let server = FixtureServer::start(HashMap::from([
        (
            "/nosize".to_owned(),
            Page::html(include_str!("data/prelude/nosize.html")),
        ),
        ("/child".to_owned(), Page::html(child)),
    ]))
    .await?;
    let run = Arc::new(LiveRun::connect().await?);
    let host: Arc<dyn OpHost> = Arc::clone(&run) as Arc<dyn OpHost>;
    let first = js_runner::run_script(
        &host,
        &json!({ "page": null }),
        &format!(
            "await page.goto({url});\n\
             await page.evaluate(src => {{ const iframe = document.createElement('iframe'); \
                 iframe.src = src; document.body.append(iframe); }}, {child});\n\
             await page.frameLocator('iframe').getByRole('textbox').waitFor({{ state: 'attached' }});\n\
             {checks}",
            url = json!(server.url("127.0.0.1", "/nosize")),
            child = json!(server.url("localhost", "/child")),
            checks = include_str!("data/prelude/locator-refs-first.js"),
        ),
    ).await?;
    run.end_run(Some(&first));
    let tab = first["tab"].as_i64().ok_or("the first run keeps its tab")?;
    let second = js_runner::run_script(
        &host,
        &json!({ "page": { "$neo": "page", "tab": tab } }),
        &format!(
            "const saved = {first};\ntry {{ {checks} }} finally {{ await page.close(); }}",
            checks = include_str!("data/prelude/locator-refs-second.js"),
        ),
    )
    .await?;
    assert_eq!(second["checked"], true, "{second}");
    Ok(())
}

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

/// Synthetic page declarations shared by the main document, an open shadow root and an
/// out-of-process child. Values are deliberately distinct from their accessible names.
fn private_fields(scope: &str) -> String {
    format!(
        r#"<section aria-label="{scope}">
<input aria-label="{scope} password" type="password" value="Pw🔒">
<input aria-label="{scope} shown password" autocomplete="current-password" value="ShowMe">
<input aria-label="{scope} card" autocomplete="CC-NUMBER" value="4111-SECRET">
<input aria-label="{scope} security" autocomplete="section-pay shipping cc-csc" value="Cvc">
<input aria-label="{scope} code" autocomplete="one-time-code" value="Otp246">
<textarea aria-label="{scope} new password" autocomplete="new-password">New
Pass</textarea>
<input aria-label="{scope} empty" type="password">
<input aria-label="{scope} expiry" autocomplete="cc-exp" value="12/34">
<input aria-label="{scope} holder" autocomplete="cc-name" value="Ada Lovelace">
<input aria-label="{scope} unlabelled card" value="4111111111111111">
<input aria-label="{scope} off" autocomplete="off" value="Readable off">
<input aria-label="{scope} near token" autocomplete="cc-numberx" value="Readable near">
<select aria-label="{scope} month" autocomplete="cc-exp-month"><option selected>December</option></select>
<input type="submit" value="Continue {scope}">
</section>{}"#,
        private_canary_fields(scope)
    )
}

/// These controls exercise alternate value/name reads, with different stored and live values.
/// Every private canary is scanned across whole results, including another control's name.
fn private_canary_fields(scope: &str) -> String {
    format!(
        r##"<section aria-label="{scope} alternate reads">
<input type="password" role="combobox" value="SecretCanary{scope}-password-attribute" data-typed="SecretCanary{scope}-password-value">
<label>{scope} wrapped search<input autocomplete="current-password" role="searchbox" value="SecretCanary{scope}-shown-attribute" data-typed="SecretCanary{scope}-shown-value"></label>
<input id="{scope}-number" type="number" autocomplete="cc-number" value="902101" data-typed="902102">
<button aria-labelledby="{scope}-number"></button>
<input id="{scope}-slider" autocomplete="cc-csc" role="slider" value="SecretCanary{scope}-slider-attribute" data-typed="SecretCanary{scope}-slider-value" aria-valuetext="SecretCanary{scope}-slider-text" aria-valuenow="SecretCanary{scope}-slider-now">
<button aria-labelledby="{scope}-slider"></button>
<input id="{scope}-spin" autocomplete="one-time-code" role="spinbutton" value="SecretCanary{scope}-spin-attribute" data-typed="SecretCanary{scope}-spin-value" aria-valuenow="SecretCanary{scope}-spin-now">
<button aria-labelledby="{scope}-spin"></button>
<label>{scope} wrapped textarea<textarea autocomplete="new-password" role="searchbox" value="SecretCanary{scope}-textarea-attribute" data-typed="SecretCanary{scope}-textarea-value">SecretCanary{scope}-textarea-content</textarea></label>
<textarea id="{scope}-self" aria-labelledby="{scope}-self" role="button" autocomplete="new-password" value="SecretCanary{scope}-self-attribute" data-typed="SecretCanary{scope}-self-value">SecretCanary{scope}-self-content</textarea>
<textarea id="{scope}-named" role="combobox" autocomplete="new-password" value="SecretCanary{scope}-named-attribute" data-typed="SecretCanary{scope}-named-value" aria-valuetext="SecretCanary{scope}-named-text" aria-valuenow="SecretCanary{scope}-named-now">SecretCanary{scope}-named-content</textarea>
<button aria-labelledby="{scope}-named"></button>
<a href="#private"><input autocomplete="new-password" value="SecretCanary{scope}-link-attribute" data-typed="SecretCanary{scope}-link-value"></a>
<button><input autocomplete="cc-csc" value="SecretCanary{scope}-button-attribute" data-typed="SecretCanary{scope}-button-value"></button>
<input role="searchbox" value="PublicCanary{scope}-input-attribute" data-typed="PublicCanary{scope}-input-value">
<textarea role="searchbox" value="PublicCanary{scope}-textarea-attribute" data-typed="PublicCanary{scope}-textarea-value">PublicCanary{scope}-textarea-content</textarea>
</section>"##
    )
}

/// Runs in the page world before a snapshot. Textareas retain distinct default text after
/// their live value is changed; the same setup applies to the document and its shadow root.
const PRIVATE_FIELD_SETUP: &str = r"
for (const field of root.querySelectorAll('[data-typed]'))
  field.value = field.getAttribute('data-typed')
";

fn private_field_pages() -> HashMap<String, Page> {
    let top = format!(
        r#"<!doctype html><html><head><title>Private fields</title></head><body><main>
{}<div id="shadow"></div><iframe id="cross" title="Cross"></iframe></main>
<script>
const shadow = document.querySelector('#shadow').attachShadow({{ mode: 'open' }});
shadow.innerHTML = {};
for (const root of [document, shadow]) {{ {} }}
document.querySelector('#cross').src = `http://localhost:${{location.port}}/private-child.html`;
</script></body></html>"#,
        private_fields("Main"),
        json!(private_fields("Shadow")),
        PRIVATE_FIELD_SETUP,
    );
    let child = format!(
        "<!doctype html><html><body>{}<script>const root = document; {}</script></body></html>",
        private_fields("Cross"),
        PRIVATE_FIELD_SETUP,
    );
    HashMap::from([
        ("/private.html".to_owned(), Page::html(&top)),
        ("/private-child.html".to_owned(), Page::html(&child)),
    ])
}

/// Text and JSON must expose the same lengths after an edit, while ordinary values survive.
const PRIVATE_FIELD_EXPECTATIONS: &[(&str, &str)] = &[
    ("password", "••••"),
    ("shown password", "•••••"),
    ("card", "•••••••••••"),
    ("security", "•••"),
    ("code", "••••••"),
    ("new password", "••••••"),
    ("expiry", "12/34"),
    ("holder", "Ada Lovelace"),
    ("unlabelled card", "4111111111111111"),
    ("off", "Readable off"),
    ("near token", "Readable near"),
];

/// Verifies each text format at the field line, so a surviving label or an unrelated bullet
/// elsewhere cannot stand in for the masked value. JSON is checked independently below.
fn assert_private_field_text(text: &str, scope: &str) -> TestResult {
    for (name, value) in PRIVATE_FIELD_EXPECTATIONS {
        let label = format!("textbox \"{scope} {name}\"");
        let line = text
            .lines()
            .find(|line| line.contains(&label))
            .ok_or_else(|| format!("missing {label} in {text}"))?;
        assert!(
            line.ends_with(&format!(": {value}")) || line.ends_with(&format!(": \"{value}\"")),
            "{line}"
        );
    }
    let empty = format!("textbox \"{scope} empty\"");
    assert!(
        text.lines().any(|line| line.trim_end().ends_with(&empty)),
        "empty fields have no value: {text}"
    );
    assert!(text.contains(&format!("button \"Continue {scope}\"")));
    assert!(text.contains(&format!("combobox \"{scope} month\"")));
    assert!(text.contains("option \"December\" [selected]"));
    Ok(())
}

/// Finds the field in the stitched JSON without depending on its frame or shadow wrappers.
fn private_field_node<'tree>(tree: &'tree Value, name: &str) -> Option<&'tree Value> {
    match tree {
        Value::Array(nodes) => nodes.iter().find_map(|node| private_field_node(node, name)),
        Value::Object(node) if node.get("name").and_then(Value::as_str) == Some(name) => Some(tree),
        Value::Object(node) => node
            .get("children")
            .and_then(|children| private_field_node(children, name)),
        _ => None,
    }
}

/// Every output route is collected from the same live controls, then direct reads verify that
/// the scoped prototype guards have already been removed. These reads are deliberately not
/// part of the snapshot canary scan: explicit inputValue() remains a clear-value API.
const PRIVATE_SNAPSHOT_SCRIPT: &str = r"
      await page.goto(__FIXTURE_URL__)
      const cross = page.locator('#cross').contentFrame().locator('body')
      await cross.getByLabel('Cross password', { exact: true }).waitFor()
      for (const [root, scope] of [[page, 'Main'], [cross, 'Cross'], [page.locator('#shadow'), 'Shadow']]) {
        await root.getByLabel(scope + ' shown password', { exact: true }).fill('Typed')
        await root.getByLabel(scope + ' new password', { exact: true }).fill('Edit🔒')
      }
      const withoutRefs = text => text.replace(/ \[ref=[^\]]+\]/g, '')
      async function collect(root, scope) {
        const texts = {}
        const reads = []
        async function readValues() {
          return await Promise.all(['password', 'shown password', 'new password'].map(
            name => root.getByLabel(scope + ' ' + name, { exact: true }).inputValue(),
          ))
        }
        for (const [name, options] of [
          ['automatic', {}], ['text', { text: true }], ['noText', { text: false }],
          ['full', { mode: 'full' }], ['default', { mode: 'default' }],
        ]) {
          texts[name] = withoutRefs(await root.ariaSnapshot(options))
          reads.push(await readValues())
        }
        const trees = []
        for (const mode of ['ai', 'default']) {
          trees.push(await root.ariaSnapshotJSON({ mode }))
          reads.push(await readValues())
        }
        return { texts, trees, reads }
      }
      const normal = await collect(page, 'Main')
      const child = await collect(cross, 'Cross')
      const shadow = await collect(page.locator('#shadow'), 'Shadow')
      await page.getByLabel('Main password', { exact: true }).focus()
      const baseline = await page.ariaSnapshot({ text: true })
      return { normal, child, shadow, baseline, tab: page.toJSON().tab }
";

const PRIVATE_CONFORMANCE_SCRIPT: &str = r"try {
  const cross = page.locator('#cross').contentFrame().locator('body')
  return {
    page: await page.ariaSnapshot({ mode: 'ai' }),
    child: await cross.ariaSnapshot({ mode: 'default' }),
    tree: await page.ariaSnapshotJSON({ mode: 'ai' }),
  }
} finally { await page.close() }";

fn assert_private_canaries_absent(value: &Value) -> TestResult {
    let encoded = serde_json::to_string(value)?;
    for canary in [
        "Pw🔒",
        "ShowMe",
        "Typed",
        "Edit🔒",
        "4111-SECRET",
        "Cvc",
        "Otp246",
        "New\\nPass",
        "Other",
        "SecretCanary",
        "902101",
        "902102",
    ] {
        assert!(!encoded.contains(canary), "leaked {canary}: {encoded}");
    }
    Ok(())
}

fn assert_public_canaries_present(text: &str, scope: &str) {
    for kind in ["input", "textarea"] {
        let visible = format!("PublicCanary{scope}-{kind}-value");
        assert!(text.contains(&visible), "missing {visible}: {text}");
    }
}

fn assert_private_field_tree(tree: &Value, scope: &str) -> TestResult {
    for (name, value) in PRIVATE_FIELD_EXPECTATIONS {
        let label = format!("{scope} {name}");
        let node =
            private_field_node(tree, &label).ok_or_else(|| format!("missing {label} in {tree}"))?;
        assert_eq!(node["text"].as_str(), Some(*value), "{node}");
    }
    let empty = private_field_node(tree, &format!("{scope} empty")).ok_or("the empty field")?;
    assert!(empty.get("text").is_none(), "{empty}");
    assert_public_canaries_present(&serde_json::to_string(tree)?, scope);
    Ok(())
}

fn assert_private_snapshot_formats(opened: &Value, key: &str, scopes: &[&str]) -> TestResult {
    assert_private_canaries_absent(&opened[key]["texts"])?;
    assert_private_canaries_absent(&opened[key]["trees"])?;
    let texts = opened[key]["texts"].as_object().ok_or("the text formats")?;
    for (format, value) in texts {
        let text = value.as_str().ok_or("a snapshot text")?;
        for scope in scopes {
            // Default mode reads one frame. Its cross-origin counterpart is verified through
            // the child locator, preserving upstream's existing traversal contract.
            if key == "normal" && format == "default" && *scope == "Cross" {
                continue;
            }
            assert_private_field_text(text, scope)?;
            assert_public_canaries_present(text, scope);
        }
    }
    assert_eq!(
        opened[key]["reads"],
        json!(vec![json!(["Pw🔒", "Typed", "Edit🔒"]); 7])
    );
    for (format, tree) in opened[key]["trees"]
        .as_array()
        .ok_or("the JSON formats")?
        .iter()
        .enumerate()
    {
        for scope in scopes {
            if key == "normal" && format == 1 && *scope == "Cross" {
                continue;
            }
            assert_private_field_tree(tree, scope)?;
        }
    }
    Ok(())
}

fn assert_private_value_delta(grown: &Value) -> TestResult {
    assert_private_canaries_absent(grown)?;
    let delta = grown.as_str().ok_or("the grown value's delta")?;
    assert!(
        delta.starts_with("# changes since your last snapshot of this tab: 1 added, 1 removed"),
        "{delta}"
    );
    for (prefix, value) in [('-', "••••"), ('+', "•••••")] {
        assert!(
            delta
                .lines()
                .any(|line| line.starts_with(prefix) && line.ends_with(&format!(": {value}"))),
            "{delta}"
        );
    }
    Ok(())
}

fn assert_conformance_private_fields_are_clear(unmasked: &Value) -> TestResult {
    for key in ["page", "child", "tree"] {
        let text = serde_json::to_string(&unmasked[key])?;
        for canary in [
            "Typed",
            "Edit🔒",
            "4111-SECRET",
            "Cvc",
            "Otp246",
            "SecretCanary",
        ] {
            assert!(text.contains(canary), "conformance {key}: {text}");
        }
        assert!(!text.contains('•'), "conformance {key}: {text}");
    }
    Ok(())
}

/// Each frame masks before protocol serialization. This exercises the normal marked page and
/// pinned locator routes, the unmarked/default/JSON routes, shadow DOM, and cross-run baselines.
/// Direct reads after every walk still see the original field value.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_private_field_values_are_masked_in_every_snapshot_format() -> TestResult {
    let _alone = ALONE.acquire().await?;
    let server = FixtureServer::start(private_field_pages()).await?;
    let run = Arc::new(LiveRun::connect().await?);
    let host: Arc<dyn OpHost> = Arc::clone(&run) as Arc<dyn OpHost>;
    let script = PRIVATE_SNAPSHOT_SCRIPT.replace(
        "__FIXTURE_URL__",
        &json!(server.url("127.0.0.1", "/private.html")).to_string(),
    );
    let opened = js_runner::run_script(&host, &json!({ "page": null }), &script).await?;
    run.end_run(Some(&opened));
    let tab = opened["tab"].as_i64().ok_or("the page's tab")?;
    let init = json!({ "page": { "$neo": "page", "tab": tab } });
    assert_private_snapshot_formats(&opened, "normal", &["Main", "Shadow", "Cross"])?;
    assert_private_snapshot_formats(&opened, "child", &["Cross"])?;
    assert_private_snapshot_formats(&opened, "shadow", &["Shadow"])?;
    assert_private_canaries_absent(&opened["baseline"])?;
    let grown = js_runner::run_script(
        &host,
        &init,
        "await page.getByLabel('Main password', { exact: true }).fill('Pw🔒x');
         return await page.ariaSnapshot({ delta: true })",
    )
    .await?;
    assert_private_value_delta(&grown)?;
    run.end_run(Some(&grown));
    let same_length = js_runner::run_script(
        &host,
        &init,
        "await page.getByLabel('Main password', { exact: true }).fill('Other');
         return await page.ariaSnapshot({ delta: true })",
    )
    .await?;
    assert_private_canaries_absent(&same_length)?;
    assert_eq!(
        same_length,
        json!("# no change since your last snapshot of this tab")
    );

    let upstream: Arc<dyn OpHost> = Arc::new(Upstream(Arc::clone(&run)));
    let unmasked = js_runner::run_script(&upstream, &init, PRIVATE_CONFORMANCE_SCRIPT).await?;
    assert_conformance_private_fields_are_clear(&unmasked)
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

/// The page for an iframe's fallback markup (design section 13): the same cross-site card frame
/// twice, the first iframe written empty and the second with fallback markup between its tags,
/// as the card frames of a real donation page carry it.
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

/// Closed shadow roots (design.md section 18): a button, a text box, a frame and a line of text
/// in a closed root; a closed root holding a section that holds a second closed root; and a
/// closed root that shows its host's own button through a slot. The page's own script keeps
/// every root, so only the page can reach inside one; the button counts its clicks. Thirty
/// ordinary links follow, so that one changed line is a delta and not most of the page.
const CLOSED_PAGE: &str = r##"<!doctype html><html><head><title>Closed</title></head><body>
<main>
<h1>Closed roots</h1>
<div id="host"></div>
<div id="outer"></div>
<div id="slotted"><button>Slotted button</button></div>
</main>
<script>
const host = document.getElementById('host').attachShadow({ mode: 'closed' })
host.innerHTML = '<button>Closed button</button><input aria-label="Closed box"><iframe title="Inner frame" src="/closed/frame.html"></iframe><p>Clicked 0</p>'
let clicks = 0
host.querySelector('button').addEventListener('click', () => { host.querySelector('p').textContent = `Clicked ${++clicks}` })
const outer = document.getElementById('outer').attachShadow({ mode: 'closed' })
outer.innerHTML = '<section aria-label="Outer"><p>Outer text</p><div></div></section>'
outer.querySelector('div').attachShadow({ mode: 'closed' }).innerHTML = '<button>Nested button</button>'
document.getElementById('slotted').attachShadow({ mode: 'closed' }).innerHTML = '<p>Slot text</p><slot></slot>'
document.querySelector('main').insertAdjacentHTML('beforeend', Array.from({ length: 30 }, (_, i) => `<p><a href="#s${i}">Story ${i}</a></p>`).join(''))
</script>
</body></html>"##;

/// The frame inside the first closed root. Two elements, since the engine inlines a frame
/// body's only child into the frame's line.
const CLOSED_FRAME: &str =
    "<!doctype html><html><body><button>Frame button</button><p>Frame text</p></body></html>";

/// The ref the first line of `text` holding `line` prints.
fn ref_on(text: &str, line: &str) -> TestResult<String> {
    let found = text
        .lines()
        .find(|printed| printed.contains(line))
        .ok_or_else(|| format!("no line with {line}:\n{text}"))?;
    let r#ref = found
        .split("[ref=")
        .nth(1)
        .and_then(|rest| rest.split(']').next())
        .ok_or_else(|| format!("no ref on {found}"))?;
    Ok(r#ref.to_owned())
}

/// What is inside a closed root prints, and is acted on, as inside an open one: the default
/// format, `{ text: true }`, `{ text: false }` and `mode: 'full'` all print the button, the
/// text box, the frame with its content, and the button inside the nested closed root, with
/// refs; the slotted button prints once beside the closed root's own text. A click and a fill
/// through the printed refs reach the page's own elements, a role locator finds the nested
/// button once a snapshot has read the page, and the page's own script still reads `null` from
/// every closed host. A delta after a change inside the root lists it. Under the spec harness's
/// conformance switch, on the same page, nothing inside a closed root prints and no DOM capture
/// is made.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_closed_shadow_root_reads_and_acts_as_an_open_one() -> TestResult {
    let _alone = ALONE.acquire().await?;
    let page = |html: &str| Page::html(html).with_header("cache-control", "no-store");
    let server = FixtureServer::start(HashMap::from([
        ("/closed/page.html".to_owned(), page(CLOSED_PAGE)),
        ("/closed/frame.html".to_owned(), page(CLOSED_FRAME)),
    ]))
    .await?;
    let url = json!(server.url("127.0.0.1", "/closed/page.html"));
    let run = Arc::new(LiveRun::connect().await?);
    let host: Arc<dyn OpHost> = Arc::clone(&run) as Arc<dyn OpHost>;
    let read = js_runner::run_script(
        &host,
        &json!({ "page": null }),
        &format!(
            "await page.goto({url})\n\
             const plain = await page.ariaSnapshot()\n\
             const text = await page.ariaSnapshot({{ text: true }})\n\
             const interactive = await page.ariaSnapshot({{ text: false }})\n\
             const full = await page.ariaSnapshot({{ mode: 'full' }})\n\
             const refOn = (line) => interactive.split('\\n').find((l) => l.includes(line)).match(/\\[ref=([^\\]]+)\\]/)[1]\n\
             await page.getByRef(refOn('button \"Closed button\"')).click()\n\
             const box = page.getByRef(refOn('textbox \"Closed box\"'))\n\
             await box.fill('typed')\n\
             const nested = await page.getByRole('button', {{ name: 'Nested button' }}).count()\n\
             const clicked = await page.getByText('Clicked 1').count()\n\
             const value = await box.inputValue()\n\
             const pageReads = await page.evaluate(() => ['host', 'outer', 'slotted'].map((id) => document.getElementById(id).shadowRoot))\n\
             return {{ plain, text, interactive, full, nested, clicked, value, pageReads, tab: page.toJSON().tab }}"
        ),
    )
    .await?;
    assert_closed_content_reads(&read);
    // The reading run is dropped; the delta's own runs make and read the baseline.
    run.end_run(None);
    let tab = read["tab"].as_i64().ok_or("the page's tab")?;
    let init = json!({ "page": { "$neo": "page", "tab": tab } });
    assert_closed_change_is_listed(&run, &host, &init).await?;
    js_runner::run_script(&host, &init, "await page.close()").await?;
    assert_the_switch_keeps_roots_closed(&run, &url).await
}

/// Every form prints what is inside the closed roots, and the actions through its refs land.
fn assert_closed_content_reads(read: &Value) {
    let field = |key: &str| read.get(key).cloned().unwrap_or_default();
    let form = |key: &str| field(key).as_str().unwrap_or_default().to_owned();
    for key in ["plain", "text", "interactive", "full"] {
        let printed = form(key);
        for line in [
            "- button \"Closed button\" [ref=",
            "- textbox \"Closed box\" [ref=",
            "- iframe [ref=",
            "- button \"Frame button\" [ref=f",
            "- button \"Nested button\" [ref=",
            "- button \"Slotted button\" [ref=",
        ] {
            assert!(printed.contains(line), "{key}: {line}\n{printed}");
        }
        assert_eq!(
            printed.matches("Slotted button").count(),
            1,
            "{key}: the slotted button prints once, where the slot shows it\n{printed}"
        );
        // The interactive form leaves text out; every other form has it, and shows the slotted
        // button where the slot is, after the root's own text.
        if key != "interactive" {
            for words in ["Clicked 0", "Outer text", "Slot text", "Frame text"] {
                assert!(printed.contains(words), "{key}: {words}\n{printed}");
            }
            assert!(
                printed.find("Slot text") < printed.find("Slotted button"),
                "{key}: the slot shows the button after the root's text\n{printed}"
            );
        }
    }
    assert_eq!(
        field("nested"),
        1,
        "a role locator reaches the nested closed root"
    );
    assert_eq!(
        field("clicked"),
        1,
        "the click through the ref reached the page"
    );
    assert_eq!(
        field("value"),
        "typed",
        "the fill through the ref reached the page"
    );
    assert_eq!(
        field("pageReads"),
        json!([null, null, null]),
        "the page's own world still reads null from every closed host"
    );
}

/// One run returns the page whole, which makes the tab's baseline; the next clicks inside the
/// closed root, and its delta lists the change.
async fn assert_closed_change_is_listed(
    run: &LiveRun,
    host: &Arc<dyn OpHost>,
    init: &Value,
) -> TestResult {
    let base =
        js_runner::run_script(host, init, "return { base: await page.ariaSnapshot() }").await?;
    run.end_run(Some(&base));
    let button = ref_on(
        base.get("base")
            .and_then(Value::as_str)
            .ok_or("the baseline")?,
        "- button \"Closed button\"",
    )?;
    let changed = js_runner::run_script(
        host,
        init,
        &format!(
            "await page.getByRef({button}).click()\n\
             return {{ delta: await page.ariaSnapshot({{ delta: true }}) }}",
            button = json!(button)
        ),
    )
    .await?;
    let delta = changed
        .get("delta")
        .and_then(Value::as_str)
        .ok_or("the delta")?;
    let listed = |mark: char, words: &str| {
        delta
            .lines()
            .any(|line| line.starts_with(mark) && line.ends_with(&format!("- text: {words}")))
    };
    assert!(
        delta.starts_with("# changes since your last snapshot of this tab:")
            && listed('-', "Clicked 1")
            && listed('+', "Clicked 2"),
        "the change inside the closed root is listed: {delta}"
    );
    Ok(())
}

/// Under the switch: Playwright's own text, without what the closed roots hold, and no capture.
async fn assert_the_switch_keeps_roots_closed(run: &Arc<LiveRun>, url: &Value) -> TestResult {
    let upstream: Arc<dyn OpHost> = Arc::new(Upstream(Arc::clone(run)));
    run.record_methods();
    let ai = js_runner::run_script(
        &upstream,
        &json!({ "page": null }),
        &format!(
            "try {{\n\
               await page.goto({url})\n\
               return {{ ai: await page.ariaSnapshot({{ mode: 'ai' }}) }}\n\
             }} finally {{\n\
               await page.close()\n\
             }}"
        ),
    )
    .await?;
    let methods = run.recorded_methods();
    let ai = ai
        .get("ai")
        .and_then(Value::as_str)
        .ok_or("Playwright's ai text")?;
    for inside in [
        "Closed button",
        "Closed box",
        "Nested button",
        "Outer text",
        "Slot text",
    ] {
        assert!(
            !ai.contains(inside),
            "under the switch {inside} stays closed: {ai}"
        );
    }
    assert!(
        ai.contains("Slotted button"),
        "the host's own button is light content, which upstream reads: {ai}"
    );
    assert!(
        !methods
            .iter()
            .any(|method| method == "DOMSnapshot.captureSnapshot"),
        "under the switch no capture is made: {methods:?}"
    );
    Ok(())
}

/// The call-log lines a run's ops noted so far.
fn notes(run: &LiveRun) -> Vec<String> {
    run.calls()
        .into_iter()
        .filter_map(|call| match call {
            live_run::EnvCall::Note(line) => Some(line),
            _ => None,
        })
        .collect()
}

/// The text of the page at `url` snapshotted with `options`, by a run that opens and closes its tab.
async fn snapshot_of(host: &Arc<dyn OpHost>, url: &str, options: &str) -> TestResult<String> {
    let result = js_runner::run_script(
        host,
        &json!({ "page": null }),
        &format!(
            "try {{\n\
               await page.goto({url})\n\
               return await page.ariaSnapshot({options})\n\
             }} finally {{\n\
               await page.close()\n\
             }}",
            url = json!(url)
        ),
    )
    .await?;
    Ok(result.as_str().ok_or("the snapshot")?.to_owned())
}

/// A closed root that holds a list of 150 items, each a button in an `li`: one root, so one
/// seed, since a list, an item and a button cannot host a root of their own. Every item prints.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_large_closed_root_takes_one_seed_and_prints_whole() -> TestResult {
    let _alone = ALONE.acquire().await?;
    let html = "<!doctype html><html><body><main><div id=\"host\"></div></main><script>\n\
                const items = Array.from({ length: 150 }, (_, i) => `<li><button>Item ${i}</button></li>`).join('')\n\
                document.getElementById('host').attachShadow({ mode: 'closed' }).innerHTML = `<ul>${items}</ul>`\n\
                </script></body></html>";
    let server = FixtureServer::start(HashMap::from([(
        "/closed/list.html".to_owned(),
        Page::html(html).with_header("cache-control", "no-store"),
    )]))
    .await?;
    let run = Arc::new(LiveRun::connect().await?);
    let host: Arc<dyn OpHost> = Arc::clone(&run) as Arc<dyn OpHost>;
    let url = server.url("127.0.0.1", "/closed/list.html");
    let text = snapshot_of(&host, &url, "{ maxTokens: 100000 }").await?;
    for item in [0, 1, 75, 149] {
        let line = format!("- button \"Item {item}\" [ref=");
        assert!(text.contains(&line), "{line}\n{text}");
    }
    assert!(!text.contains("# partial"), "{text}");
    let notes = notes(&run);
    assert!(
        notes
            .iter()
            .any(|line| line.ends_with(", closed shadow root seeds: 1")),
        "one seed went: {notes:#?}"
    );
    Ok(())
}

/// `count` hosts, each with a closed root holding one button: a page built from many small
/// closed components needs one seed per component.
fn closed_cards(count: usize) -> String {
    format!(
        "<!doctype html><html><body><main></main><script>\n\
         const main = document.querySelector('main')\n\
         for (let i = 0; i < {count}; i++) {{\n\
           const card = main.appendChild(document.createElement('div'))\n\
           card.attachShadow({{ mode: 'closed' }}).innerHTML = `<button>Card ${{i}}</button>`\n\
         }}\n\
         </script></body></html>"
    )
}

/// More closed roots than a frame sends seeds for: the text says so on its first line, in the
/// default format and in `mode: 'full'`, and the op's call log names the cut; the roots that
/// were sent print. With fewer roots than the cap, no such line and every card prints.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_more_closed_roots_than_the_cap_say_so_in_the_text() -> TestResult {
    let _alone = ALONE.acquire().await?;
    let page = |html: String| Page::html(&html).with_header("cache-control", "no-store");
    let server = FixtureServer::start(HashMap::from([
        ("/closed/many.html".to_owned(), page(closed_cards(1_001))),
        ("/closed/few.html".to_owned(), page(closed_cards(40))),
    ]))
    .await?;
    let run = Arc::new(LiveRun::connect().await?);
    let host: Arc<dyn OpHost> = Arc::clone(&run) as Arc<dyn OpHost>;
    let many = server.url("127.0.0.1", "/closed/many.html");
    let line = "# partial: closed shadow roots not all read (1000 of 1001)";
    for options in ["{ text: false }", "{ mode: 'full' }"] {
        let text = snapshot_of(&host, &many, options).await?;
        assert_eq!(text.lines().next(), Some(line), "{options}: {text}");
        assert!(
            text.contains("- button \"Card 0\" [ref="),
            "{options}: {text}"
        );
    }
    let notes = notes(&run);
    assert!(
        notes.iter().any(
            |line| line == "  closed shadow roots: a frame has 1001 seeds, the first 1000 sent"
        ),
        "the cut is on the call log: {notes:#?}"
    );
    let few = server.url("127.0.0.1", "/closed/few.html");
    for options in ["{}", "{ mode: 'full' }"] {
        let text = snapshot_of(&host, &few, options).await?;
        assert!(!text.contains("# partial"), "{options}: {text}");
        for card in [0, 39] {
            let line = format!("- button \"Card {card}\" [ref=");
            assert!(text.contains(&line), "{options}: {line}\n{text}");
        }
    }
    Ok(())
}

/// Slots in closed roots, each shape in a host of its own: named slots that show the host's
/// children out of their order, a host child no slot takes, a root that assigns its slot by hand
/// (`slot.assign()`) and leaves a child out, and a slot inside a slot (a root that shows its host's
/// child through a slot placed in a nested root's host). `?mode=open` builds the same page with
/// open roots.
const SLOTS_PAGE: &str = r#"<!doctype html><html><head><title>Slots</title></head><body>
<main>
<div id="named"><button slot="b">Bee</button><button slot="a">Ay</button><button>Unslotted</button></div>
<div id="manual"><button id="one">One</button><button id="two">Two</button></div>
<div id="nested"><button>Deep</button></div>
</main>
<script>
const mode = new URLSearchParams(location.search).get('mode') ?? 'closed'
const root = (id, init) => document.getElementById(id).attachShadow({ mode, ...init })
root('named').innerHTML = '<p>Named</p><slot name="a"></slot><slot name="b"></slot>'
const manual = root('manual', { slotAssignment: 'manual' })
manual.innerHTML = '<p>Manual</p><slot></slot>'
manual.querySelector('slot').assign(document.getElementById('two'))
const outer = root('nested')
outer.innerHTML = '<p>Outer</p><div class="inner"><slot></slot></div>'
outer.querySelector('.inner').attachShadow({ mode }).innerHTML = '<p>Inner</p><slot></slot>'
</script>
</body></html>"#;

/// A closed root's slots show what an open root's would: on the same page, the default format and
/// `mode: 'full'` print the same text with closed roots as with open ones, refs aside, so the
/// host children no slot takes stay hidden and every slotted one prints once, where its slot is.
/// Under the conformance switch the closed page reads as upstream reads it, its roots closed:
/// none of their own text, and the hosts' drawn children in document order.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_closed_roots_slot_their_hosts_children_as_open_ones_do() -> TestResult {
    let _alone = ALONE.acquire().await?;
    let server = FixtureServer::start(HashMap::from([(
        "/closed/slots.html".to_owned(),
        Page::html(SLOTS_PAGE).with_header("cache-control", "no-store"),
    )]))
    .await?;
    let run = Arc::new(LiveRun::connect().await?);
    let host: Arc<dyn OpHost> = Arc::clone(&run) as Arc<dyn OpHost>;
    let closed = server.url("127.0.0.1", "/closed/slots.html");
    let open = format!("{closed}?mode=open");
    for options in ["{ text: true }", "{ mode: 'full' }"] {
        let closed_text = without_refs(&snapshot_of(&host, &closed, options).await?);
        let open_text = without_refs(&snapshot_of(&host, &open, options).await?);
        assert_eq!(closed_text, open_text, "{options}");
        for shown in [
            "Named", "Ay", "Bee", "Manual", "Two", "Outer", "Inner", "Deep",
        ] {
            assert_eq!(
                closed_text.matches(shown).count(),
                1,
                "{options}: {shown}\n{closed_text}"
            );
        }
        for hidden in ["Unslotted", "One"] {
            assert!(
                !closed_text.contains(hidden),
                "{options}: {hidden}\n{closed_text}"
            );
        }
        assert!(
            closed_text.find("\"Ay\"") < closed_text.find("\"Bee\""),
            "{options}: named slots show in their own order\n{closed_text}"
        );
    }
    let upstream: Arc<dyn OpHost> = Arc::new(Upstream(Arc::clone(&run)));
    let ai = snapshot_of(&upstream, &closed, "{ mode: 'ai' }").await?;
    for inside in ["Named", "Manual", "Outer", "Inner"] {
        assert!(
            !ai.contains(inside),
            "under the switch {inside} stays closed: {ai}"
        );
    }
    assert!(
        ai.find("\"Bee\"") < ai.find("\"Ay\""),
        "upstream reads a closed host's own drawn children in their own order, not the slots': {ai}"
    );
    Ok(())
}
