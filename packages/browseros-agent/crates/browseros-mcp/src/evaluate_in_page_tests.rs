//! Where `evaluate_in_page` and its `run` SDK routes send the agent's script, against a one-tab
//! fake browser, and what an old helper calling the removed `page.evaluate` gets back. World
//! selection itself is covered in `browseros_core::page_js`.
use crate::{
    framework::{BrowserToolDefaults, BrowserToolOptions, ToolCtx, ToolResult, execute_tool},
    output_file::create_browser_output_file_access,
    tools,
};
use browseros_cdp::{CdpError, CdpEvent};
use browseros_core::{BrowserSession, BrowserSessionHooks, BrowserView, CdpConnection, SessionId};
use futures_util::future::BoxFuture;
use serde_json::{Value, json};
use std::{
    error::Error,
    sync::{Arc, Mutex},
};
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;

type TestResult = Result<(), Box<dyn Error>>;

/// Appears only in the agent's script, so the evaluation carrying it is easy to find among the
/// readiness probes the tools also send.
const SCRIPT: &str = "40 + 2";
/// A script whose result is longer than the inline limit.
const LONG_RESULT: &str = "'x'.repeat(6000)";

struct Native {
    evaluations: Mutex<Vec<Value>>,
    events: broadcast::Sender<CdpEvent>,
}

impl Native {
    fn new() -> Arc<Self> {
        let (events, _) = broadcast::channel(8);
        Arc::new(Self {
            evaluations: Mutex::new(Vec::new()),
            events,
        })
    }

    /// The `Runtime.evaluate` params that carried the agent's script, in order.
    fn scripts(&self) -> Vec<Value> {
        self.evaluations
            .lock()
            .map(|evaluations| {
                evaluations
                    .iter()
                    .filter(|params| {
                        params["expression"]
                            .as_str()
                            .is_some_and(|expression| expression.contains(SCRIPT))
                    })
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    fn reply(&self, method: &str, params: &Value) -> Result<Value, CdpError> {
        let tab = json!({"tabId":1,"targetId":"target-1","windowId":7,"url":"https://site.test/",
            "title":"t","isActive":true,"isLoading":false,"loadProgress":1.0,"isPinned":false,
            "isHidden":false});
        Ok(match method {
            "Browser.getTabs" => json!({"tabs": [tab]}),
            "Browser.getTabInfo" => json!({"tab": tab}),
            "Target.attachToTarget" => json!({"sessionId": "session-1"}),
            "Browser.getVersion" => json!({"product": "Chrome/155.0.8315.26"}),
            "Page.getFrameTree" => json!({"frameTree": {"frame": {"id": "target-1"}}}),
            // Readiness probes run in BrowserOS's isolated world.
            "Page.createIsolatedWorld" => json!({"executionContextId": 77}),
            "Runtime.evaluate" => {
                if let Ok(mut evaluations) = self.evaluations.lock() {
                    evaluations.push(params.clone());
                }
                let is_script = params["expression"]
                    .as_str()
                    .is_some_and(|expression| expression.contains(SCRIPT));
                if params["expression"]
                    .as_str()
                    .is_some_and(|expression| expression.contains(LONG_RESULT))
                {
                    return Ok(json!({"result": {"type": "string", "value": "x".repeat(6000)}}));
                }
                if is_script {
                    json!({"result": {"type": "number", "value": 42}})
                } else {
                    json!({"result": {"type": "undefined"}})
                }
            }
            _ => json!({}),
        })
    }
}

impl CdpConnection for Native {
    fn send<'a>(
        &'a self,
        method: &'a str,
        params: Value,
        _session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<Value, CdpError>> {
        Box::pin(async move { self.reply(method, &params) })
    }

    fn send_raw_json<'a>(
        &'a self,
        method: &'a str,
        params_json: &'a str,
        _session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<String, CdpError>> {
        Box::pin(async move {
            let value = self.reply(method, &serde_json::from_str(params_json)?)?;
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

fn ctx(native: &Arc<Native>) -> ToolCtx {
    let session = BrowserSession::new(native.clone(), BrowserSessionHooks::default());
    ToolCtx::new(BrowserToolOptions {
        browser: BrowserView::unbound(session, None),
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

fn script_args() -> Value {
    json!({"page": 1, "code": format!("return {SCRIPT};")})
}

#[tokio::test]
async fn tests_that_evaluate_in_page_runs_the_script_in_the_main_world() -> TestResult {
    let native = Native::new();
    let result = call(
        &ctx(&native),
        tools::evaluate_in_page::definition(),
        script_args(),
    )
    .await;

    assert!(!result.is_error, "{}", text(&result));
    assert!(text(&result).contains("42"), "{}", text(&result));
    let scripts = native.scripts();
    assert_eq!(scripts.len(), 1, "{scripts:?}");
    assert!(scripts[0].get("contextId").is_none(), "{scripts:?}");
    assert_eq!(scripts[0]["userGesture"], json!(true));
    Ok(())
}

#[tokio::test]
async fn tests_that_run_sends_both_sdk_routes_to_the_main_world() -> TestResult {
    let native = Native::new();
    let code = format!(
        "const viaPage = await browser.page(1).evaluateInPage(() => {SCRIPT});
         const viaBrowser = await browser.evaluateInPage(1, {{ code: 'return {SCRIPT};' }});
         return [viaPage.value, viaBrowser.value];"
    );
    let result = call(
        &ctx(&native),
        tools::run::definition(),
        json!({"code": code, "timeout": 5000}),
    )
    .await;

    assert!(!result.is_error, "{}", text(&result));
    let scripts = native.scripts();
    assert_eq!(scripts.len(), 2, "{scripts:?}");
    assert!(
        scripts
            .iter()
            .all(|params| params.get("contextId").is_none()),
        "{scripts:?}"
    );
    Ok(())
}

#[tokio::test]
async fn tests_that_an_old_page_evaluate_call_is_pointed_at_evaluate_in_page() -> TestResult {
    let native = Native::new();
    let code = format!("return await browser.page(1).evaluate(() => {SCRIPT});");
    let result = call(
        &ctx(&native),
        tools::run::definition(),
        json!({"code": code, "timeout": 5000}),
    )
    .await;

    assert!(result.is_error, "{}", text(&result));
    assert!(
        text(&result).contains("evaluateInPage"),
        "{}",
        text(&result)
    );
    assert!(native.scripts().is_empty(), "{:?}", native.scripts());
    Ok(())
}

/// A run script reads the value itself, so it gets all of it rather than an excerpt plus a
/// file path it has no way to open.
#[tokio::test]
async fn tests_that_a_run_script_gets_a_long_result_whole() -> TestResult {
    let native = Native::new();
    let direct = call(
        &ctx(&native),
        tools::evaluate_in_page::definition(),
        json!({"page": 1, "code": format!("return {LONG_RESULT};")}),
    )
    .await;
    assert!(text(&direct).contains("truncated"), "{}", text(&direct));

    let code = format!(
        "const result = await browser.page(1).evaluateInPage(\"return {LONG_RESULT};\");
         return result.value.length;"
    );
    let result = call(
        &ctx(&native),
        tools::run::definition(),
        json!({"code": code, "timeout": 5000}),
    )
    .await;
    assert!(!result.is_error, "{}", text(&result));
    assert_eq!(
        result
            .structured_content
            .as_ref()
            .and_then(|data| data.get("value")),
        Some(&json!(6000))
    );
    Ok(())
}
