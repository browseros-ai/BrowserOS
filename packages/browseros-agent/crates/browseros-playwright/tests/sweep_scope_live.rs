//! The main-world sweep's scope on a real browser, through `Playwright::invoke` (decisions.md,
//! "Main-world sweep scope"): before and after `page.evaluate` and `page.waitForFunction`, every
//! frame of the page in the main frame's site is swept, across every CDP session the page has.
//! A frame of that site that cannot be swept within its budget refuses the script with nothing
//! sent; a frame of another site is never asked, so a hung one costs nothing.
//!
//! The nested fixture holds the leak R10c measured: a3 has the top page's site but sits under two
//! cross-site OOPIFs on a session of its own, so a script on top reads it through `frames[…]`.
//! Two more hide such a frame where no count in the page sees it: under a same-process frame, and
//! under a cross-site frame in a closed shadow root. The last test prints what a sweep costs; run
//! it with `--nocapture`.
//!
//! Needs a throwaway BrowserOS with CDP on `BROWSEROS_CDP_PORT` (default 9005), for example
//! `PORT_BASE=9631 spikes/eval/browser.sh <profile> &`, then
//! `BROWSEROS_CDP_PORT=9631 cargo test -p browseros-playwright --test sweep_scope_live -- --ignored`.
//! Every test opens its own tab and closes it, one test at a time.

#[path = "support/fixture_server.rs"]
#[expect(dead_code, reason = "no script pages here")]
mod fixture_server;
#[path = "support/live_run.rs"]
#[expect(
    dead_code,
    reason = "the capture refusal and the click serve pages_live, which includes this too"
)]
mod live_run;
#[path = "support/locator.rs"]
#[expect(
    dead_code,
    reason = "element scripts here need CSS and frame hops only"
)]
mod locator;

use std::collections::HashMap;
use std::future::Future;
use std::mem;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use browseros_cdp::{CdpError, CdpEvent};
use browseros_core::{CdpConnection, FrameId, SessionId};
use browseros_playwright::{ErrorCode, PlaywrightError};
use browseros_trace::ErrorOwner;
use fixture_server::{FixtureServer, Page, nest_pages};
use futures_util::future::BoxFuture;
use live_run::{EnvCall, LiveRun, TestPage, TestResult, brand, plain};
use locator::{attr, css, enter_frame, locator};
use serde_json::{Value, json};
use tokio::sync::{Semaphore, broadcast, mpsc};
use tokio::time;

/// One test at a time: two of them time their ops, and each connects its own `Playwright`,
/// whose frame registry attaches to every frame target in the browser.
static ALONE: Semaphore = Semaphore::const_new(1);

/// From top, a3's field: b1, then c2, then a3 (`frames` crosses origins, `document` does not).
const A3_FIELD: &str = "frames[1].frames[0].frames[0].document.querySelector('input')";

/// From b2 (under a1), its same-origin sibling b1's field, on another session.
const B1_FIELD: &str = "parent.parent.frames[1].document.querySelector('input')";

/// `hang.html` spins its main thread forever from 1.5 s after its load event.
const HANG_STARTS: Duration = Duration::from_millis(2_000);

/// How long an op with no Playwright timeout tries a late frame (`UNTIMED_RETRY` in the crate).
const UNTIMED_RETRY: Duration = Duration::from_secs(5);

/// The short Playwright timeout the hung-frame test gives an op that has one.
const SHORT_TIMEOUT: Duration = Duration::from_millis(1_500);

/// The nested fixture with the unresponsive cross-site frame D (`top.html?hang` adds it).
fn nest_and_hang() -> HashMap<String, Page> {
    let mut pages = nest_pages();
    pages.insert(
        "/nest/hang.html".to_owned(),
        Page::html(include_str!("data/nest/hang.html")).with_header("cache-control", "no-store"),
    );
    pages
}

