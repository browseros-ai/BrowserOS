//! Session state on a real BrowserOS neo (`src/browser_representations/cookies_and_storage.rs`): what a run sets on its
//! sessions (extra HTTP headers, offline mode, geolocation) ends with it, so the next run on the
//! same tab finds none of it; a cookie it adds is browser data and stays; a permission it grants
//! is reset once it ends, back to what the profile had before, and one run's clearing, even
//! cancelled halfway, never takes another run's grants.
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
use neo_runtime::{PageId, RunOutcome, RunRequest, RunStatus, Runtime, StartPage};
use serde_json::{Value, json};
use tokio::task::JoinHandle;
use tokio::time;
use tokio_util::sync::CancellationToken;

const PAGE: &str =
    "<!doctype html><html><head><title>state</title></head><body>state</body></html>";

async fn open() -> Option<Tab> {
    let binary = browser::e2e_browser()?;
    Some(
        Tab::open(
            binary,
            Fixture {
                pages: vec![("/state.html".into(), PAGE.into())],
                ..Fixture::default()
            },
            "/state.html",
        )
        .await,
    )
}

fn request(tab: &Tab, code: &str) -> RunRequest {
    cancellable(tab, code, &CancellationToken::new())
}

/// A run of `code` on the tab that `cancel` ends.
fn cancellable(tab: &Tab, code: &str, cancel: &CancellationToken) -> RunRequest {
    RunRequest {
        code: code.to_string(),
        page: StartPage::Page(PageId::new(tab.target_id.clone())),
        timeout: Duration::from_secs(30),
        cancel: cancel.clone(),
        artifacts: None,
    }
}

fn value(outcome: &RunOutcome) -> &Value {
    assert_eq!(outcome.status, RunStatus::Ok, "{outcome:#?}");
    outcome.value.as_ref().unwrap_or(&Value::Null)
}

/// What `expression` settles to on the page, read over the test's own session.
async fn settled(tab: &Tab, expression: &str) -> Value {
    let reply = tab
        .cdp
        .send(
            "Runtime.evaluate",
            json!({ "expression": expression, "awaitPromise": true, "returnByValue": true }),
            Some(&tab.driver),
        )
        .await
        .unwrap_or_else(|error| panic!("evaluate {expression}: {error}"));
    assert!(reply.get("exceptionDetails").is_none(), "{reply}");
    reply["result"]["value"].clone()
}

/// The page's geolocation permission state, read over the test's own session.
async fn geolocation_state(tab: &Tab) -> Value {
    settled(
        tab,
        "navigator.permissions.query({ name: 'geolocation' }).then((status) => status.state)",
    )
    .await
}

