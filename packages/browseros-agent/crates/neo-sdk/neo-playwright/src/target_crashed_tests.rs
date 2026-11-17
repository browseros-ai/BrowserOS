//! Crash observations outlive a run and refuse renderer admission while browser-only close
//! and a replacement native target remain usable.

use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use browseros_cdp::{CdpError, CdpEvent};
use browseros_core::pages::NewPageOptions;
use browseros_core::{
    BrowserProfileId, BrowserSession, BrowserSessionHooks, BrowserView, CdpConnection, CoreError,
    FrameId, NativeCapabilities, SessionId, TargetBinding, TargetId, TargetIntent,
};
use futures_util::FutureExt;
use futures_util::future::BoxFuture;
use serde_json::{Value, json};
use tokio::sync::{broadcast, mpsc};
use tokio::{task, time};

use crate::events::EventFence;
use crate::op_scope::OpScope;
use crate::playwright_error::TARGET_CRASH_MESSAGE;
use crate::target::CallHeader;
use crate::test_env::{EnvCall, TestEnv};
use crate::test_support::{
    ProbeAnswer, SCRIPTED_CHILD, SCRIPTED_MAIN, SCRIPTED_NEW, SCRIPTED_OOPIF, ScriptedRun,
    ScriptedTab, TabScript, TestResult,
};
use crate::{
    Answer, ErrorCode, Op, OpEnv, Playwright, PlaywrightError, Refusal, RunState, TimedOut,
};

const REFUSAL_BOUND: Duration = Duration::from_millis(100);
const REPLACEMENT: &str = "replacement-target";

/// The browser remains responsive when the renderer stops answering. The crash reply is
/// emitted only by an already-sent navigation; later renderer silence exposes admission that
/// accidentally attaches, installs scripts or sends a second operation before refusing.
#[derive(Default)]
struct CrashedTab {
    crash_navigation: AtomicBool,
    crash_input: AtomicBool,
    renderer_silent: AtomicBool,
    replaced: AtomicBool,
    closed: AtomicBool,
    profile_targeting: bool,
    crash_preparation: AtomicBool,
    direct_crash: Mutex<Option<&'static str>>,
    delay_navigation: bool,
}

impl CrashedTab {
    fn target(&self) -> &'static str {
        if self.replaced.load(Ordering::SeqCst) {
            REPLACEMENT
        } else {
            SCRIPTED_MAIN
        }
    }

    fn tab(&self) -> Value {
        json!({ "tabId": 7, "targetId": self.target(),
            "windowId": 1, "url": "https://a.test/", "title": "A", "isActive": true,
            "isLoading": false, "loadProgress": 1.0, "isPinned": false,
            "isHidden": false, "browserProfileId": "test", "liveBrowserContextId": "live-test" })
    }
}

impl TabScript for CrashedTab {
    fn probe(&self, _frame: &str, _request: &Value) -> ProbeAnswer {
        ProbeAnswer::unexpected()
    }

    fn outlasts_client(&self, method: &str) -> bool {
        self.renderer_silent.load(Ordering::SeqCst) && renderer_command(method)
    }

    fn cdp_error(&self, method: &str, _params: &Value) -> Option<&'static str> {
        let mut direct = self
            .direct_crash
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if direct.as_ref().is_some_and(|selected| *selected == method) {
            *direct = None;
            return Some("Target crashed");
        }
        ((method == "Page.navigate" && self.crash_navigation.swap(false, Ordering::SeqCst))
            || (method == "Input.dispatchMouseEvent"
                && self.crash_input.swap(false, Ordering::SeqCst))
            || (method == "Page.enable" && self.crash_preparation.swap(false, Ordering::SeqCst)))
        .then_some("Target crashed")
    }

    fn delay(&self, method: &str) -> Duration {
        if self.delay_navigation && method == "Page.navigate" {
            Duration::from_millis(10)
        } else {
            Duration::ZERO
        }
    }

    fn cdp_reply(&self, method: &str, params: &Value) -> Option<Value> {
        match method {
            "Target.closeTarget" => {
                self.closed.store(true, Ordering::SeqCst);
                Some(json!({ "success": true }))
            }
            "Browser.getTabs" if self.closed.load(Ordering::SeqCst) => Some(json!({ "tabs": [] })),
            "Browser.getTabs" if self.replaced.load(Ordering::SeqCst) || self.profile_targeting => {
                Some(json!({ "tabs": [self.tab()] }))
            }
            "Browser.getTabInfo" if params["tabId"] == 7 => Some(json!({ "tab": self.tab() })),
            "Browser.getProfiles" => Some(json!({ "profiles": [{ "browserProfileId": "test",
                "directory": "Test", "name": "Test", "loaded": true,
                "browserContextId": "live-test" }] })),
            "Browser.getWindows" => Some(json!({ "windows": [{ "windowId": 1,
                "windowType": "normal", "bounds": {}, "isActive": true, "isVisible": true,
                "tabCount": 1, "browserProfileId": "test", "liveBrowserContextId": "live-test" }] })),
            "Target.getTargets" => {
                let targets: Vec<Value> = (!self.closed.load(Ordering::SeqCst))
                    .then(|| {
                        json!({ "targetId": self.target(), "type": "page",
                        "url": "https://a.test/" })
                    })
                    .into_iter()
                    .collect();
                Some(json!({ "targetInfos": targets }))
            }
            _ => None,
        }
    }
}

/// Browser facts and root target discovery can still be read under view policy. Attaching
/// a page is preparation for renderer work, so it is counted along with every renderer
/// domain rather than exempted with browser-only target commands.
fn renderer_command(method: &str) -> bool {
    method == "Target.attachToTarget"
        || (!method.starts_with("Browser.") && !method.starts_with("Target."))
}

fn renderer_commands<S: TabScript>(tab: &ScriptedTab<S>) -> Vec<String> {
    tab.sent_methods()
        .into_iter()
        .filter(|method| renderer_command(method))
        .collect()
}