/// Opens a tab on `url`, runs `body` on it, and closes the tab whatever happened.
async fn on_page<F, Fut>(run: LiveRun, url: &str, body: F) -> TestResult
where
    F: FnOnce(LiveRun, TestPage) -> Fut,
    Fut: Future<Output = (LiveRun, TestResult)>,
{
    let page = run.new_page().await?;
    let opened = run
        .invoke(
            "page.goto",
            json!({ "call": { "timeoutMs": 30_000 }, "page": brand(&page), "url": url }),
        )
        .await;
    if let Err(error) = opened {
        run.close_page(&page).await;
        return Err(error.into());
    }
    let (run, result) = body(run, page.clone()).await;
    run.close_page(&page).await;
    result
}

/// A function script as the prelude sends it, its argument `undefined`.
fn script(function: &str) -> Value {
    json!({ "run": { "expression": function, "isFunction": true, "arg": r#"{"v":"undefined"}"# } })
}

/// `page.evaluate(fn)` as the prelude sends it; the result is in Playwright's in-page format.
async fn page_evaluate(
    run: &LiveRun,
    page: &TestPage,
    function: &str,
) -> Result<Value, PlaywrightError> {
    run.invoke(
        "page.evaluate",
        json!({ "page": brand(page), "script": script(function) }),
    )
    .await
}

/// `page.waitForFunction(fn)` as the prelude sends it: the value it settled with.
async fn wait_for_function(
    run: &LiveRun,
    page: &TestPage,
    function: &str,
) -> Result<Value, PlaywrightError> {
    wait_for_function_within(run, page, function, 5_000).await
}

/// [`wait_for_function`] with Playwright's timeout of `timeout_ms`.
async fn wait_for_function_within(
    run: &LiveRun,
    page: &TestPage,
    function: &str,
    timeout_ms: u64,
) -> Result<Value, PlaywrightError> {
    let waited = run
        .invoke(
            "page.waitForFunction",
            json!({ "call": { "timeoutMs": timeout_ms }, "page": brand(page),
                    "script": script(function) }),
        )
        .await?;
    Ok(waited.get("value").cloned().unwrap_or(Value::Null))
}

/// `selector` in the frames down `titles` from the main frame (the nested fixture's iframes
/// carry their page's name as their title).
fn down(page: &TestPage, titles: &[&str], selector: &str) -> Value {
    let mut parts = Vec::new();
    for title in titles {
        parts.push(attr(&format!("[title=\"{title}\"s]")));
        parts.push(enter_frame());
    }
    parts.push(css(selector));
    locator(
        page,
        &parts,
        &format!("{titles:?} >> locator('{selector}')"),
    )
}

/// `locator.evaluate(fn)`, or `$eval` for an `api` ending so, as the prelude sends it; the
/// answer as plain JSON.
async fn element_evaluate(
    run: &LiveRun,
    api: &str,
    target: &Value,
    function: &str,
) -> Result<Value, PlaywrightError> {
    element_evaluate_within(run, api, target, function, 10_000).await
}

/// [`element_evaluate`] with Playwright's timeout of `timeout_ms`.
async fn element_evaluate_within(
    run: &LiveRun,
    api: &str,
    target: &Value,
    function: &str,
    timeout_ms: u64,
) -> Result<Value, PlaywrightError> {
    let no_wait = api.ends_with("$eval");
    run.invoke(
        "locator.evaluate",
        json!({ "call": { "api": api, "timeoutMs": timeout_ms }, "target": target,
                "strict": if no_wait { "firstMatch" } else { "strict" }, "noWait": no_wait,
                "script": script(function) }),
    )
    .await
    .map(plain)
}

/// `locator.evaluateAll(fn)` on `target`; the answer as plain JSON.
async fn evaluate_all(
    run: &LiveRun,
    target: &Value,
    function: &str,
) -> Result<Value, PlaywrightError> {
    run.invoke(
        "locator.evaluateAll",
        json!({ "call": { "api": "locator.evaluateAll", "timeoutMs": 0 }, "target": target,
                "script": script(function) }),
    )
    .await
    .map(plain)
}

/// Makes the field `target` names a password holding `secret`, from its own main world.
async fn type_password(run: &LiveRun, target: &Value, secret: &str) -> TestResult {
    let function = format!("(el) => {{ el.type = 'password'; el.value = {secret:?}; return 1; }}");
    element_evaluate(run, "locator.evaluate", target, &function).await?;
    Ok(())
}

/// The host was told `secret`, so it registers it.
fn reported(run: &LiveRun, secret: &str) {
    let swept = run.swept();
    assert!(
        swept.iter().any(|value| value == secret),
        "{secret} reaches values_swept: {swept:?}"
    );
}

/// One measured line, shown with `--nocapture`.
fn report(line: &str) {
    #[expect(
        clippy::print_stdout,
        reason = "the cost and timing lines are what a reviewer reruns these tests for"
    )]
    {
        println!("{line}");
    }
}

/// Nothing was marked sent since the calls were last cleared.
fn not_sent(run: &LiveRun, what: &str) {
    let calls = run.calls();
    assert!(
        !calls.contains(&EnvCall::MarkSent),
        "{what}: nothing was sent: {calls:?}"
    );
}

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_top_script_reads_a_same_site_frame_under_cross_site_frames_masked()
-> TestResult {
    let _alone = ALONE.acquire().await?;
    let server = FixtureServer::start(nest_pages()).await?;
    let url = server.url("127.0.0.1", "/nest/top.html");
    on_page(LiveRun::connect().await?, &url, |run, page| async move {
        let result = async {
            run.eval(
                &page,
                &format!("(() => {{ const f = {A3_FIELD}; f.type = 'password'; f.value = 'a3-secret-55'; return 1; }})()"),
            )
            .await?;
            let read = page_evaluate(&run, &page, &format!("() => [document.title, {A3_FIELD}.value]")).await?;
            assert_eq!(
                read,
                json!({ "a": ["Top", "[private]"], "id": 1 }),
                "page.evaluate on top reaches a3 through two cross-site frames, and a3 is swept"
            );
            reported(&run, "a3-secret-55");
            let waited =
                wait_for_function(&run, &page, &format!("() => {A3_FIELD}.value")).await?;
            assert_eq!(waited, json!("[private]"), "page.waitForFunction");
            run.eval(&page, "(() => { const f = document.querySelector('input'); f.type = 'password'; f.value = 'top-secret-33'; return 1; })()")
                .await?;
            let from_a3 = element_evaluate(
                &run,
                "locator.evaluate",
                &down(&page, &["b1", "c2", "a3"], "button"),
                "(el) => [el.textContent, top.document.querySelector('input').value]",
            )
            .await?;
            assert_eq!(
                from_a3,
                json!(["A3 button", "[private]"]),
                "a script in a3 reaches top, the page's own session, and it is swept"
            );
            reported(&run, "top-secret-33");
            Ok(())
        }
        .await;
        (run, result)
    })
    .await
}

/// Top's only child is a same-origin frame, on top's own session; under it a cross-site bridge,
/// and under that a frame of top's site with a password. Only the inner frame's child count
/// shows that the page holds an OOPIF at all.
fn deep_pages() -> HashMap<String, Page> {
    let child = |host: &str, path: &str| {
        format!(
            "<!doctype html><body><script>const f = document.createElement('iframe'); \
             f.src = `http://{host}:${{location.port}}{path}`; document.body.append(f);</script>"
        )
    };
    HashMap::from([
        (
            "/deep/top.html".to_owned(),
            Page::html(
                "<!doctype html><title>Deep</title><iframe src=\"/deep/inner.html\"></iframe>",
            ),
        ),
        (
            "/deep/inner.html".to_owned(),
            Page::html(&child("localhost", "/deep/bridge.html")),
        ),
        (
            "/deep/bridge.html".to_owned(),
            Page::html(&child("127.0.0.1", "/deep/leaf.html")),
        ),
        (
            "/deep/leaf.html".to_owned(),
            Page::html("<!doctype html><input type=\"password\">"),
        ),
    ])
}

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_an_oopif_under_a_same_process_frame_is_found() -> TestResult {
    let _alone = ALONE.acquire().await?;
    let server = FixtureServer::start(deep_pages()).await?;
    let url = server.url("127.0.0.1", "/deep/top.html");
    on_page(LiveRun::connect().await?, &url, |run, page| async move {
        let result = async {
            let leaf = "frames[0].frames[0].frames[0].document.querySelector('input')";
            run.eval(&page, &format!("{leaf}.value = 'deep-secret-88'"))
                .await?;
            let read =
                page_evaluate(&run, &page, &format!("() => [frames.length, {leaf}.value]")).await?;
            assert_eq!(
                read,
                json!({ "a": [1, "[private]"], "id": 1 }),
                "the frame of top's site under the bridge is swept"
            );
            reported(&run, "deep-secret-88");
            Ok(())
        }
        .await;
        (run, result)
    })
    .await
}

/// Top hides a cross-site bridge in a closed shadow root, and under it a frame of top's site
/// with a password. `window.length` counts no frame of a shadow tree and top's session lists
/// none, so only the browser's target list shows that the page holds an OOPIF (measured by
/// FX-F). The page keeps the bridge in a global, as a widget's own code would, and a script
/// reaches the field through it.
fn shadow_pages() -> HashMap<String, Page> {
    HashMap::from([
        (
            "/shadow/top.html".to_owned(),
            Page::html(
                "<!doctype html><title>Shadow</title><div id=\"host\"></div><script>\
                 const root = document.getElementById('host').attachShadow({ mode: 'closed' }); \
                 const bridge = document.createElement('iframe'); \
                 bridge.src = `http://localhost:${location.port}/shadow/bridge.html`; \
                 root.append(bridge); window.bridge = bridge;</script>",
            ),
        ),
        (
            "/shadow/bridge.html".to_owned(),
            Page::html(
                "<!doctype html><body><script>const f = document.createElement('iframe'); \
                 f.src = `http://127.0.0.1:${location.port}/shadow/leaf.html`; \
                 document.body.append(f);</script>",
            ),
        ),
        (
            "/shadow/leaf.html".to_owned(),
            Page::html("<!doctype html><input type=\"password\">"),
        ),
    ])
}

/// Runs `expression` in the page's main world until it returns `true`, for up to 5 s.
async fn until_true(run: &LiveRun, page: &TestPage, expression: &str) -> TestResult {
    for _ in 0..50 {
        if run.eval(page, expression).await? == json!(true) {
            return Ok(());
        }
        time::sleep(Duration::from_millis(100)).await;
    }
    Err(format!("never true: {expression}").into())
}

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_an_oopif_in_a_closed_shadow_root_is_found() -> TestResult {
    let _alone = ALONE.acquire().await?;
    let server = FixtureServer::start(shadow_pages()).await?;
    let url = server.url("127.0.0.1", "/shadow/top.html");
    on_page(LiveRun::connect().await?, &url, |run, page| async move {
        let result = async {
            let leaf = "bridge.contentWindow.frames[0].document.querySelector('input')";
            until_true(
                &run,
                &page,
                &format!(
                    "(() => {{ try {{ {leaf}.value = 'shadow-secret-77'; return true; }} \
                     catch (_) {{ return false; }} }})()"
                ),
            )
            .await?;
            let read =
                page_evaluate(&run, &page, &format!("() => [frames.length, {leaf}.value]")).await?;
            assert_eq!(
                read,
                json!({ "a": [0, "[private]"], "id": 1 }),
                "the frame of top's site under the shadow-tree bridge is swept"
            );
            reported(&run, "shadow-secret-77");
            Ok(())
        }
        .await;
        (run, result)
    })
    .await
}

/// What a [`DetachOnUse`] is to do, and what it did.
#[derive(Debug, Default, PartialEq, Eq)]
enum Arming {
    #[default]
    Idle,
    /// Detach `session` just before the next command on it. A session the browser attached on
    /// its own is detached on the session that auto-attached it, `parent`: the browser's own
    /// session answers "No session with given id" (measured).
    Armed {
        session: SessionId,
        parent: SessionId,
    },
    /// The browser detached it.
    Detached,
    /// The browser refused the detach, with this error.
    Refused(String),
}

/// The run's connection, which detaches the session a test armed it with just before the next
/// command on it. That leaves core's frame registry holding a session the browser dropped, as
/// another client's attach and detach, or a lost `Target.detachedFromTarget`, does; the command
/// then fails as Chromium fails it, "No session with given id".
struct DetachOnUse {
    browser: Arc<dyn CdpConnection>,
    arming: Arc<Mutex<Arming>>,
}

impl DetachOnUse {
    /// Detaches `session` first when it is the armed one, once, then sends `command` whatever
    /// the detach did.
    fn detach_first<'a, T: Send + 'a>(
        &'a self,
        session: Option<&SessionId>,
        command: BoxFuture<'a, Result<T, CdpError>>,
    ) -> BoxFuture<'a, Result<T, CdpError>> {
        let detach = {
            let mut arming = self.arming.lock().unwrap_or_else(PoisonError::into_inner);
            match &*arming {
                Arming::Armed { session: armed, .. } if Some(armed) == session => {
                    Some(mem::take(&mut *arming))
                }
                _ => None,
            }
        };
        Box::pin(async move {
            if let Some(Arming::Armed { session, parent }) = detach {
                let detached = self
                    .browser
                    .send(
                        "Target.detachFromTarget",
                        json!({ "sessionId": session.as_str() }),
                        Some(&parent),
                    )
                    .await;
                *self.arming.lock().unwrap_or_else(PoisonError::into_inner) = match detached {
                    Ok(_) => Arming::Detached,
                    Err(error) => Arming::Refused(error.to_string()),
                };
            }
            command.await
        })
    }
}

