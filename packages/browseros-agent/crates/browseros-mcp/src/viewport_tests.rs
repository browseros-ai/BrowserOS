//! `setViewport` and viewport restoration through the tool framework and the run SDK, against
//! a one-tab fake browser. Viewport mechanics are covered in `browseros_core::viewport`.
use crate::{
    framework::{
        BrowserToolDefaults, BrowserToolOptions, ToolCtx, ToolError, ToolResult, execute_tool,
    },
    output_file::create_browser_output_file_access,
    tools,
};
use browseros_cdp::{CdpError, CdpEvent};
use browseros_core::{
    BrowserSession, BrowserSessionHooks, BrowserView, CdpConnection, PageId, SessionId,
    viewport::Viewports,
};
use futures_util::future::BoxFuture;
use serde_json::{Value, json};
use std::{
    error::Error,
    sync::{Arc, Mutex, MutexGuard},
    time::Duration,
};
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;

type TestResult = Result<(), Box<dyn Error>>;

const CHROME_HEIGHT: i64 = 121;

struct State {
    calls: Vec<String>,
    width: i64,
    height: i64,
    metrics: Option<Value>,
    touch: bool,
    user_agent: String,
    closed: bool,
}

struct Native {
    state: Mutex<State>,
    events: broadcast::Sender<CdpEvent>,
}

impl Native {
    fn new() -> Arc<Self> {
        let (events, _) = broadcast::channel(8);
        Arc::new(Self {
            state: Mutex::new(State {
                calls: Vec::new(),
                width: 1200,
                height: 900,
                metrics: None,
                touch: false,
                user_agent: String::new(),
                closed: false,
            }),
            events,
        })
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(|err| panic!("fake poisoned: {err}"))
    }

    fn calls(&self) -> Vec<String> {
        self.lock().calls.clone()
    }

    fn clear(&self) {
        self.lock().calls.clear();
    }

    fn resize(&self, width: i64, height: i64) {
        let mut state = self.lock();
        state.width = width;
        state.height = height;
    }

    fn window(&self) -> (i64, i64) {
        let state = self.lock();
        (state.width, state.height)
    }

    fn position(&self, method: &str) -> Option<usize> {
        self.calls().iter().position(|call| call == method)
    }

    /// Calls that read or change the window; setViewport and page tools send none.
    fn window_calls(&self) -> Vec<String> {
        self.calls()
            .into_iter()
            .filter(|call| {
                matches!(
                    call.as_str(),
                    "Browser.getWindowForTarget"
                        | "Browser.getWindowBounds"
                        | "Browser.setWindowBounds"
                        | "Browser.setContentsSize"
                        | "Emulation.getScreenInfos"
                )
            })
            .collect()
    }

    fn reply(&self, method: &str, params: &Value) -> Value {
        let mut state = self.lock();
        state.calls.push(method.to_string());
        let tab = json!({"tabId":1,"targetId":"target-1","windowId":7,"url":"https://site.test/",
            "title":"t","isActive":true,"isLoading":false,"loadProgress":1.0,"isPinned":false,
            "isHidden":false});
        match method {
            "Browser.getTabs" if state.closed => json!({"tabs": []}),
            "Browser.getTabs" => json!({"tabs": [tab]}),
            "Browser.getTabInfo" => json!({"tab": tab}),
            "Browser.closeTab" => {
                state.closed = true;
                json!({})
            }
            "Target.attachToTarget" => json!({"sessionId": "session-1"}),
            "Browser.getVersion" => json!({"product": "Chrome/155.0.8315.26"}),
            "Emulation.setDeviceMetricsOverride" => {
                state.metrics = Some(params.clone());
                json!({})
            }
            "Emulation.clearDeviceMetricsOverride" => {
                state.metrics = None;
                json!({})
            }
            "Emulation.setTouchEmulationEnabled" => {
                state.touch = params["enabled"] == true;
                json!({})
            }
            "Emulation.setUserAgentOverride" => {
                state.user_agent = params["userAgent"].as_str().unwrap_or_default().to_string();
                json!({})
            }
            "Runtime.evaluate"
                if params["expression"]
                    .as_str()
                    .is_some_and(|expression| expression.contains("screenWidth")) =>
            {
                let value = match &state.metrics {
                    Some(metrics) => json!({"width": metrics["width"], "height": metrics["height"],
                        "screenWidth": metrics["screenWidth"], "screenHeight": metrics["screenHeight"],
                        "deviceScaleFactor": metrics["deviceScaleFactor"],
                        "touchPoints": if state.touch { 5 } else { 0 },
                        "mobile": state.user_agent.contains("Mobile")}),
                    None => json!({"width": state.width, "height": state.height - CHROME_HEIGHT,
                        "screenWidth": 3000, "screenHeight": 3000, "deviceScaleFactor": 2,
                        "touchPoints": 0, "mobile": false}),
                };
                json!({"result": {"type": "object", "value": value}})
            }
            "Runtime.evaluate" | "Runtime.callFunctionOn" => {
                json!({"result": {"type": "number", "value": 2}})
            }
            "Page.getLayoutMetrics" => {
                let viewport =
                    json!({"pageX": 0, "pageY": 0, "clientWidth": 400, "clientHeight": 906});
                json!({"layoutViewport": viewport, "cssLayoutViewport": viewport})
            }
            "Page.captureScreenshot" => json!({"data": "aGk="}),
            // The viewport check reads the page in BrowserOS's isolated world.
            "Page.getFrameTree" => json!({"frameTree": {"frame": {"id": "target-1"}}}),
            "Page.createIsolatedWorld" => json!({"executionContextId": 1}),
            _ => json!({}),
        }
    }
}

