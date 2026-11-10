//! Child-frame waits on a real BrowserOS neo, through neo-quickjs-injected's public
//! `Frame.waitForLoadState`, `Frame.waitForURL` and `locator.waitFor` in frames: a same-origin
//! frame (`pay`) and a cross-site out-of-process frame (OOPIF, `cross`: `localhost` under a
//! `127.0.0.1` page). Each wait follows its own frame's document, not the page's; a navigation that
//! moves the frame to another process mid-wait (same-origin to cross-site and back) keeps the wait
//! going; a removed frame ends its wait with `Frame was detached` at once; a wait with no match
//! times out. Frames report their `DOMContentLoaded` to the top page with `postMessage`, so a test
//! can start a wait while a frame's own `load` is still pending (its image answers after
//! [`SLOW_RESPONSE`]).
//!
//! Runs only when `NEO_E2E_BROWSER` names the browser binary (`support/browser.rs`); otherwise
//! each test passes as a skip.
#![expect(
    clippy::disallowed_methods,
    reason = "the test drives an independent CDP session as the browser oracle"
)]

#[expect(
    dead_code,
    reason = "shared browser fixture includes helpers used by other suites"
)]
#[path = "support/browser.rs"]
mod browser;

use std::time::Duration;

use browser::{Fixture, Tab};
use neo_runtime::{RunOutcome, RunStatus};
use serde_json::{Value, json};
use tokio::time;

/// How long the fixture holds `/slow…` paths: a frame's image, and so its `load`.
const SLOW_RESPONSE: Duration = Duration::from_secs(3);

/// The top page, on `127.0.0.1`: a same-origin frame, a cross-site frame on `localhost` (whose
/// port only the page knows), buttons that remove each, and `window.__log`, what frames report.
const TOP: &str = r"<!doctype html><title>Top</title>
<h1>Top</h1>
<button id=remove-pay onclick=document.getElementById('pay').remove()>Remove pay</button>
<button id=remove-cross onclick=document.getElementById('cross').remove()>Remove cross</button>
<iframe id=pay name=pay src=/pay/step-1.html width=420 height=220></iframe>
<iframe id=cross name=cross width=420 height=220></iframe>
<script>
window.__log = [];
addEventListener('message', (event) => window.__log.push(event.data));
document.getElementById('cross').src = 'http://localhost:' + location.port + '/cross/step-1.html';
</script>";

/// Every frame page reports `<frame name>:dcl:<path>` once parsed and `<frame name>:load:<path>`
/// once loaded.
const REPORT: &str = "<script>
const report = (what) => top.postMessage(window.name + ':' + what, '*');
addEventListener('DOMContentLoaded', () => report('dcl:' + location.pathname));
addEventListener('load', () => report('load:' + location.pathname));
</script>";

/// A frame's first page: a same-document link, a next page, a link to the other site (which
/// moves the frame to another process), and text that appears shortly after a click.
fn step_1() -> String {
    format!(
        r#"<!doctype html><title>Step 1</title>{REPORT}
<a id=section href=#section>Section</a>
<a id=next href=step-2.html>Next</a>
<a id=away>Away</a>
<button id=reveal onclick="setTimeout(() => document.getElementById('done').hidden = false, 300)">Reveal</button>
<p id=done hidden>Done</p>
<script>
const other = location.hostname === 'localhost' ? '127.0.0.1' : 'localhost';
document.getElementById('away').href = 'http://' + other + ':' + location.port + '/' + window.name + '/away.html';
</script>"#
    )
}

fn page(title: &str, text: &str) -> String {
    format!("<!doctype html><title>{title}</title>{REPORT}<p>{text}</p>")
}

/// A page whose `load` waits for an image held [`SLOW_RESPONSE`]; `#state` says when it came.
fn slow_image() -> String {
    format!(
        r#"<!doctype html><title>Slow</title>{REPORT}
<p id=state>loading</p>
<img src=/slow-image.png onload="document.getElementById('state').textContent = 'image done'"
  onerror="document.getElementById('state').textContent = 'image done'">"#
    )
}

/// The fixture, once the top page and both frames have loaded.
async fn open() -> Option<Tab> {
    let binary = browser::e2e_browser()?;
    let mut pages = vec![("/top.html".to_string(), TOP.to_string())];
    for frame in ["pay", "cross"] {
        pages.extend([
            (format!("/{frame}/step-1.html"), step_1()),
            (format!("/{frame}/step-2.html"), page("Step 2", "Step 2")),
            (format!("/{frame}/away.html"), page("Away", "Away")),
            (format!("/{frame}/slow-image.html"), slow_image()),
        ]);
    }
    let tab = Tab::open(
        binary,
        Fixture {
            slow_response: SLOW_RESPONSE,
            pages,
        },
        "/top.html",
    )
    .await;
    // A page's `load` waits for its frames', so the top's means both frames are in.
    wait_for(&tab, "document.readyState === 'complete'").await;
    Some(tab)
}