impl CdpConnection for DetachOnUse {
    fn send<'a>(
        &'a self,
        method: &'a str,
        params: Value,
        session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<Value, CdpError>> {
        self.detach_first(session, self.browser.send(method, params, session))
    }

    fn send_raw_json<'a>(
        &'a self,
        method: &'a str,
        params_json: &'a str,
        session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<String, CdpError>> {
        self.detach_first(
            session,
            self.browser.send_raw_json(method, params_json, session),
        )
    }

    fn send_at_epoch<'a>(
        &'a self,
        expected_epoch: u64,
        method: &'a str,
        params: Value,
        session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<Value, CdpError>> {
        self.detach_first(
            session,
            self.browser
                .send_at_epoch(expected_epoch, method, params, session),
        )
    }

    fn send_raw_json_at_epoch<'a>(
        &'a self,
        expected_epoch: u64,
        method: &'a str,
        params_json: &'a str,
        session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<String, CdpError>> {
        self.detach_first(
            session,
            self.browser
                .send_raw_json_at_epoch(expected_epoch, method, params_json, session),
        )
    }

    fn events(&self) -> broadcast::Receiver<CdpEvent> {
        self.browser.events()
    }

    fn events_targeted(&self, method: &str) -> mpsc::UnboundedReceiver<CdpEvent> {
        self.browser.events_targeted(method)
    }