impl CdpConnection for Native {
    fn send<'a>(
        &'a self,
        method: &'a str,
        params: Value,
        _session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<Value, CdpError>> {
        Box::pin(async move { Ok(self.reply(method, &params)) })
    }

    fn send_raw_json<'a>(
        &'a self,
        method: &'a str,
        params_json: &'a str,
        _session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<String, CdpError>> {
        Box::pin(async move {
            let value = self.reply(method, &serde_json::from_str(params_json)?);
            serde_json::to_string(&value).map_err(CdpError::from)
        })
    }

    fn events(&self) -> broadcast::Receiver<CdpEvent> {
        self.events.subscribe()
    }

    fn is_connected(&self) -> bool {
        true
    }

    fn connection_epoch(&self) -> u64 {
        1
    }
}

fn ctx(native: &Arc<Native>, hosted: bool) -> ToolCtx {
    let session = BrowserSession::new(native.clone(), BrowserSessionHooks::default());
    let mut browser = BrowserView::unbound(session, None);
    if hosted {
        browser = browser.with_viewports(Arc::new(Viewports::default()));
    }
    ToolCtx::new(BrowserToolOptions {
        browser,
        target: None,
        defaults: BrowserToolDefaults::default(),
        cancel: CancellationToken::new(),
        output_files: create_browser_output_file_access(),
        inner_call_hook: None,
        preloaded_helpers: Vec::new(),
    })
}

async fn call(ctx: &ToolCtx, def: crate::framework::ToolDef, args: Value) -> ToolResult {
    execute_tool(&def, args, ctx)
        .await
        .unwrap_or_else(|err| ToolResult::error(err.to_string()))
}

async fn set_viewport(ctx: &ToolCtx, args: Value) -> ToolResult {
    call(ctx, tools::viewport::definition(), args).await
}

async fn run(ctx: &ToolCtx, code: &str) -> ToolResult {
    call(
        ctx,
        tools::run::definition(),
        json!({"code": code, "timeout": 5000}),
    )
    .await
}