async fn settle_watcher(run: &ScriptedRun) -> TestResult {
    assert!(
        run.playwright
            .synchronize_events(run.playwright.event_fence())
            .await,
        "the watcher must finish discovery before command deltas are measured"
    );
    // The watcher seeds frame trees in a separate task. Let its already-responsive setup
    // finish before the test makes the renderer silent or takes a command baseline.
    time::sleep(Duration::from_millis(1)).await;
    Ok(())
}

/// The old environment is dropped, including its `RunState`. Only the browser facade and
/// native page brand cross this boundary, as they do between separate host runs.
fn next_run(run: ScriptedRun) -> ScriptedRun {
    let env = TestEnv::over(run.env.view().clone());
    assert!(
        !Arc::ptr_eq(run.env.run_state(), env.run_state()),
        "the regression must not reuse per-run crash knowledge"
    );
    ScriptedRun {
        playwright: run.playwright,
        env,
        page: run.page,
    }
}

async fn crash_reply(
    run: &ScriptedRun,
    tab: &ScriptedTab<CrashedTab>,
) -> TestResult<PlaywrightError> {
    tab.script().crash_navigation.store(true, Ordering::SeqCst);
    let op = Op::from_name("page.goto").ok_or("page.goto is registered")?;
    let error = run
        .playwright
        .invoke(
            &run.env,
            op,
            json!({ "call": { "timeoutMs": 10_000 }, "page": run.page,
                "url": "https://a.test/next", "waitUntil": "commit" }),
        )
        .await
        .err()
        .ok_or("the exact crash protocol reply must fail the navigation")?;
    assert!(
        run.env.calls().contains(&EnvCall::MarkSent),
        "the crash reply interrupts a navigation that was already sent"
    );
    settle_watcher(run).await?;
    Ok(error)
}

async fn crash_event(run: &ScriptedRun, tab: &ScriptedTab<CrashedTab>) -> TestResult {
    settle_watcher(run).await?;
    let before = run.playwright.event_fence();
    tab.emit(
        "Target.targetCrashed",
        json!({ "targetId": SCRIPTED_MAIN, "status": "crashed", "errorCode": 1 }),
    );
    // The connection first forwards the injected event to the facade's watcher. Its fence
    // must include that event; synchronizing an earlier fence would test a scheduling race.
    time::sleep(Duration::from_millis(1)).await;
    let fence = run.playwright.event_fence();
    assert!(
        fence > before,
        "the connection must forward the crash event"
    );
    assert!(run.playwright.synchronize_events(fence).await);
    assert!(
        run.playwright.events_handled_through(fence),
        "a held frame-tree event is not yet an observed crash"
    );
    Ok(())
}

#[derive(Clone, Copy, Debug)]
enum NativeReadFailure {
    Timeout,
    ConnectionLost,
}

/// Keeps crash events and renderer behavior real to the scripted connection, but can fail
/// both native refresh routes with typed transport errors. Failing only getTabInfo would let
/// Core's getTabs fallback hide the ordering bug this fixture needs to expose.
struct FailingNativeReads {
    tab: Arc<ScriptedTab<CrashedTab>>,
    failure: Mutex<Option<NativeReadFailure>>,
    get_tab_info: AtomicUsize,
    get_tabs: AtomicUsize,
}

impl FailingNativeReads {
    fn new(tab: Arc<ScriptedTab<CrashedTab>>) -> Arc<Self> {
        Arc::new(Self {
            tab,
            failure: Mutex::default(),
            get_tab_info: AtomicUsize::default(),
            get_tabs: AtomicUsize::default(),
        })
    }

    async fn run(self: &Arc<Self>) -> TestResult<ScriptedRun> {
        let connection = Arc::clone(self) as Arc<dyn CdpConnection>;
        let browser = BrowserSession::new(connection, BrowserSessionHooks::default());
        let playwright = Playwright::new(Arc::clone(&browser));
        let listed = browser.pages.list().await?;
        let page = listed
            .first()
            .ok_or("the native tab is cached before failure")?;
        Ok(ScriptedRun {
            playwright,
            env: TestEnv::over(BrowserView::unbound(browser, None)),
            page: serde_json::from_value(json!({ "$neo": "page", "tab": page.tab_id }))?,
        })
    }

    fn fail_with(&self, failure: NativeReadFailure) {
        self.get_tab_info.store(0, Ordering::SeqCst);
        self.get_tabs.store(0, Ordering::SeqCst);
        *self.failure.lock().unwrap_or_else(PoisonError::into_inner) = Some(failure);
    }

    fn error(&self, method: &str) -> Option<CdpError> {
        let failure = (*self.failure.lock().unwrap_or_else(PoisonError::into_inner))?;
        let counter = match method {
            "Browser.getTabInfo" => &self.get_tab_info,
            "Browser.getTabs" => &self.get_tabs,
            _ => return None,
        };
        counter.fetch_add(1, Ordering::SeqCst);
        Some(match failure {
            NativeReadFailure::Timeout => CdpError::Timeout {
                method: method.to_owned(),
            },
            NativeReadFailure::ConnectionLost => CdpError::ConnectionLost,
        })
    }
}

impl CdpConnection for FailingNativeReads {
    fn send<'a>(
        &'a self,
        method: &'a str,
        params: Value,
        session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<Value, CdpError>> {
        if let Some(error) = self.error(method) {
            return Box::pin(async move { Err(error) });
        }
        self.tab.send(method, params, session)
    }

    fn send_raw_json<'a>(
        &'a self,
        method: &'a str,
        params_json: &'a str,
        session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<String, CdpError>> {
        if let Some(error) = self.error(method) {
            return Box::pin(async move { Err(error) });
        }
        self.tab.send_raw_json(method, params_json, session)
    }

    fn events(&self) -> broadcast::Receiver<CdpEvent> {
        self.tab.events()
    }

    fn events_counted(&self) -> (broadcast::Receiver<CdpEvent>, u64) {
        self.tab.events_counted()
    }

    fn events_broadcast(&self) -> u64 {
        self.tab.events_broadcast()
    }

    fn events_targeted(&self, method: &str) -> mpsc::UnboundedReceiver<CdpEvent> {
        self.tab.events_targeted(method)
    }

    fn is_connected(&self) -> bool {
        self.tab.is_connected()
    }

    fn connection_epoch(&self) -> u64 {
        self.tab.connection_epoch()
    }
}