    fn is_connected(&self) -> bool {
        self.browser.is_connected()
    }

    fn connection_epoch(&self) -> u64 {
        self.browser.connection_epoch()
    }
}

/// The target of the out-of-process frame at `url`.
async fn frame_target(run: &LiveRun, url: &str) -> TestResult<FrameId> {
    let targets = run
        .browser()
        .cdp(
            "Target.getTargets",
            json!({ "filter": [{ "type": "iframe" }] }),
            None,
        )
        .await?;
    let target = targets
        .get("targetInfos")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .find(|info| info.get("url").and_then(Value::as_str) == Some(url))
        .and_then(|info| info.get("targetId").and_then(Value::as_str))
        .ok_or(format!("{url} is its own target"))?;
    Ok(FrameId(target.to_owned()))
}

/// The session core holds for the out-of-process frame `frame` of `page`, the one a sweep uses.
async fn held_session(run: &LiveRun, page: &TestPage, frame: FrameId) -> TestResult<SessionId> {
    let held = run
        .browser()
        .frames()
        .resolve_frame_target(page.id.clone(), Some(frame), None, true)
        .await?;
    Ok(held
        .session
        .session_id()
        .cloned()
        .ok_or("an OOPIF has a session")?)
}

/// a3's debugger session is lost between two `page.evaluate` calls while core still holds it:
/// the browser detaches it just before the second call's first command on it, so core learns
/// of it only as that command fails. (Detached between the calls, it is already forgotten by
/// the second, and the old sweep reattached a3 too; measured.) The second call's script
/// reads a3's password and clears the field, so a sweep that skipped a3 before the script
/// would never learn the value, and the sweep after it would find the field empty. The lost
/// session is reattached and a3 swept, so the value comes back masked.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_lost_session_never_skips_a_same_site_frame() -> TestResult {
    let _alone = ALONE.acquire().await?;
    let server = FixtureServer::start(nest_pages()).await?;
    let url = server.url("127.0.0.1", "/nest/top.html");
    let arming = Arc::new(Mutex::new(Arming::Idle));
    let detaching = Arc::clone(&arming);
    let run = LiveRun::connect_through(move |client| {
        Arc::new(DetachOnUse {
            browser: Arc::new(client),
            arming: detaching,
        }) as Arc<dyn CdpConnection>
    })
    .await?;
    on_page(run, &url, |run, page| async move {
        let result = async {
            run.eval(
                &page,
                &format!("(() => {{ const f = {A3_FIELD}; f.type = 'password'; f.value = 'a3-secret-55'; return 1; }})()"),
            )
            .await?;
            let first = page_evaluate(&run, &page, &format!("() => {A3_FIELD}.value")).await?;
            assert_eq!(first, json!("[private]"), "a3 is swept on its session");
            let a3 = frame_target(&run, &server.url("127.0.0.1", "/nest/a3.html")).await?;
            let c2 = frame_target(&run, &server.url("[::1]", "/nest/c2.html")).await?;
            let held = held_session(&run, &page, a3).await?;
            // c2's session auto-attached a3, so it owns a3's session.
            let parent = held_session(&run, &page, c2).await?;
            *arming.lock().unwrap_or_else(PoisonError::into_inner) = Arming::Armed {
                session: held.clone(),
                parent,
            };
            run.clear_calls();
            let read = page_evaluate(
                &run,
                &page,
                &format!("() => {{ const f = {A3_FIELD}; const value = f.value; f.value = ''; return value; }}"),
            )
            .await;
            assert_eq!(
                *arming.lock().unwrap_or_else(PoisonError::into_inner),
                Arming::Detached,
                "the browser detached the session core held for a3 ({held:?}) during the call"
            );
            report(&format!("page.evaluate after a3's session was lost: {read:?}"));
            assert_eq!(
                read?,
                json!("[private]"),
                "a3 is reattached and swept before the script, never skipped"
            );
            reported(&run, "a3-secret-55");
            Ok(())
        }
        .await;
        (run, result)
    })
    .await
}