fn text(result: &ToolResult) -> String {
    result
        .content
        .iter()
        .filter_map(|block| match block {
            rmcp::model::ContentBlock::Text(text) => Some(text.text.clone()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn run_value(result: &ToolResult) -> Result<Value, Box<dyn Error>> {
    if result.is_error {
        return Err(format!("run failed: {}", text(result)).into());
    }
    Ok(result
        .structured_content
        .as_ref()
        .and_then(|value| value.get("value"))
        .cloned()
        .ok_or("run returned no value")?)
}

#[tokio::test]
async fn tests_that_set_viewport_applies_mobile_and_reports_it() -> TestResult {
    let native = Native::new();
    let ctx = ctx(&native, true);
    let result = set_viewport(&ctx, json!({"page": 1, "mode": "mobile", "zoom": 150})).await;

    assert!(!result.is_error, "{}", text(&result));
    let report = result.structured_content.clone().ok_or("no report")?;
    assert_eq!(report["mode"], "mobile");
    assert_eq!(report["zoom"], 150.0);
    assert_eq!(report["viewport"]["width"], 400.0);
    let shown = text(&result);
    assert!(
        shown.contains("mobile") && shown.contains("150%"),
        "{shown}"
    );
    Ok(())
}

#[tokio::test]
async fn tests_that_set_viewport_rejects_invalid_arguments() -> TestResult {
    let native = Native::new();
    let ctx = ctx(&native, true);
    for args in [
        json!({"page": 1, "mode": "mobile", "zoom": 500}),
        json!({"page": 1, "mode": "desktop", "zoom": 100}),
        json!({"page": 1, "mode": "tablet"}),
        json!({"mode": "mobile"}),
        json!({"page": 1, "mode": "mobile", "width": 390}),
        json!({"page": 1, "mode": "mobile", "width": 50, "height": 844}),
        json!({"page": 1, "mode": "default", "width": 390, "height": 844}),
    ] {
        let result = set_viewport(&ctx, args.clone()).await;
        assert!(result.is_error, "{args}");
        assert!(
            text(&result).starts_with("Invalid arguments for setViewport"),
            "{args}: {}",
            text(&result)
        );
    }
    assert!(
        !native
            .calls()
            .iter()
            .any(|call| call.starts_with("Emulation."))
    );
    Ok(())
}

#[tokio::test]
async fn tests_that_set_viewport_is_refused_without_hosted_viewports() -> TestResult {
    let native = Native::new();
    let ctx = ctx(&native, false);
    let result = set_viewport(&ctx, json!({"page": 1, "mode": "mobile"})).await;

    assert!(result.is_error);
    assert!(text(&result).contains("hosted"), "{}", text(&result));
    assert!(
        !native
            .calls()
            .iter()
            .any(|call| call.starts_with("Emulation."))
    );
    Ok(())
}

/// A caller's raw call that replaces the tab's mode; the tab's next operation puts it back.
const RAW_CLEAR: &str =
    "await browser.page(1).cdp('Emulation.clearDeviceMetricsOverride', '{}'); return 1";

#[tokio::test]
async fn tests_that_page_tools_put_back_a_lost_mode_and_leave_the_window() -> TestResult {
    let native = Native::new();
    let ctx = ctx(&native, true);
    set_viewport(&ctx, json!({"page": 1, "mode": "mobile"})).await;
    run_value(&run(&ctx, RAW_CLEAR).await)?;
    native.resize(900, 700);
    native.clear();

    let result = call(
        &ctx,
        tools::screenshot::definition(),
        json!({"page": 1, "format": "png"}),
    )
    .await;
    assert!(!result.is_error, "{}", text(&result));
    let restored = native
        .position("Emulation.setDeviceMetricsOverride")
        .ok_or("mode not put back")?;
    let capture = native
        .position("Page.captureScreenshot")
        .ok_or("no capture")?;
    assert!(restored < capture, "{:?}", native.calls());
    assert!(native.window_calls().is_empty(), "{:?}", native.calls());
    assert_eq!(native.window(), (900, 700));

    // While the mode holds, the next tool sends nothing before its own work.
    native.clear();
    let result = call(
        &ctx,
        tools::screenshot::definition(),
        json!({"page": 1, "format": "png"}),
    )
    .await;
    assert!(!result.is_error, "{}", text(&result));
    assert!(
        !native
            .calls()
            .iter()
            .any(|call| call.starts_with("Emulation.")),
        "{:?}",
        native.calls()
    );
    Ok(())
}

#[tokio::test]
async fn tests_that_tab_management_leaves_the_window_alone() -> TestResult {
    let native = Native::new();
    let ctx = ctx(&native, true);
    set_viewport(&ctx, json!({"page": 1, "mode": "mobile"})).await;
    native.resize(900, 700);
    native.clear();

    let result = call(
        &ctx,
        tools::tabs::definition(),
        json!({"action": "close", "page": 1}),
    )
    .await;
    assert!(!result.is_error, "{}", text(&result));
    assert!(native.window_calls().is_empty(), "{:?}", native.calls());
    assert_eq!(native.window(), (900, 700));
    Ok(())
}

#[tokio::test]
async fn tests_that_run_offers_set_viewport_only_when_hosted() -> TestResult {
    let native = Native::new();
    let hosted = ctx(&native, true);
    assert_eq!(
        run_value(&run(&hosted, "return typeof browser.page(1).setViewport").await)?,
        "function"
    );
    let hint = run_value(
        &run(
            &hosted,
            "try { browser.page(1).setViewportSize({ width: 390, height: 844 }); return 'present' } catch (e) { return String(e.message) }",
        )
        .await,
    )?;
    assert!(
        hint.as_str()
            .is_some_and(|message| message.contains("width?, height?")),
        "{hint}"
    );
    let desktop = ctx(&native, false);
    let missing = run_value(
        &run(
            &desktop,
            "try { browser.page(1).setViewport; return 'present' } catch (e) { return String(e.message) }",
        )
        .await,
    )?;
    assert!(
        missing
            .as_str()
            .is_some_and(|message| message.contains("not a method")),
        "{missing}"
    );
    Ok(())
}

#[tokio::test]
async fn tests_that_run_set_viewport_returns_the_report() -> TestResult {
    let native = Native::new();
    let ctx = ctx(&native, true);
    let report = run_value(
        &run(
            &ctx,
            "return await browser.page(1).setViewport({ mode: 'mobile', zoom: 50 })",
        )
        .await,
    )?;
    assert_eq!(report["mode"], "mobile");
    assert_eq!(report["zoom"], 50.0);
    assert_eq!(report["viewport"]["width"], 400.0);

    let refused = run(
        &ctx,
        "return await browser.page(1).setViewport({ mode: 'desktop', zoom: 50 })",
    )
    .await;
    assert!(refused.is_error);
    assert!(text(&refused).contains("zoom"), "{}", text(&refused));
    Ok(())
}

#[tokio::test]
async fn tests_that_run_page_primitives_put_back_a_lost_mode_first() -> TestResult {
    let native = Native::new();
    let ctx = ctx(&native, true);
    set_viewport(&ctx, json!({"page": 1, "mode": "mobile"})).await;
    run_value(&run(&ctx, RAW_CLEAR).await)?;
    native.resize(900, 700);
    native.clear();

    run_value(&run(&ctx, "await browser.page(1).type('x'); return 1").await)?;
    let restored = native
        .position("Emulation.setDeviceMetricsOverride")
        .ok_or("mode not put back")?;
    let key = native
        .position("Input.dispatchKeyEvent")
        .ok_or("no key event")?;
    assert!(restored < key, "{:?}", native.calls());
    assert!(native.window_calls().is_empty(), "{:?}", native.calls());
    assert_eq!(native.window(), (900, 700));
    Ok(())
}

#[tokio::test]
async fn tests_that_run_tool_primitives_on_fixed_tabs_do_not_wait_on_themselves() -> TestResult {
    let native = Native::new();
    let ctx = ctx(&native, true);
    set_viewport(&ctx, json!({"page": 1, "mode": "desktop"})).await;
    native.resize(900, 700);

    let value = run_value(
        &run(
            &ctx,
            "const shot = await browser.page(1).screenshot({ format: 'png' }); return shot.page",
        )
        .await,
    )?;
    assert_eq!(value, 1);
    assert_eq!(native.window(), (900, 700));
    Ok(())
}

#[tokio::test]
async fn tests_that_set_viewport_takes_an_optional_device_size() -> TestResult {
    let native = Native::new();
    let ctx = ctx(&native, true);
    let result = set_viewport(
        &ctx,
        json!({"page": 1, "mode": "mobile", "width": 390, "height": 844}),
    )
    .await;
    assert!(!result.is_error, "{}", text(&result));
    let report = result.structured_content.clone().ok_or("no report")?;
    assert_eq!(report["size"], json!({"width": 390, "height": 844}));
    assert_eq!(report["viewport"]["width"], 390.0);
    assert!(text(&result).contains("390x844"), "{}", text(&result));

    let scripted = run_value(
        &run(
            &ctx,
            "return await browser.page(1).setViewport({ mode: 'desktop', width: 1440, height: 900 })",
        )
        .await,
    )?;
    assert_eq!(scripted["size"], json!({"width": 1440, "height": 900}));
    assert_eq!(scripted["viewport"]["deviceScaleFactor"], 1.0);
    Ok(())
}

#[tokio::test]
async fn tests_that_a_cancelled_set_viewport_waiting_for_its_tab_changes_nothing() -> TestResult {
    let native = Native::new();
    let ctx = ctx(&native, true);
    set_viewport(&ctx, json!({"page": 1, "mode": "mobile"})).await;
    // Another operation on the tab holds it.
    let held = ctx.browser.prepare_viewport(PageId(1)).await?;
    native.clear();

    let def = tools::viewport::definition();
    let mut change = Box::pin(execute_tool(
        &def,
        json!({"page": 1, "mode": "desktop"}),
        &ctx,
    ));
    assert!(
        tokio::time::timeout(Duration::from_millis(300), change.as_mut())
            .await
            .is_err()
    );
    ctx.cancel.cancel();
    let outcome = tokio::time::timeout(Duration::from_secs(2), change).await?;
    assert!(matches!(outcome, Err(ToolError::Cancelled)), "{outcome:?}");
    drop(held);
    assert!(
        !native
            .calls()
            .iter()
            .any(|call| call.starts_with("Emulation.")),
        "{:?}",
        native.calls()
    );
    Ok(())
}