/// Polls `condition` in the top page's main world for up to 20 s; a failure names what the
/// frames had reported.
async fn wait_for(tab: &Tab, condition: &str) {
    let held = time::timeout(Duration::from_secs(20), async {
        while tab.evaluate(condition).await != json!(true) {
            time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await;
    if held.is_err() {
        let reports = tab.evaluate("JSON.stringify(window.__log ?? null)").await;
        panic!("{condition} never held; the frames reported {reports}");
    }
}

/// Points both frames at their slow page and returns once each has parsed it, while its `load`
/// still waits for the image.
async fn load_slowly(tab: &Tab) {
    tab.evaluate(
        "document.getElementById('pay').src = '/pay/slow-image.html';
         document.getElementById('cross').src =
           'http://localhost:' + location.port + '/cross/slow-image.html';",
    )
    .await;
    wait_for(
        tab,
        "window.__log.includes('pay:dcl:/pay/slow-image.html')
         && window.__log.includes('cross:dcl:/cross/slow-image.html')",
    )
    .await;
}

/// Runs `code` with a deadline well under any wait's own timeout, so a wait that misses its
/// frame's end shows up as a run that timed out.
async fn run(tab: &Tab, code: &str) -> RunOutcome {
    tab.start_run_within(format!("{PRELUDE}\n{code}"), Duration::from_secs(20))
        .await
        .await
        .unwrap_or_else(|error| panic!("the run's task: {error}"))
}

/// What every script starts with: a frame by name, and an error as a kind.
const PRELUDE: &str = "
const frame = (name) => page.frames().find((frame) => frame.name() === name);
const caught = (error) => ({
  failed: error.name,
  detached: /Frame was detached/.test(error.message),
});";

/// The script's return value, after checking the run itself went well.
fn value(outcome: &RunOutcome) -> Value {
    assert_eq!(outcome.status, RunStatus::Ok, "{outcome:#?}");
    outcome
        .value
        .clone()
        .unwrap_or_else(|| panic!("the script returns a value: {outcome:#?}"))
}

/// The CDP frame id of the iframe `selector` names in the top page.
async fn frame_id(tab: &Tab, selector: &str) -> String {
    let element = tab
        .cdp
        .send(
            "Runtime.evaluate",
            json!({ "expression": format!("document.querySelector('{selector}')") }),
            Some(&tab.driver),
        )
        .await
        .unwrap_or_else(|error| panic!("find {selector}: {error}"));
    let node = tab
        .cdp
        .send(
            "DOM.describeNode",
            json!({ "objectId": element["result"]["objectId"] }),
            Some(&tab.driver),
        )
        .await
        .unwrap_or_else(|error| panic!("describe {selector}: {error}"));
    node["node"]["frameId"]
        .as_str()
        .unwrap_or_else(|| panic!("{selector} has no frame: {node}"))
        .to_string()
}

/// Whether `frame` is an `iframe` target of its own: an out-of-process frame.
async fn is_oopif(tab: &Tab, frame: &str) -> bool {
    let targets = tab
        .cdp
        .send("Target.getTargets", json!({ "filter": [{}] }), None)
        .await
        .unwrap_or_else(|error| panic!("targets: {error}"));
    targets["targetInfos"].as_array().is_some_and(|targets| {
        targets
            .iter()
            .any(|target| target["type"] == "iframe" && target["targetId"] == frame)
    })
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_load_state_waits_follow_each_frames_own_document() {
    let Some(tab) = open().await else { return };
    load_slowly(&tab).await;
    let outcome = run(
        &tab,
        "const state = (name) => page.frameLocator('#' + name).locator('#state').textContent();
         const before = { pay: await state('pay'), cross: await state('cross') };
         const after = {};
         for (const name of ['pay', 'cross']) {
           await frame(name).waitForLoadState('commit');
           await frame(name).waitForLoadState('domcontentloaded');
           await frame(name).waitForLoadState();
           after[name] = await state(name);
         }
         return { before, after };",
    )
    .await;
    let result = value(&outcome);
    tab.close().await;
    assert_eq!(
        result["before"],
        json!({ "pay": "loading", "cross": "loading" }),
        "both frames were still loading when the waits began: {result}"
    );
    assert_eq!(
        result["after"],
        json!({ "pay": "image done", "cross": "image done" }),
        "each load wait ended on its own frame's load, not the page's: {result}"
    );
}

/// A reload keeps the frame and its OOPIF target, and starts a new document in the same session:
/// the wait must take that document's `load`, not the previous one's.
#[tokio::test(flavor = "multi_thread")]
async fn tests_that_load_state_waits_follow_a_reloaded_oopif() {
    let Some(tab) = open().await else { return };
    load_slowly(&tab).await;
    let reports =
        "window.__log.filter((report) => report === 'cross:%:/cross/slow-image.html').length";
    wait_for(&tab, &format!("{} === 1", reports.replace('%', "load"))).await;
    let cross = frame_id(&tab, "#cross").await;
    tab.evaluate("const cross = document.getElementById('cross'); cross.src = cross.src")
        .await;
    wait_for(&tab, &format!("{} === 2", reports.replace('%', "dcl"))).await;
    let outcome = run(
        &tab,
        "const state = () => page.frameLocator('#cross').locator('#state').textContent();
         const before = await state();
         await frame('cross').waitForLoadState();
         return { before, after: await state() };",
    )
    .await;
    let result = value(&outcome);
    let still_oopif = is_oopif(&tab, &cross).await;
    tab.close().await;
    assert!(still_oopif, "the reload kept cross in its own process");
    assert_eq!(
        result,
        json!({ "before": "loading", "after": "image done" }),
        "the wait ended on the reloaded document's load"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_wait_for_url_follows_each_frames_navigations() {
    let Some(tab) = open().await else { return };
    let outcome = run(
        &tab,
        "const result = {};
         for (const name of ['pay', 'cross']) {
           const inFrame = page.frameLocator('#' + name);
           await Promise.all([
             frame(name).waitForURL('**/' + name + '/step-1.html#section'),
             inFrame.locator('#section').click(),
           ]);
           await Promise.all([
             frame(name).waitForURL('**/' + name + '/step-2.html'),
             inFrame.locator('#next').click(),
           ]);
           // Already there: straight to its load state.
           await frame(name).waitForURL('**/step-2.html', { waitUntil: 'domcontentloaded' });
           result[name] = {
             url: frame(name).url(),
             text: await inFrame.locator('p').textContent(),
           };
         }
         result.page = page.url();
         return result;",
    )
    .await;
    let result = value(&outcome);
    let origin = tab.origin.clone();
    tab.close().await;
    let port = origin.rsplit(':').next().unwrap_or_default();
    assert_eq!(
        result,
        json!({
            "pay": { "url": format!("{origin}/pay/step-2.html"), "text": "Step 2" },
            "cross": { "url": format!("http://localhost:{port}/cross/step-2.html"), "text": "Step 2" },
            "page": format!("{origin}/top.html"),
        }),
        "each frame navigated, the page did not"
    );
}

/// The frame's document moves to another process while the wait is pending: `pay` from the
/// page's process to its own (cross-site), `cross` from its own back into the page's.
#[tokio::test(flavor = "multi_thread")]
async fn tests_that_a_frame_wait_keeps_going_when_its_document_changes_process() {
    let Some(tab) = open().await else { return };
    let pay = frame_id(&tab, "#pay").await;
    let cross = frame_id(&tab, "#cross").await;
    assert_eq!(
        (is_oopif(&tab, &pay).await, is_oopif(&tab, &cross).await),
        (false, true),
        "pay starts in the page's process, cross in its own"
    );
    let outcome = run(
        &tab,
        "const result = {};
         for (const name of ['pay', 'cross']) {
           await Promise.all([
             frame(name).waitForURL('**/' + name + '/away.html'),
             page.frameLocator('#' + name).locator('#away').click(),
           ]);
           await frame(name).waitForLoadState();
           result[name] = {
             url: frame(name).url(),
             text: await page.frameLocator('#' + name).locator('p').textContent(),
           };
         }
         return result;",
    )
    .await;
    let result = value(&outcome);
    let swapped = (is_oopif(&tab, &pay).await, is_oopif(&tab, &cross).await);
    let origin = tab.origin.clone();
    tab.close().await;
    let port = origin.rsplit(':').next().unwrap_or_default();
    assert_eq!(
        swapped,
        (true, false),
        "both frames changed process: pay out of the page's, cross into it"
    );
    assert_eq!(
        result,
        json!({
            "pay": { "url": format!("http://localhost:{port}/pay/away.html"), "text": "Away" },
            "cross": { "url": format!("{origin}/cross/away.html"), "text": "Away" },
        })
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_removing_a_frame_ends_its_waits_at_once() {
    let Some(tab) = open().await else { return };
    load_slowly(&tab).await;
    // Each wait's own timeout is past the run's 20 s deadline: only the removal can end it.
    let outcome = run(
        &tab,
        "const pay = frame('pay');
         const cross = frame('cross');
         const loading = pay.waitForLoadState('load', { timeout: 60000 }).then(() => 'loaded', caught);
         const matching = cross.waitForURL('**/never', { timeout: 60000 }).then(() => 'matched', caught);
         await page.click('#remove-pay');
         await page.click('#remove-cross');
         return {
           loading: await loading,
           matching: await matching,
           detached: [pay.isDetached(), cross.isDetached()],
           afterwards: await pay.waitForURL('**', { timeout: 60000 }).then(() => 'matched', caught),
         };",
    )
    .await;
    let result = value(&outcome);
    tab.close().await;
    let gone = json!({ "failed": "Error", "detached": true });
    assert_eq!(
        result,
        json!({
            "loading": gone,
            "matching": gone,
            "detached": [true, true],
            "afterwards": gone,
        }),
        "a load wait in the page's process and a URL wait in an OOPIF both end with the frame"
    );
}

/// The page navigating away takes every frame of its document with it.
#[tokio::test(flavor = "multi_thread")]
async fn tests_that_a_page_navigation_ends_its_frames_waits() {
    let Some(tab) = open().await else { return };
    let outcome = run(
        &tab,
        "const pending = ['pay', 'cross'].map((name) =>
           frame(name).waitForURL('**/never', { timeout: 60000 }).then(() => 'matched', caught));
         await page.goto(page.url().replace('/top.html', '/elsewhere'));
         return await Promise.all(pending);",
    )
    .await;
    let result = value(&outcome);
    tab.close().await;
    let gone = json!({ "failed": "Error", "detached": true });
    assert_eq!(result, json!([gone, gone]));
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_a_frame_wait_without_a_match_times_out() {
    let Some(tab) = open().await else { return };
    let outcome = run(
        &tab,
        "return {
           pay: await frame('pay').waitForURL('**/never', { timeout: 800 }).then(() => 'matched', caught),
           cross: await frame('cross').waitForURL('**/never', { timeout: 800 }).then(() => 'matched', caught),
         };",
    )
    .await;
    let result = value(&outcome);
    tab.close().await;
    let timed_out = json!({ "failed": "TimeoutError", "detached": false });
    assert_eq!(result, json!({ "pay": timed_out, "cross": timed_out }));
}

/// `locator.waitFor` through a frame selector (nf-core's routing), in both frames, and through
/// an iframe that does not exist: `hidden` and `detached` hold at once, as in Playwright.
#[tokio::test(flavor = "multi_thread")]
async fn tests_that_locator_wait_for_reaches_inside_frames() {
    let Some(tab) = open().await else { return };
    let outcome = run(
        &tab,
        "const result = {};
         for (const name of ['pay', 'cross']) {
           const inFrame = page.frameLocator('#' + name);
           await inFrame.locator('#reveal').click();
           await inFrame.getByText('Done').waitFor();
           await inFrame.locator('#done').waitFor({ state: 'attached' });
           await inFrame.locator('#nothing').waitFor({ state: 'detached' });
           await inFrame.locator('#nothing').waitFor({ state: 'hidden' });
           result[name] = await inFrame.locator('#done').isVisible();
         }
         const missing = page.frameLocator('#missing').locator('p');
         result.missing = {
           hidden: await missing.waitFor({ state: 'hidden' }).then(() => 'ok', caught),
           detached: await missing.waitFor({ state: 'detached' }).then(() => 'ok', caught),
           visible: await missing.waitFor({ state: 'visible', timeout: 500 }).then(() => 'ok', caught),
           attached: await missing.waitFor({ state: 'attached', timeout: 500 }).then(() => 'ok', caught),
         };
         return result;",
    )
    .await;
    let result = value(&outcome);
    tab.close().await;
    let timed_out = json!({ "failed": "TimeoutError", "detached": false });
    assert_eq!(
        result,
        json!({
            "pay": true,
            "cross": true,
            "missing": { "hidden": "ok", "detached": "ok", "visible": timed_out, "attached": timed_out },
        })
    );
}