/// A script in b2 reaches its same-origin sibling b1, which sits on another session: every
/// element form sweeps it, and the host learns b1's value (fx-e `c77bb3c6e`, measured raw on
/// r10c).
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_an_oopif_script_reads_its_same_origin_sibling_masked() -> TestResult {
    let _alone = ALONE.acquire().await?;
    let server = FixtureServer::start(nest_pages()).await?;
    let url = server.url("127.0.0.1", "/nest/top.html");
    on_page(LiveRun::connect().await?, &url, |run, page| async move {
        let result = async {
            type_password(&run, &down(&page, &["b1"], "input"), "b1-secret-77").await?;
            let in_b2 = down(&page, &["a1", "b2"], "button");
            let read = format!("(el) => [el.textContent, {B1_FIELD}.value]");
            for api in ["locator.evaluate", "page.$eval"] {
                let value = element_evaluate(&run, api, &in_b2, &read).await?;
                assert_eq!(
                    value,
                    json!(["B2 button", "[private]"]),
                    "{api}: b2's script reaches b1 on another session, and b1 is swept"
                );
            }
            let all = evaluate_all(
                &run,
                &in_b2,
                &format!("(els) => [els.length, {B1_FIELD}.value]"),
            )
            .await?;
            assert_eq!(all, json!([1, "[private]"]), "locator.evaluateAll");
            let handle = run
                .invoke(
                    "locator.elementHandle",
                    json!({ "call": { "timeoutMs": 10_000 }, "target": in_b2, "strict": "strict" }),
                )
                .await?;
            let by_handle =
                element_evaluate(&run, "elementHandle.evaluate", &handle, &read).await?;
            assert_eq!(
                by_handle,
                json!(["B2 button", "[private]"]),
                "elementHandle.evaluate"
            );
            reported(&run, "b1-secret-77");
            Ok(())
        }
        .await;
        (run, result)
    })
    .await
}

