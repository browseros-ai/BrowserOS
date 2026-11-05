//! Runner N (R13): Playwright scripts as agents write them, run by the real prelude in QuickJS
//! (`tests/support/js_runner.rs`), with every op going through `Playwright::invoke`. This is the
//! end-to-end check of the wire contract: wire-examples.json proves the inputs decode, and these
//! scripts prove the outputs read back as Playwright's values: navigation responses, reads, both
//! snapshot modes and refs from them, the frame tree, popups through a wait and through the
//! event pump, and page handles that keep their identity.
//!
//! The offline test needs no browser. The live test needs a throwaway BrowserOS with CDP on
//! `BROWSEROS_CDP_PORT` (default 9005): `cargo test -p browseros-playwright --test prelude_live
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
use std::sync::Arc;

use browseros_playwright::{ErrorCode, PlaywrightError};
use fixture_server::{FixtureServer, Page};
use futures_util::future::BoxFuture;
use js_runner::{OpHost, run_script};
use live_run::{LiveRun, TestPage, TestResult};
use serde_json::{Value, json};

/// The goldens recorded from playwright-core 1.64.1: each expression's selector and `toString()`.
const SELECTOR_GOLDENS: &str =
    include_str!("../../../packages/browseros-playwright-js/tests/data/selector-goldens.json");

/// A host for scripts that must not reach Rust: builders are values and send nothing.
struct NoOps;

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
    let init = json!({ "page": { "$neo": "page", "id": 7, "tab": 1007 } });
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
    let result = run_script(&host, &init, &script).await?;
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
    let init = json!({ "page": { "$neo": "page", "id": 7, "tab": 1007 } });
    let script = "await expect(page.locator('#total')).toHaveText('nope', { timeout: 50 })";
    let settled = run_script(&host, &init, script).await;
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

#[tokio::test]
async fn tests_that_context_pages_lists_the_tabs_the_run_init_names() -> TestResult {
    let host: Arc<dyn OpHost> = Arc::new(NoOps);
    let init = json!({
        "page": { "$neo": "page", "id": 7, "tab": 1007 },
        "pages": [
            { "id": 7, "tab": 1007, "url": "https://shop.example/cart", "title": "Cart", "closed": false },
            { "id": 9, "tab": 1009, "url": "https://mail.example/", "title": "Mail", "closed": false },
        ],
    });
    let script =
        "return context.pages().map((p) => ({ url: p.url(), brand: p.toJSON(), run: p === page }))";
    let listed = run_script(&host, &init, script).await?;
    assert_eq!(
        listed,
        json!([
            { "url": "https://shop.example/cart", "brand": { "$neo": "page", "id": 7, "tab": 1007 }, "run": true },
            { "url": "https://mail.example/", "brand": { "$neo": "page", "id": 9, "tab": 1009 }, "run": false },
        ]),
        "every tab the init lists, with no op sent"
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
           const snapshot = await page.ariaSnapshot()\n\
           const aiSnapshot = await page.ariaSnapshot({{ mode: 'ai' }})\n\
           const refs = [...aiSnapshot.matchAll(/\\[ref=([^\\]]+)\\]/g)].map((match) => match[1])\n\
           const byRef = [await page.getByRef(refs[1]).textContent(), await page.getByRef(refs[8]).textContent()]\n\
           const frames = page.frames().map((frame) => ({{\n\
             name: frame.name(), url: frame.url(), parent: frame.parentFrame()?.name() ?? null,\n\
           }}))\n\
           const fromPage = new Promise((resolve) => page.once('popup', resolve))\n\
           const fromContext = new Promise((resolve) => context.once('page', resolve))\n\
           const opened = page.waitForEvent('popup')\n\
           const failed = await __neo_test_gesture(page._id, page._tab, \"void window.open('/prelude/popup.html')\")\n\
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
             go, items, snapshot, ai: aiSnapshot.replace(/\\[ref=[^\\]]+\\]/g, '[ref]'), byRef, frames, popupUrl, pumped, opener, listed, popupClosed: popup.isClosed(),\n\
             after: page.frames().map((frame) => frame.url()), url: page.url(),\n\
           }}\n\
         }} finally {{\n\
           await page.close()\n\
         }}",
        page = json!(url("page")),
        side = json!(url("side")),
    );
    let result = run_script(&host, &init, &script).await?;
    assert_eq!(
        result,
        json!({
            "status": 200, "ok": true, "statusText": "OK",
            "go": "Go",
            "items": 3,
            "snapshot": "- button \"Go\"\n- button \"Stop\"\n- list:\n  - listitem: One\n  \
                         - listitem: Two\n  - listitem: Three\n- iframe",
            // Refs differ per process, so they are compared as `[ref]`; two of them, one in the
            // iframe, are read back through getByRef.
            "ai": "- generic [active] [ref]:\n  - button \"Go\" [ref]\n  - button \"Stop\" [ref]\n  \
                   - list [ref]:\n    - listitem [ref]: One\n    - listitem [ref]: Two\n    \
                   - listitem [ref]: Three\n  - iframe [ref]:\n    - paragraph [ref]: side",
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