/// Waits, a bounded while, until `expression` is true on the page.
async fn until(tab: &Tab, expression: &str) {
    time::timeout(Duration::from_secs(15), async {
        while tab.evaluate(expression).await != json!(true) {
            time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("the page never reached {expression}"));
}

/// A script's read of where the page is, `"error"` when it learns nothing in a second.
const POSITION: &str = r"page.evaluate(() => new Promise((resolve) => navigator.geolocation.getCurrentPosition(
  (position) => resolve([position.coords.latitude, position.coords.longitude]),
  () => resolve('error'),
  { timeout: 1000 },
)))";

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_session_state_ends_with_the_run_and_cookies_stay() {
    let Some(tab) = open().await else {
        return;
    };
    let runtime = Runtime::connect(tab.cdp.clone())
        .await
        .unwrap_or_else(|error| panic!("Runtime::connect: {error}"));
    let first = runtime
        .run(request(
            &tab,
            &format!(
                r"await context.setExtraHTTPHeaders({{ 'x-neo-run': 'one' }})
const echoed = await page.evaluate(() => fetch('/echo-headers').then((response) => response.text()))
await context.addCookies([{{ name: 'neo_run', value: 'one', url: page.url() }}])
await context.grantPermissions(['geolocation'], {{ origin: page.url() }})
await context.setGeolocation({{ latitude: 10, longitude: 20 }})
const position = await {POSITION}
await context.setOffline(true)
const network = await page.evaluate(() => fetch('/again').then(() => 'online', () => 'offline'))
return [echoed.toLowerCase().includes('x-neo-run: one'), position, network]"
            ),
        ))
        .await;
    assert_eq!(
        value(&first),
        &json!([true, [10, 20], "offline"]),
        "the run's headers, geolocation and offline mode reach its tab"
    );

    let second = runtime
        .run(request(
            &tab,
            &format!(
                r"const echoed = await page.evaluate(() => fetch('/echo-headers').then((response) => response.text()))
const network = await page.evaluate(() => fetch('/again').then(() => 'online', () => 'offline'))
await context.grantPermissions(['geolocation'], {{ origin: page.url() }})
const position = await {POSITION}
const cookies = (await context.cookies(page.url())).filter((cookie) => cookie.name === 'neo_run')
await context.clearCookies({{ name: 'neo_run' }})
const cleared = (await context.cookies(page.url())).filter((cookie) => cookie.name === 'neo_run').length
return [echoed.toLowerCase().includes('x-neo-run'), network, position, cookies.map((cookie) => cookie.value), cleared]"
            ),
        ))
        .await;
    let second = value(&second);
    assert_eq!(
        second.get(0),
        Some(&json!(false)),
        "no header of the first run"
    );
    assert_eq!(second.get(1), Some(&json!("online")), "not offline");
    assert_ne!(
        second.get(2),
        Some(&json!([10, 20])),
        "not the first run's position"
    );
    assert_eq!(
        second.get(3),
        Some(&json!(["one"])),
        "the first run's cookie stayed"
    );
    assert_eq!(
        second.get(4),
        Some(&json!(0)),
        "and a filtered clear removed it"
    );
    tab.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_a_runs_permission_grants_are_reset_when_it_ends() {
    let Some(tab) = open().await else {
        return;
    };
    let before = geolocation_state(&tab).await;
    let runtime = Runtime::connect(tab.cdp.clone())
        .await
        .unwrap_or_else(|error| panic!("Runtime::connect: {error}"));
    let outcome = runtime
        .run(request(
            &tab,
            r"await context.grantPermissions(['geolocation'], { origin: page.url() })
return await page.evaluate(() => navigator.permissions.query({ name: 'geolocation' }).then((status) => status.state))",
        ))
        .await;
    assert_eq!(
        value(&outcome),
        &json!("granted"),
        "granted while the run holds it"
    );
    assert_ne!(
        before,
        json!("granted"),
        "a fresh profile has not granted it"
    );
    assert_eq!(
        geolocation_state(&tab).await,
        before,
        "the profile's own state once the run ended"
    );
    tab.close().await;
}

/// Review C1 P2: run B's `clearPermissions` resets the browser's overrides, then grants the
/// other runs' again. B cancelled right after the reset must still finish that, so run A, still
/// going, keeps its grant. A grants many origins first, so its grants take a while to come
/// back; the page's own origin sorts after them.
#[tokio::test(flavor = "multi_thread")]
async fn tests_that_cancelling_a_runs_clear_keeps_another_live_runs_grant() {
    let Some(tab) = open().await else {
        return;
    };
    let runtime = Runtime::connect(tab.cdp.clone())
        .await
        .unwrap_or_else(|error| panic!("Runtime::connect: {error}"));
    let first_cancel = CancellationToken::new();
    let first = {
        let runtime = runtime.clone();
        let request = cancellable(
            &tab,
            r"for (let i = 0; i < 80; i++) {
  await context.grantPermissions(['geolocation'], { origin: 'http://0-a' + String(i).padStart(3, '0') + '.localhost' })
}
await context.grantPermissions(['geolocation'], { origin: page.url() })
await page.evaluate(() => { window.firstReady = true })
await new Promise(() => {})",
            &first_cancel,
        );
        tokio::spawn(async move { runtime.run(request).await })
    };
    until(&tab, "window.firstReady === true").await;
    for (method, params) in [
        ("Runtime.enable", json!({})),
        ("Runtime.addBinding", json!({ "name": "permissionReset" })),
    ] {
        tab.cdp
            .send(method, params, Some(&tab.driver))
            .await
            .unwrap_or_else(|error| panic!("{method}: {error}"));
    }
    let mut events = tab.cdp.events();
    assert_eq!(
        settled(
            &tab,
            r"navigator.permissions.query({ name: 'geolocation' }).then((status) => {
  window.watched = status
  status.onchange = () => { if (status.state !== 'granted') permissionReset(status.state) }
  return status.state
})"
        )
        .await,
        json!("granted"),
        "run A's grant, before B clears"
    );
    let second_cancel = CancellationToken::new();
    let second = {
        let runtime = runtime.clone();
        let request = cancellable(
            &tab,
            r"await context.grantPermissions(['notifications'], { origin: page.url() })
await context.clearPermissions()
return 'cleared'",
            &second_cancel,
        );
        tokio::spawn(async move { runtime.run(request).await })
    };
    time::timeout(Duration::from_secs(15), async {
        loop {
            let event = events
                .recv()
                .await
                .unwrap_or_else(|error| panic!("events: {error}"));
            if event.method == "Runtime.bindingCalled" && event.params["name"] == "permissionReset"
            {
                second_cancel.cancel();
                return;
            }
        }
    })
    .await
    .unwrap_or_else(|_| panic!("the page never saw B's reset"));
    let second = second
        .await
        .unwrap_or_else(|error| panic!("run B: {error}"));
    let after = geolocation_state(&tab).await;
    first_cancel.cancel();
    let first = first.await.unwrap_or_else(|error| panic!("run A: {error}"));
    runtime.drain().await;
    tab.close().await;
    assert_eq!(
        second.status,
        RunStatus::Cancelled,
        "B's clear was cut short"
    );
    assert_eq!(first.status, RunStatus::Cancelled, "{first:#?}");
    assert_eq!(
        after,
        json!("granted"),
        "run A, still going, keeps its grant"
    );
}