/// A same-site frame that cannot be swept: another origin of the page's site, asking for an
/// origin-keyed agent cluster, so Chromium gives it a process of its own (measured), which
/// then spins. The page's own frame still answers. Each op tries the frame again on fresh
/// budgets until its own bound and then refuses: `page.evaluate`, which has no timeout, for
/// the untimed window; the other two inside their 1.5 s.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_hung_same_site_frame_refuses_the_script_with_nothing_sent() -> TestResult {
    let _alone = ALONE.acquire().await?;
    let hang = Page::html(include_str!("data/nest/hang.html"))
        .with_header("origin-agent-cluster", "?1")
        .with_header("cache-control", "no-store");
    let hanging = FixtureServer::start(HashMap::from([("/hang.html".to_owned(), hang)])).await?;
    let parent = format!(
        "<!doctype html><title>Parent</title><input id=\"p\" type=\"password\">\
         <iframe title=\"hang\" src=\"{}\"></iframe>",
        hanging.url("127.0.0.1", "/hang.html")
    );
    let server = FixtureServer::start(HashMap::from([(
        "/parent.html".to_owned(),
        Page::html(&parent),
    )]))
    .await?;
    let url = server.url("127.0.0.1", "/parent.html");
    on_page(LiveRun::connect().await?, &url, |run, page| async move {
        let result = async {
            time::sleep(HANG_STARTS).await;
            assert_eq!(
                run.eval(&page, "document.title").await?,
                json!("Parent"),
                "the page's own frame still answers"
            );
            let script = "() => { window.ran = true; return document.getElementById('p').value; }";
            let field = locator(&page, &[css("#p")], "locator('#p')");
            for api in ["page.evaluate", "page.waitForFunction", "locator.evaluate"] {
                run.clear_calls();
                let started = Instant::now();
                let (refused, bound) = match api {
                    "page.evaluate" => (page_evaluate(&run, &page, script).await, UNTIMED_RETRY),
                    "page.waitForFunction" => (
                        wait_for_function_within(&run, &page, script, 1_500).await,
                        SHORT_TIMEOUT,
                    ),
                    _ => (
                        element_evaluate_within(
                            &run,
                            api,
                            &field,
                            "(el) => { window.ran = true; return el.value; }",
                            1_500,
                        )
                        .await,
                        SHORT_TIMEOUT,
                    ),
                };
                let took = started.elapsed();
                let error = refused
                    .err()
                    .ok_or(format!("{api}: an unswept same-site frame refuses"))?;
                report(&format!("{api} refused after {took:?}: {error}"));
                assert_eq!(
                    (error.error_code(), error.owner()),
                    (Some(ErrorCode::Unsupported), ErrorOwner::Neo),
                    "{api}: {error}"
                );
                assert!(
                    error.message().contains("did not answer"),
                    "{api}: the message says why: {error}"
                );
                assert!(
                    took >= Duration::from_secs(1) && took < bound + Duration::from_millis(1_000),
                    "{api}: refused after a late try and inside its bound of {bound:?}, took \
                     {took:?}"
                );
                not_sent(&run, api);
            }
            assert_eq!(
                run.eval(&page, "window.ran === true").await?,
                json!(false),
                "no script ran"
            );
            Ok(())
        }
        .await;
        (run, result)
    })
    .await
}

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_hung_cross_site_frame_does_not_slow_page_evaluate() -> TestResult {
    let _alone = ALONE.acquire().await?;
    let server = FixtureServer::start(nest_and_hang()).await?;
    let url = server.url("127.0.0.1", "/nest/top.html?hang");
    on_page(LiveRun::connect().await?, &url, |run, page| async move {
        let result = async {
            time::sleep(HANG_STARTS).await;
            run.eval(
                &page,
                &format!("(() => {{ const f = {A3_FIELD}; f.type = 'password'; f.value = 'a3-secret-55'; return 1; }})()"),
            )
            .await?;
            let mut took = Vec::new();
            for _call in 0..10 {
                let started = Instant::now();
                let read = page_evaluate(&run, &page, &format!("() => [frames.length, {A3_FIELD}.value]")).await?;
                took.push(started.elapsed());
                assert_eq!(read, json!({ "a": [3, "[private]"], "id": 1 }), "the same-site frame is still swept");
            }
            took.sort();
            let slowest = took.last().copied().unwrap_or_default();
            report(&format!("page.evaluate beside a hung cross-site frame: median {:?}, slowest {slowest:?}", took.get(took.len() / 2)));
            assert!(
                slowest < Duration::from_millis(250),
                "no call waits on the hung frame (its budget is 1 s): {took:?}"
            );
            Ok(())
        }
        .await;
        (run, result)
    })
    .await
}