#[tokio::test(start_paused = true)]
async fn target_crashed_cached_refusal_precedes_native_refresh_failure_in_a_fresh_run() -> TestResult
{
    for failure in [
        NativeReadFailure::Timeout,
        NativeReadFailure::ConnectionLost,
    ] {
        let tab = ScriptedTab::new(CrashedTab::default());
        let native = FailingNativeReads::new(Arc::clone(&tab));
        let run = native.run().await?;
        crash_event(&run, &tab).await?;
        tab.script().renderer_silent.store(true, Ordering::SeqCst);
        let run = next_run(run);
        let before = renderer_commands(&tab);
        native.fail_with(failure);
        let error = time::timeout(
            REFUSAL_BOUND,
            run.playwright.invoke(
                &run.env,
                Op::from_name("page.title").ok_or("page.title")?,
                json!({ "call": { "timeoutMs": 10_000 }, "page": run.page }),
            ),
        )
        .await
        .map_err(|_elapsed| "remembered crash waited for failing native tab facts")?
        .err()
        .ok_or("the remembered native target must refuse renderer work")?;
        assert_eq!(
            error.error_code(),
            Some(ErrorCode::TargetCrashed),
            "{failure:?}"
        );
        assert_plain_crash(&error);
        assert!(
            error.message().starts_with("page.title:"),
            "the refusal keeps the operation's source context: {error}"
        );
        assert_eq!(error.answer(), Answer::Definite);
        assert_eq!(
            native.get_tab_info.load(Ordering::SeqCst),
            0,
            "remembered refusal must precede native getTabInfo: {failure:?}"
        );
        assert_eq!(
            native.get_tabs.load(Ordering::SeqCst),
            0,
            "remembered refusal must precede the native getTabs fallback: {failure:?}"
        );
        assert_eq!(
            renderer_commands(&tab),
            before,
            "the crashed renderer must receive no admission or operation command"
        );
        assert!(
            !run.env.calls().contains(&EnvCall::MarkSent),
            "the host must still derive not_sent for cached refusal"
        );
    }
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn target_crashed_unremembered_native_refresh_failures_remain_browser_disconnected()
-> TestResult {
    for failure in [
        NativeReadFailure::Timeout,
        NativeReadFailure::ConnectionLost,
    ] {
        let tab = ScriptedTab::new(CrashedTab::default());
        let native = FailingNativeReads::new(Arc::clone(&tab));
        let run = native.run().await?;
        settle_watcher(&run).await?;
        native.fail_with(failure);
        let error = run
            .playwright
            .invoke(
                &run.env,
                Op::from_name("page.title").ok_or("page.title")?,
                json!({ "page": run.page }),
            )
            .await
            .err()
            .ok_or("the native refresh must fail")?;
        assert_eq!(
            error.error_code(),
            Some(ErrorCode::BrowserDisconnected),
            "a native request failure without remembered crash is not crash evidence: {failure:?}"
        );
        assert!(
            error.message().starts_with("page.title: CDP"),
            "the original native error and operation remain visible: {error}"
        );
        assert!(
            native.get_tab_info.load(Ordering::SeqCst) > 0,
            "the control must exercise the native refresh"
        );
        assert!(
            native.get_tabs.load(Ordering::SeqCst) > 0,
            "the control must also exercise Core's native fallback"
        );
        assert!(!run.env.calls().contains(&EnvCall::MarkSent));
    }
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn target_crashed_cached_precheck_preserves_hidden_and_bound_admission() -> TestResult {
    let tab = ScriptedTab::new(CrashedTab::default());
    let native = FailingNativeReads::new(Arc::clone(&tab));
    let run = native.run().await?;
    crash_event(&run, &tab).await?;
    run.env.hide(run.page.tab());
    native.fail_with(NativeReadFailure::ConnectionLost);
    let error = run
        .playwright
        .invoke(
            &run.env,
            Op::from_name("page.title").ok_or("page.title")?,
            json!({ "page": run.page }),
        )
        .await
        .err()
        .ok_or("the hidden tab's ordinary admission must fail")?;
    assert_eq!(
        error.error_code(),
        Some(ErrorCode::BrowserDisconnected),
        "hidden tabs must not disclose cached crash knowledge ahead of ordinary admission"
    );
    assert!(native.get_tab_info.load(Ordering::SeqCst) > 0);

    let tab = ScriptedTab::new(CrashedTab {
        profile_targeting: true,
        ..CrashedTab::default()
    });
    let connection = Arc::clone(&tab) as Arc<dyn CdpConnection>;
    let browser = BrowserSession::new_with_native(
        connection,
        BrowserSessionHooks::default(),
        NativeCapabilities::from_browseros_version(Some("0.51.1.3")),
    );
    let view = BrowserView::capture(
        Arc::clone(&browser),
        Some(TargetBinding {
            intent: TargetIntent {
                browser_profile_id: BrowserProfileId("test".to_owned()),
                directory: "Test".to_owned(),
                name: "Test".to_owned(),
                window: None,
            },
            revision: 1,
        }),
        None,
    )
    .await;
    assert!(
        view.scope()?.is_some(),
        "the control must retain captured authority"
    );
    let listed = browser.pages.list().await?;
    let page = listed.first().ok_or("the bound native tab is listed")?;
    let run = ScriptedRun {
        playwright: Playwright::new(Arc::clone(&browser)),
        env: TestEnv::over(view),
        page: serde_json::from_value(json!({ "$neo": "page", "tab": page.tab_id }))?,
    };
    crash_event(&run, &tab).await?;
    tab.reconnect();
    let error = run
        .playwright
        .invoke(
            &run.env,
            Op::from_name("page.title").ok_or("page.title")?,
            json!({ "page": run.page }),
        )
        .await
        .err()
        .ok_or("Core must reject stale captured authority")?;
    assert_eq!(
        error.error_code(),
        Some(ErrorCode::TargetClosed),
        "a bound view's epoch rejection must precede remembered crash refusal"
    );
    assert!(!run.env.calls().contains(&EnvCall::MarkSent));
    Ok(())
}

fn assert_plain_crash(error: &PlaywrightError) {
    let message = error.message().to_ascii_lowercase();
    assert!(
        message.contains("page") && message.contains("crashed"),
        "the caller needs a plain page crash explanation: {error}"
    );
    assert_eq!(
        error.timed_out(),
        TimedOut::No,
        "a known crash is not a timeout"
    );
}

async fn assert_cached_refusal(
    run: &ScriptedRun,
    tab: &ScriptedTab<CrashedTab>,
    name: &str,
    input: Value,
) -> TestResult {
    let before = renderer_commands(tab);
    let op = Op::from_name(name).ok_or("the renderer operation is registered")?;
    let result = time::timeout(REFUSAL_BOUND, run.playwright.invoke(&run.env, op, input)).await;
    assert_eq!(
        renderer_commands(tab),
        before,
        "a remembered crash must refuse {name} before any extra renderer command"
    );
    assert!(
        !run.env.calls().contains(&EnvCall::MarkSent),
        "cached refusal must never acquire delivery's sent proof"
    );
    let error = result
        .map_err(|_elapsed| "a known crash waited for renderer work")?
        .err()
        .ok_or("a known crash must refuse the renderer operation")?;
    assert_plain_crash(&error);
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn target_crashed_protocol_reply_refuses_renderer_work_in_a_fresh_run() -> TestResult {
    let tab = ScriptedTab::new(CrashedTab::default());
    let run = ScriptedRun::over(&tab).await?;
    crash_reply(&run, &tab).await?;
    tab.script().renderer_silent.store(true, Ordering::SeqCst);
    let run = next_run(run);
    assert_cached_refusal(
        &run,
        &tab,
        "mouse.move",
        json!({ "call": { "timeoutMs": 10_000 }, "page": run.page, "x": 1, "y": 2 }),
    )
    .await
}

#[tokio::test]
async fn target_crashed_later_requests_in_one_operation_follow_a_new_document() -> TestResult {
    let tab = ScriptedTab::new(CrashedTab::default());
    let run = ScriptedRun::over(&tab).await?;
    settle_watcher(&run).await?;
    let browser = run.playwright.browser();
    let scope = OpScope::new(
        &run.env,
        Op::from_name("page.waitForFunction").ok_or("page.waitForFunction")?,
        CallHeader::default(),
    )
    .tracking(run.playwright.crashes(), browser);
    let admitted = scope.admit(browser, run.page.tab()).await?;
    let session = browser.protocol_session(admitted.session.session_id.clone());
    let earlier = scope.begin_protocol(&session);
    let _ = run.playwright.crashes().main_document(
        &admitted.session.target_id,
        FrameId(SCRIPTED_MAIN.to_owned()),
        "next-document".to_owned(),
        run.playwright.event_fence(),
        browser.connection().connection_epoch(),
    );
    let later = scope.begin_protocol(&session);
    let error = CoreError::Cdp(CdpError::Protocol {
        code: -32000,
        message: "Target crashed".to_owned(),
    });
    earlier.record(&error);
    assert!(
        !run.playwright
            .crashes()
            .is_crashed(&admitted.session.target_id)
    );
    later.record(&error);
    assert!(
        run.playwright
            .crashes()
            .is_crashed(&admitted.session.target_id)
    );
    drop(scope);
    let run = next_run(run);
    assert_cached_refusal(
        &run,
        &tab,
        "mouse.move",
        json!({"page": run.page, "x": 1, "y": 1}),
    )
    .await?;
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn target_crashed_source_known_direct_replies_are_cached_without_a_crash_event() -> TestResult
{
    for (name, command) in [
        ("page.requestGC", "HeapProfiler.collectGarbage"),
        ("page.pdf", "Page.printToPDF"),
    ] {
        let tab = ScriptedTab::new(CrashedTab::default());
        let run = ScriptedRun::over(&tab).await?;
        settle_watcher(&run).await?;
        *tab.script()
            .direct_crash
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(command);
        let error = run
            .playwright
            .invoke(
                &run.env,
                Op::from_name(name).ok_or("a direct page operation")?,
                json!({ "page": run.page }),
            )
            .await
            .err()
            .ok_or("the selected direct command must meet a crashed target")?;
        assert_eq!(error.error_code(), Some(ErrorCode::TargetCrashed));
        tab.script().renderer_silent.store(true, Ordering::SeqCst);
        let run = next_run(run);
        assert_cached_refusal(
            &run,
            &tab,
            "mouse.move",
            json!({ "page": run.page, "x": 1, "y": 2 }),
        )
        .await?;
    }
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn target_crashed_event_refuses_reads_writes_and_sdk_navigation_in_fresh_runs() -> TestResult
{
    let tab = ScriptedTab::new(CrashedTab::default());
    let mut run = ScriptedRun::over(&tab).await?;
    crash_event(&run, &tab).await?;
    tab.script().renderer_silent.store(true, Ordering::SeqCst);
    // SDK navigation has renderer setup of its own; external verified recovery is required
    // before these operations can be admitted just like any other renderer operation.
    for (name, args) in [
        ("page.title", json!({})),
        ("mouse.move", json!({ "x": 1, "y": 2 })),
        (
            "page.goto",
            json!({ "url": "https://a.test/recover", "waitUntil": "commit" }),
        ),
        ("page.reload", json!({ "waitUntil": "commit" })),
    ] {
        run = next_run(run);
        let mut input = json!({ "call": { "timeoutMs": 10_000 }, "page": run.page });
        input
            .as_object_mut()
            .ok_or("the operation input is an object")?
            .extend(
                args.as_object()
                    .ok_or("the operation arguments are an object")?
                    .clone(),
            );
        assert_cached_refusal(&run, &tab, name, input).await?;
    }
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn target_crashed_still_allows_browser_only_close() -> TestResult {
    let tab = ScriptedTab::new(CrashedTab::default());
    let run = ScriptedRun::over(&tab).await?;
    crash_event(&run, &tab).await?;
    tab.script().renderer_silent.store(true, Ordering::SeqCst);
    let run = next_run(run);
    let before = renderer_commands(&tab);
    let op = Op::from_name("page.close").ok_or("page.close is registered")?;
    time::timeout(
        REFUSAL_BOUND,
        run.playwright
            .invoke(&run.env, op, json!({ "page": run.page })),
    )
    .await??;
    assert_eq!(
        renderer_commands(&tab),
        before,
        "close is a browser-only operation"
    );
    assert_eq!(tab.sent("Target.closeTarget"), 1);
    assert!(
        run.env.calls().contains(&EnvCall::MarkSent),
        "the close was sent"
    );
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn target_crashed_lifecycle_waits_need_no_renderer_admission() -> TestResult {
    use std::pin::pin;

    for (event, method) in [
        ("close", "Target.targetDestroyed"),
        ("crash", "Target.targetCrashed"),
    ] {
        let tab = ScriptedTab::new(CrashedTab::default());
        let run = ScriptedRun::over(&tab).await?;
        settle_watcher(&run).await?;
        run.playwright.crashes().crashed_event(
            TargetId(SCRIPTED_MAIN.to_owned()),
            run.playwright.event_fence(),
        );
        tab.script().renderer_silent.store(true, Ordering::SeqCst);
        let before = renderer_commands(&tab);
        let mut waiting = pin!(run.playwright.invoke(
            &run.env,
            Op::from_name("events.wait").ok_or("events.wait")?,
            json!({ "page": run.page, "event": event, "since": 0 }),
        ));
        assert!(
            waiting.as_mut().now_or_never().is_none(),
            "the browser-only wait must wait for its event, not refuse renderer admission"
        );
        if event == "close" {
            tab.script().closed.store(true, Ordering::SeqCst);
        }
        tab.emit(method, json!({ "targetId": SCRIPTED_MAIN }));
        let received = time::timeout(REFUSAL_BOUND, waiting).await??;
        assert_eq!(received["event"], event);
        assert_eq!(
            renderer_commands(&tab),
            before,
            "lifecycle subscriptions must not attach or prepare a renderer"
        );
        assert!(!run.env.calls().contains(&EnvCall::MarkSent));
    }
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn target_crashed_is_not_inherited_by_a_replacement_native_target() -> TestResult {
    let tab = ScriptedTab::new(CrashedTab::default());
    let run = ScriptedRun::over(&tab).await?;
    crash_reply(&run, &tab).await?;
    tab.script().replaced.store(true, Ordering::SeqCst);
    // A browser-owned tab can retain its native tab id while Chromium replaces its target.
    // Reconcile that identity through core's ordinary browser listing, with no document
    // recovery event for the crashed target: the new target must be admitted independently.
    let listed = run.playwright.browser().pages.list().await?;
    assert_eq!(
        listed
            .first()
            .ok_or("the replacement tab is listed")?
            .target_id
            .as_str(),
        REPLACEMENT
    );
    let run = next_run(run);
    let before = tab.sent("Input.dispatchMouseEvent");
    let op = Op::from_name("mouse.move").ok_or("mouse.move is registered")?;
    time::timeout(
        Duration::from_secs(1),
        run.playwright
            .invoke(&run.env, op, json!({ "page": run.page, "x": 1, "y": 2 })),
    )
    .await??;
    assert_eq!(tab.sent("Input.dispatchMouseEvent"), before + 1);
    assert!(
        run.env.calls().contains(&EnvCall::MarkSent),
        "input reached the replacement target"
    );
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn target_crashed_protocol_reply_has_a_plain_message_and_preserves_navigation_delivery()
-> TestResult {
    let tab = ScriptedTab::new(CrashedTab::default());
    let run = ScriptedRun::over(&tab).await?;
    let error = crash_reply(&run, &tab).await?;
    assert_plain_crash(&error);
    assert_eq!(
        error.answer(),
        Answer::Definite,
        "remembering a crash preserves the navigation reply's existing delivery classification"
    );
    Ok(())
}

#[test]
fn target_crashed_classification_accepts_only_the_exact_protocol_reply() {
    let crash = CoreError::Cdp(CdpError::Protocol {
        code: -32000,
        message: "Target crashed".to_owned(),
    });
    let code = ErrorCode::from_core(&crash);
    assert_eq!(
        code.as_str(),
        "target_crashed",
        "the exact crash reply needs a typed SDK classification"
    );
    assert_eq!(
        code.answer(),
        Answer::Definite,
        "classification alone does not change delivery; uncertain input is marked at its sending seam"
    );
    for message in [
        "Target crashed during setup",
        "Target crashed?",
        "target crashed",
        "Crash report unavailable",
    ] {
        let error = CoreError::Cdp(CdpError::Protocol {
            code: -32000,
            message: message.to_owned(),
        });
        assert_eq!(
            ErrorCode::from_core(&error),
            ErrorCode::Internal,
            "unrelated protocol messages are not crash evidence: {message}"
        );
    }
}

#[tokio::test(start_paused = true)]
async fn target_crashed_refusal_does_not_wait_for_an_unread_listener() -> TestResult {
    use crate::events::{PageEventKind, Subscription};
    use browseros_core::TargetId;
    let tab = ScriptedTab::new(CrashedTab::default());
    let run = ScriptedRun::over(&tab).await?;
    crash_event(&run, &tab).await?;
    let run = next_run(run);
    let state = run.env.run_state();
    run.playwright.events().bind(state);
    state.events.listen(
        Subscription::Page {
            tab: browseros_core::TabId(7),
            target: TargetId(SCRIPTED_MAIN.to_owned()),
            event: PageEventKind::Crash,
        },
        0,
        0,
    );
    let before = time::Instant::now();
    assert_cached_refusal(&run, &tab, "page.title", json!({"page":run.page})).await?;
    assert!(
        before.elapsed() < REFUSAL_BOUND,
        "a crash failure cannot wait for the independent event pump"
    );
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn target_crashed_sent_reply_keeps_prior_events_before_the_reply() -> TestResult {
    use std::pin::pin;

    use crate::events::{PageEventKind, Subscription};

    let tab = ScriptedTab::new(CrashedTab {
        delay_navigation: true,
        ..CrashedTab::default()
    });
    let run = ScriptedRun::over(&tab).await?;
    settle_watcher(&run).await?;
    let state = run.env.run_state();
    run.playwright.events().bind(state);
    state.events.listen(
        Subscription::Page {
            tab: browseros_core::TabId(7),
            target: TargetId(SCRIPTED_MAIN.to_owned()),
            event: PageEventKind::Crash,
        },
        0,
        0,
    );
    // Model a pump in flight whose consumer has not taken this event yet.
    let _pumping = state.events.pumping();
    tab.script().crash_navigation.store(true, Ordering::SeqCst);
    let mut pending = pin!(run.playwright.invoke(
        &run.env,
        Op::from_name("page.goto").ok_or("page.goto")?,
        json!({ "page": run.page, "url": "https://a.test/next", "waitUntil": "commit" }),
    ));
    tokio::select! {
        _early = &mut pending => return Err("navigation answered before the fixture could publish its crash".into()),
        () = async {
            while tab.sent("Page.navigate") == 0 {
                task::yield_now().await;
            }
        } => {}
    }
    assert!(run.env.calls().contains(&EnvCall::MarkSent));
    crash_event(&run, &tab).await?;
    assert!(
        time::timeout(REFUSAL_BOUND, pending.as_mut())
            .await
            .is_err(),
        "a sent crash reply still waits for its earlier listener event"
    );
    let pumped = run
        .playwright
        .invoke(
            &run.env,
            Op::from_name("events.next").ok_or("events.next")?,
            json!({}),
        )
        .await?;
    assert_eq!(pumped["events"][0]["event"], "crash");
    let error = time::timeout(REFUSAL_BOUND, pending)
        .await?
        .err()
        .ok_or("the navigation still returns its crash error")?;
    assert_eq!(error.error_code(), Some(ErrorCode::TargetCrashed));
    assert!(
        state.events.pump_replies() > 0,
        "the event reply precedes the sent failure"
    );
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn target_crashed_does_not_block_a_second_healthy_target() -> TestResult {
    let tab = ScriptedTab::new(CrashedTab::default());
    let run = ScriptedRun::over(&tab).await?;
    let open = Op::from_name("context.newPage").ok_or("context.newPage")?;
    run.playwright.invoke(&run.env, open, json!({})).await?;
    crash_event(&run, &tab).await?;
    let run = next_run(run);
    let before = tab.sent("Input.dispatchMouseEvent");
    run.playwright
        .invoke(
            &run.env,
            Op::from_name("mouse.move").ok_or("mouse.move")?,
            json!({"page":{"$neo":"page","tab":9}, "x":1, "y":2}),
        )
        .await?;
    assert_eq!(
        tab.sent("Input.dispatchMouseEvent"),
        before + 1,
        "only the addressed native target is refused"
    );
    Ok(())
}

/// A child uses either its own renderer or its parent's endpoint. The same snapshot failure
/// must retain that distinction when its diagnostic leaves the frame's transport boundary.
struct FrameCrashTab {
    independent: bool,
    main_seq: AtomicU64,
    child_reads: AtomicUsize,
}

impl FrameCrashTab {
    fn child_frame(&self) -> &'static str {
        if self.independent {
            SCRIPTED_OOPIF
        } else {
            SCRIPTED_CHILD
        }
    }

    fn iframe_ref(&self) -> String {
        let seq = self.main_seq.load(Ordering::SeqCst);
        if seq == 0 {
            "e1".to_owned()
        } else {
            format!("f{seq}e1")
        }
    }
}

impl TabScript for FrameCrashTab {
    fn child(&self) -> bool {
        !self.independent
    }

    fn oopif(&self) -> bool {
        self.independent
    }

    fn probe(&self, frame: &str, request: &Value) -> ProbeAnswer {
        match request["op"].as_str() {
            Some("snapshot") if frame == self.child_frame() => {
                self.child_reads.fetch_add(1, Ordering::SeqCst);
                ProbeAnswer::error("Target crashed")
            }
            Some("snapshot") => ProbeAnswer::reply(json!({ "kind": "snapshot",
                "json": [{ "role": "iframe", "ref": self.iframe_ref() }],
                "iframes": [{ "ref": self.iframe_ref(), "depth": 1, "src": "https://child.test/" }] })),
            Some("resolve") => ProbeAnswer::reply(json!({ "kind": "frame", "pin": 1,
                "key": self.iframe_ref(), "visible": true, "src": "https://child.test/" })),
            _ => ProbeAnswer::unexpected(),
        }
    }

    fn cdp_reply(&self, method: &str, params: &Value) -> Option<Value> {
        if method == "Runtime.callFunctionOn" {
            let first = &params["arguments"][0]["value"];
            if params["objectId"] == "g"
                && let Some(seq) = first["frameSeq"].as_u64()
            {
                self.main_seq.store(seq, Ordering::SeqCst);
            }
            if first["request"]["op"] == "pinned" {
                return Some(json!({ "result": { "type": "object", "subtype": "node",
                    "objectId": "iframe-owner" } }));
            }
        }
        if method == "DOM.describeNode" && params["objectId"] == "iframe-owner" {
            let mut node = json!({ "backendNodeId": 1, "nodeName": "IFRAME",
                "frameId": self.child_frame() });
            if !self.independent {
                node["contentDocument"] = json!({ "backendNodeId": 2 });
            }
            return Some(json!({ "node": node }));
        }
        None
    }
}

async fn snapshot_child_crash(
    independent: bool,
) -> TestResult<(ScriptedRun, Arc<ScriptedTab<FrameCrashTab>>)> {
    let tab = ScriptedTab::new(FrameCrashTab {
        independent,
        main_seq: AtomicU64::default(),
        child_reads: AtomicUsize::default(),
    });
    let run = ScriptedRun::over(&tab).await?;
    settle_watcher(&run).await?;
    let snapshot = run
        .playwright
        .invoke(
            &run.env,
            Op::from_name("page.ariaSnapshot").ok_or("page.ariaSnapshot")?,
            json!({ "page": run.page, "mode": "ai" }),
        )
        .await?;
    assert_eq!(
        tab.script().child_reads.load(Ordering::SeqCst),
        1,
        "the snapshot must reach the child's crashing endpoint"
    );
    assert_eq!(
        snapshot["partial"][0]["reason"], TARGET_CRASH_MESSAGE,
        "partial output retains the typed crash of the frame it could not read"
    );
    Ok((next_run(run), tab))
}

#[tokio::test(start_paused = true)]
async fn target_crashed_oopif_snapshot_does_not_poison_parent_in_a_fresh_run() -> TestResult {
    let (run, tab) = snapshot_child_crash(true).await?;
    let before = tab.sent("Input.dispatchMouseEvent");
    run.playwright
        .invoke(
            &run.env,
            Op::from_name("mouse.move").ok_or("mouse.move")?,
            json!({ "page": run.page, "x": 1, "y": 2 }),
        )
        .await?;
    assert_eq!(
        tab.sent("Input.dispatchMouseEvent"),
        before + 1,
        "an independent child renderer cannot refuse healthy parent input"
    );
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn target_crashed_same_process_snapshot_is_cached_for_the_parent() -> TestResult {
    let (run, tab) = snapshot_child_crash(false).await?;
    let before = renderer_commands(&tab);
    let error = run
        .playwright
        .invoke(
            &run.env,
            Op::from_name("mouse.move").ok_or("mouse.move")?,
            json!({ "page": run.page, "x": 1, "y": 2 }),
        )
        .await
        .err()
        .ok_or("the child's endpoint is also the parent's crashed renderer")?;
    assert_plain_crash(&error);
    assert_eq!(
        renderer_commands(&tab),
        before,
        "same-process children share their parent's renderer refusal"
    );
    assert!(!run.env.calls().contains(&EnvCall::MarkSent));
    Ok(())
}

/// A host can publish a newer native inventory between SDK crash admission and session
/// acquisition. Polling this immediate fake listing at the view boundary fixes that ordering
/// without scheduling sleeps or a production-only test hook.
struct RebindingEnv<'a> {
    inner: TestEnv,
    browser: &'a BrowserSession,
    tab: &'a ScriptedTab<CrashedTab>,
    views: AtomicUsize,
    rebind_at: usize,
}

impl OpEnv for RebindingEnv<'_> {
    fn view(&self) -> &BrowserView {
        if self.views.fetch_add(1, Ordering::SeqCst) == self.rebind_at {
            self.tab.script().replaced.store(true, Ordering::SeqCst);
            assert!(
                matches!(self.browser.pages.list().now_or_never(), Some(Ok(_))),
                "the fake browser publishes its replacement before returning the view"
            );
        }
        self.inner.view()
    }

    fn run_state(&self) -> &Arc<RunState> {
        self.inner.run_state()
    }

    fn recheck(&self) -> Result<(), Refusal> {
        self.inner.recheck()
    }

    fn mark_sent(&self) {
        self.inner.mark_sent();
    }

    fn note(&self, line: String) {
        self.inner.note(line);
    }
}

#[expect(
    clippy::too_many_lines,
    reason = "one replacement race and a healthy-original follow-up share the same native fixture"
)]
async fn admission_rebind(bound: bool) -> TestResult {
    let tab = ScriptedTab::new(CrashedTab {
        profile_targeting: bound,
        ..CrashedTab::default()
    });
    let connection: Arc<dyn CdpConnection> = Arc::clone(&tab) as Arc<dyn CdpConnection>;
    let browser = BrowserSession::new_with_native(
        connection,
        BrowserSessionHooks::default(),
        NativeCapabilities::from_browseros_version(Some("0.51.1.3")),
    );
    let view = if bound {
        BrowserView::capture(
            Arc::clone(&browser),
            Some(TargetBinding {
                intent: TargetIntent {
                    browser_profile_id: BrowserProfileId("test".to_owned()),
                    directory: "Test".to_owned(),
                    name: "Test".to_owned(),
                    window: None,
                },
                revision: 1,
            }),
            None,
        )
        .await
    } else {
        BrowserView::unbound(Arc::clone(&browser), None)
    };
    assert_eq!(
        view.scope()?.is_some(),
        bound,
        "both core admission paths are exercised"
    );
    let playwright = Playwright::new(Arc::clone(&browser));
    let listed = browser.pages.list().await?;
    let page = listed.first().ok_or("the original tab is listed")?;
    let tab_id = page.tab_id;
    let run = ScriptedRun {
        playwright,
        env: TestEnv::over(view),
        page: serde_json::from_value(json!({ "$neo": "page", "tab": tab_id }))?,
    };
    settle_watcher(&run).await?;
    run.playwright
        .crashes()
        .crashed_event(TargetId(REPLACEMENT.to_owned()), EventFence(0));
    tab.script().crash_preparation.store(true, Ordering::SeqCst);
    let before = renderer_commands(&tab);
    let env = RebindingEnv {
        inner: TestEnv::over(run.env.view().clone()),
        browser: &browser,
        tab: &tab,
        views: AtomicUsize::default(),
        // Publish B after the SDK read A, just before Core acquires the native session.
        rebind_at: 0,
    };
    let scope = OpScope::new(
        &env,
        Op::from_name("mouse.move").ok_or("mouse.move")?,
        CallHeader::default(),
    )
    .tracking(run.playwright.crashes(), &browser);
    let refused = scope.observe_page(&browser, tab_id).await;
    assert!(
        tab.script().replaced.load(Ordering::SeqCst),
        "the replacement race was exercised"
    );
    assert!(
        refused.is_err(),
        "the original target cannot authorize its replacement"
    );
    assert_eq!(
        renderer_commands(&tab),
        before,
        "a cached-crashed replacement receives no attachment or renderer setup"
    );
    assert!(
        !run.playwright
            .crashes()
            .is_crashed(&TargetId(SCRIPTED_MAIN.to_owned())),
        "a replacement's failure cannot poison the original target's ticket"
    );
    assert!(
        run.playwright
            .crashes()
            .is_crashed(&TargetId(REPLACEMENT.to_owned()))
    );
    assert!(!env.inner.calls().contains(&EnvCall::MarkSent));
    drop(scope);

    tab.script()
        .crash_preparation
        .store(false, Ordering::SeqCst);
    tab.script().replaced.store(false, Ordering::SeqCst);
    browser.pages.list().await?;
    let run = next_run(run);
    let before = tab.sent("Input.dispatchMouseEvent");
    run.playwright
        .invoke(
            &run.env,
            Op::from_name("mouse.move").ok_or("mouse.move")?,
            json!({ "page": run.page, "x": 1, "y": 2 }),
        )
        .await?;
    assert_eq!(
        tab.sent("Input.dispatchMouseEvent"),
        before + 1,
        "a fresh run can still use the healthy original target"
    );
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn target_crashed_rebind_refuses_before_preparation_on_unbound_views() -> TestResult {
    admission_rebind(false).await
}

#[tokio::test(start_paused = true)]
async fn target_crashed_rebind_refuses_before_preparation_on_bound_views() -> TestResult {
    admission_rebind(true).await
}

#[tokio::test(start_paused = true)]
async fn target_crashed_root_input_reply_is_cached_in_a_fresh_run() -> TestResult {
    let tab = ScriptedTab::new(CrashedTab::default());
    let run = ScriptedRun::over(&tab).await?;
    settle_watcher(&run).await?;
    tab.script().crash_input.store(true, Ordering::SeqCst);
    let input = json!({ "page": run.page, "x": 1, "y": 2 });
    let error = run
        .playwright
        .invoke(
            &run.env,
            Op::from_name("mouse.move").ok_or("mouse.move")?,
            input.clone(),
        )
        .await
        .err()
        .ok_or("the root renderer refused the dispatched input")?;
    assert_plain_crash(&error);
    assert!(run.env.calls().contains(&EnvCall::MarkSent));
    assert_eq!(
        error.answer(),
        Answer::Ambiguous,
        "the input may have reached the renderer"
    );
    tab.script().renderer_silent.store(true, Ordering::SeqCst);
    assert_cached_refusal(&next_run(run), &tab, "mouse.move", input).await
}

#[tokio::test(start_paused = true)]
async fn target_crashed_healthy_unbound_view_follows_a_reconnected_transport() -> TestResult {
    let tab = ScriptedTab::new(CrashedTab::default());
    let run = ScriptedRun::over(&tab).await?;
    settle_watcher(&run).await?;
    tab.reconnect();
    let before = tab.sent("Input.dispatchMouseEvent");
    run.playwright
        .invoke(
            &run.env,
            Op::from_name("mouse.move").ok_or("mouse.move")?,
            json!({ "page": run.page, "x": 1, "y": 2 }),
        )
        .await?;
    assert_eq!(
        tab.sent("Input.dispatchMouseEvent"),
        before + 1,
        "ordinary unbound admission follows the live socket instead of the view's capture epoch"
    );
    Ok(())
}

/// Creation receipts retain the original placement. A later ordinary page operation follows
/// allowed owner moves within the captured profile, so it cannot reuse creation pinning as
/// its general native-target admission contract.
#[derive(Default)]
struct CreatedMoveTab {
    created: AtomicBool,
    moved: AtomicBool,
}

impl CreatedMoveTab {
    fn tab(&self) -> Value {
        let window = if self.moved.load(Ordering::SeqCst) {
            2
        } else {
            1
        };
        json!({ "tabId": 9, "targetId": SCRIPTED_NEW, "windowId": window,
            "url": "about:blank", "title": "", "isActive": false, "isLoading": false,
            "loadProgress": 1.0, "isPinned": false, "isHidden": false,
            "browserProfileId": "test", "liveBrowserContextId": "live-test" })
    }
}

impl TabScript for CreatedMoveTab {
    fn probe(&self, _frame: &str, _request: &Value) -> ProbeAnswer {
        ProbeAnswer::unexpected()
    }

    fn cdp_reply(&self, method: &str, params: &Value) -> Option<Value> {
        match method {
            "Browser.createTab" => {
                self.created.store(true, Ordering::SeqCst);
                Some(json!({ "tab": self.tab() }))
            }
            "Browser.getTabs" => Some(json!({ "tabs": if self.created.load(Ordering::SeqCst) {
                vec![self.tab()]
            } else { Vec::new() } })),
            "Browser.getTabInfo" => Some(json!({ "tab": self.tab() })),
            "Browser.getWindows" => {
                let windows = [1, 2].map(|window| {
                    json!({ "windowId": window,
                    "windowType": "normal", "bounds": {},
                    "isActive": true, "isVisible": true, "tabCount": 1,
                    "browserProfileId": "test", "liveBrowserContextId": "live-test" })
                });
                Some(json!({ "windows": windows }))
            }
            _ => CrashedTab::default().cdp_reply(method, params),
        }
    }
}

#[tokio::test(start_paused = true)]
async fn target_crashed_general_admission_cannot_reuse_a_creation_window_receipt() -> TestResult {
    let tab = ScriptedTab::new(CreatedMoveTab::default());
    let connection: Arc<dyn CdpConnection> = Arc::clone(&tab) as Arc<dyn CdpConnection>;
    let browser = BrowserSession::new_with_native(
        connection,
        BrowserSessionHooks::default(),
        NativeCapabilities::from_browseros_version(Some("0.51.1.3")),
    );
    let view = BrowserView::capture(
        Arc::clone(&browser),
        Some(TargetBinding {
            intent: TargetIntent {
                browser_profile_id: BrowserProfileId("test".to_owned()),
                directory: "Test".to_owned(),
                name: "Test".to_owned(),
                window: None,
            },
            revision: 1,
        }),
        None,
    )
    .await;
    let created = view
        .new_page("about:blank", NewPageOptions::default())
        .await?;
    tab.script().moved.store(true, Ordering::SeqCst);
    let moved = view.page(created.page_id.clone()).await?;
    assert_eq!(
        moved.target_id, created.target_id,
        "the renderer did not change"
    );
    assert_ne!(
        moved.window_id, created.window_id,
        "the owner moved the tab within the profile"
    );
    let ordinary = view.page_session(created.page_id.clone()).await?;
    assert_eq!(
        ordinary.target_id, created.target_id,
        "the unpinned profile still admits the healthy moved tab"
    );
    assert!(
        view.with_expected_created_page(&moved).is_err(),
        "the historical creation window makes this API unsuitable for ordinary target pinning"
    );
    Ok(())
}