/// The tab's geolocation, read over the test's own session; an error code when there is none.
const POSITION_NOW: &str = "new Promise((resolve) => navigator.geolocation.getCurrentPosition((p) => resolve([p.coords.latitude, p.coords.longitude]), (e) => resolve({ error: e.code }), { timeout: 1500 }))";

/// The tab's geolocation and notifications permissions, as the test's own session reads them.
const PERMISSIONS: &str = "Promise.all(['geolocation', 'notifications'].map((name) => navigator.permissions.query({ name }).then((status) => status.state)))";

/// Starts `code` as a run of its own on the tab, ended by the returned token.
fn spawn_run(
    runtime: &Runtime,
    tab: &Tab,
    code: &str,
) -> (JoinHandle<RunOutcome>, CancellationToken) {
    let cancel = CancellationToken::new();
    let runtime = runtime.clone();
    let request = cancellable(tab, code, &cancel);
    (
        tokio::spawn(async move { runtime.run(request).await }),
        cancel,
    )
}

/// Waits, a bounded while, until `expression` settles to `expected` on the page.
async fn until_settled(tab: &Tab, expression: &str, expected: &Value) {
    let mut last = Value::Null;
    let reached = time::timeout(Duration::from_secs(15), async {
        loop {
            last = settled(tab, expression).await;
            if &last == expected {
                return;
            }
            time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await;
    assert!(
        reached.is_ok(),
        "the page read {last}, never {expected}, from {expression}"
    );
}

/// Review D1 P2: run B's `context.close()` takes back only B's own on a tab run A, still going,
/// also set: A keeps its init script, its header, its permission grant and its geolocation,
/// though the browser keeps one geolocation per tab and one session's clear ends another's
/// (measured).
#[tokio::test(flavor = "multi_thread")]
async fn tests_that_closing_one_run_leaves_another_runs_state_on_their_tab() {
    let Some(tab) = open().await else {
        return;
    };
    let runtime = Runtime::connect(tab.cdp.clone())
        .await
        .unwrap_or_else(|error| panic!("Runtime::connect: {error}"));
    let (a, a_cancel) = spawn_run(
        &runtime,
        &tab,
        r"await context.addInitScript(() => { window.scriptA = true })
await context.setExtraHTTPHeaders({ 'x-run-a': 'A' })
await context.grantPermissions(['geolocation'], { origin: page.url() })
await context.setGeolocation({ latitude: 10, longitude: 20 })
await page.evaluate(() => { window.readyA = true })
await new Promise(() => {})",
    );
    until(&tab, "window.readyA === true").await;
    let (b, b_cancel) = spawn_run(
        &runtime,
        &tab,
        r"await context.addInitScript(() => { window.scriptB = true })
await context.setExtraHTTPHeaders({ 'x-run-b': 'B' })
await context.grantPermissions(['notifications'], { origin: page.url() })
await context.setGeolocation({ latitude: 30, longitude: 40 })
await page.evaluate(() => new Promise((resolve) => { window.closeB = resolve }))
await context.close()
await new Promise((resolve) => setTimeout(resolve, 15000))",
    );
    until(&tab, "typeof window.closeB === 'function'").await;
    assert_eq!(
        settled(&tab, POSITION_NOW).await,
        json!([30, 40]),
        "B's position, the last set"
    );
    settled(&tab, "window.closeB()").await;
    // B's notifications grant goes last in its close: once it is gone, the close let go.
    until_settled(&tab, PERMISSIONS, &json!(["granted", "denied"])).await;
    assert!(!b.is_finished(), "B's realm lives on after its close");
    let position = settled(&tab, POSITION_NOW).await;
    let headers = settled(
        &tab,
        "fetch('/echo-headers').then((response) => response.text())",
    )
    .await;
    tab.navigate("/state.html").await;
    until(&tab, "document.readyState === 'complete'").await;
    let scripts = settled(
        &tab,
        "({ a: window.scriptA === true, b: window.scriptB === true })",
    )
    .await;
    b_cancel.cancel();
    let b = b.await.unwrap_or_else(|error| panic!("run B: {error}"));
    a_cancel.cancel();
    let a = a.await.unwrap_or_else(|error| panic!("run A: {error}"));
    runtime.drain().await;
    tab.close().await;
    assert_eq!(
        (a.status, b.status),
        (RunStatus::Cancelled, RunStatus::Cancelled)
    );
    assert_eq!(
        position,
        json!([10, 20]),
        "A's position is back once B let go of its own"
    );
    let headers = headers.as_str().unwrap_or_default().to_lowercase();
    assert!(
        headers.contains("x-run-a: a"),
        "A's header stays: {headers}"
    );
    assert!(!headers.contains("x-run-b"), "B's header goes: {headers}");
    assert_eq!(
        scripts,
        json!({ "a": true, "b": false }),
        "A's init script stays, B's goes"
    );
}

/// Review D1 P2, offline: the browser keeps one offline state per tab, one session's reset ends
/// another's, and a session that set it once ends the page's `navigator.onLine` override when it
/// detaches (measured). Run A, still going, stays offline after run B's close and after B's
/// sessions detach at its end.
#[tokio::test(flavor = "multi_thread")]
async fn tests_that_closing_one_run_leaves_another_runs_offline_mode() {
    let Some(tab) = open().await else {
        return;
    };
    let runtime = Runtime::connect(tab.cdp.clone())
        .await
        .unwrap_or_else(|error| panic!("Runtime::connect: {error}"));
    let (a, a_cancel) = spawn_run(
        &runtime,
        &tab,
        r"await context.setOffline(true)
await page.evaluate(() => { window.readyA = true })
await new Promise(() => {})",
    );
    until(&tab, "window.readyA === true").await;
    let (b, b_cancel) = spawn_run(
        &runtime,
        &tab,
        r"await context.setOffline(true)
await context.grantPermissions(['notifications'], { origin: page.url() })
await page.evaluate(() => new Promise((resolve) => { window.closeB = resolve }))
await context.close()
await new Promise((resolve) => setTimeout(resolve, 15000))",
    );
    until(&tab, "typeof window.closeB === 'function'").await;
    let notifications =
        "navigator.permissions.query({ name: 'notifications' }).then((status) => status.state)";
    assert_eq!(
        settled(&tab, notifications).await,
        json!("granted"),
        "B's grant, before its close"
    );
    settled(&tab, "window.closeB()").await;
    // B's notifications grant goes last in its close: once it is gone, the close let go.
    until_settled(
        &tab,
        &format!("{notifications}.then((state) => state !== 'granted')"),
        &json!(true),
    )
    .await;
    let offline = "(async () => [navigator.onLine, await fetch('/again').then(() => 'fetched', () => 'failed')])()";
    let after_close = settled(&tab, offline).await;
    let mut events = tab.cdp.events();
    b_cancel.cancel();
    let b = b.await.unwrap_or_else(|error| panic!("run B: {error}"));
    time::timeout(Duration::from_secs(15), async {
        loop {
            let event = events
                .recv()
                .await
                .unwrap_or_else(|error| panic!("events: {error}"));
            if event.method == "Target.detachedFromTarget" {
                return;
            }
        }
    })
    .await
    .unwrap_or_else(|_| panic!("B's session never detached"));
    // Neo puts A's state back once it hears of the detach: read until it did, a bounded while.
    let expected = json!([false, "failed"]);
    let mut after_detach = settled(&tab, offline).await;
    let _back = time::timeout(Duration::from_secs(5), async {
        while after_detach != expected {
            time::sleep(Duration::from_millis(20)).await;
            after_detach = settled(&tab, offline).await;
        }
    })
    .await;
    a_cancel.cancel();
    let a = a.await.unwrap_or_else(|error| panic!("run A: {error}"));
    runtime.drain().await;
    tab.close().await;
    assert_eq!(
        (a.status, b.status),
        (RunStatus::Cancelled, RunStatus::Cancelled)
    );
    assert_eq!(
        after_close,
        json!([false, "failed"]),
        "A is offline after B's close"
    );
    assert_eq!(
        after_detach,
        json!([false, "failed"]),
        "and after B's session detached"
    );
}