/// A page on 127.0.0.1 that builds twelve frames on `localhost` and twelve on `[::1]` (24 OOPIF
/// targets of two other sites, each with a field).
const MANY: &str = r#"<!doctype html><title>Many</title><input id="p"><script>
for (let index = 0; index < 24; index++) {
  const frame = document.createElement('iframe');
  const host = index % 2 === 0 ? 'localhost' : '[::1]';
  frame.title = 'leaf' + index;
  frame.src = `http://${host}:${location.port}/leaf.html`;
  document.body.append(frame);
}
</script>"#;

fn cost_pages() -> HashMap<String, Page> {
    let leaf = "<!doctype html><title>Leaf</title><input aria-label=\"leaf\"><button>Leaf</button>";
    let mut pages = nest_and_hang();
    pages.insert("/many.html".to_owned(), Page::html(MANY));
    pages.insert("/leaf.html".to_owned(), Page::html(leaf));
    pages.insert(
        "/one.html".to_owned(),
        Page::html("<!doctype html><title>One</title><input id=\"p\" type=\"password\">"),
    );
    pages
}

/// What `calls` timed runs of one op took.
struct Cost {
    median: Duration,
    slowest: Duration,
}

/// The cost of `calls` runs of `op`, after three to warm the worlds up.
async fn cost<F, Fut>(calls: usize, op: F) -> TestResult<Cost>
where
    F: Fn() -> Fut,
    Fut: Future<Output = Result<Value, PlaywrightError>>,
{
    for _warm in 0..3 {
        op().await?;
    }
    let mut took = Vec::with_capacity(calls);
    for _call in 0..calls {
        let started = Instant::now();
        op().await?;
        took.push(started.elapsed());
    }
    took.sort();
    Ok(Cost {
        median: took.get(took.len() / 2).copied().unwrap_or_default(),
        slowest: took.last().copied().unwrap_or_default(),
    })
}

/// Prints what `page.evaluate` costs on one frame, on the nested fixture with and without its
/// hung cross-site frame, and beside 24 cross-site frames.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn measures_the_cost_of_the_sweep() -> TestResult {
    let _alone = ALONE.acquire().await?;
    let server = FixtureServer::start(cost_pages()).await?;
    let pages = [
        ("one frame", server.url("127.0.0.1", "/one.html")),
        ("nested fixture", server.url("127.0.0.1", "/nest/top.html")),
        (
            "nested fixture beside a hung cross-site frame",
            server.url("127.0.0.1", "/nest/top.html?hang"),
        ),
        (
            "24 cross-site frames",
            server.url("127.0.0.1", "/many.html"),
        ),
    ];
    for (name, url) in pages {
        let hangs = url.ends_with("?hang");
        on_page(LiveRun::connect().await?, &url, |run, page| async move {
            let result = async {
                if hangs {
                    time::sleep(HANG_STARTS).await;
                }
                let Cost { median, slowest } =
                    cost(40, || page_evaluate(&run, &page, "() => document.title")).await?;
                report(&format!(
                    "page.evaluate, {name}: median {median:?}, slowest {slowest:?}"
                ));
                Ok(())
            }
            .await;
            (run, result)
        })
        .await?;
    }
    Ok(())
}
