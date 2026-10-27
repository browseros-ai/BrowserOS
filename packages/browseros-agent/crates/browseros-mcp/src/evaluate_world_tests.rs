//! Which JavaScript world `evaluate`, `evaluate_in_page` and their `run` SDK twins send the
//! agent's script to, against a one-tab fake browser. World selection itself is covered in
//! `browseros_core::page_js`.
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
const ISOLATED_CONTEXT: i64 = 77;

struct Native {
    refuse_world: bool,
    evaluations: Mutex<Vec<Value>>,
    events: broadcast::Sender<CdpEvent>,
}

impl Native {
    fn new(refuse_world: bool) -> Arc<Self> {
        let (events, _) = broadcast::channel(8);
        Arc::new(Self {
            refuse_world,
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
            "Page.createIsolatedWorld" if self.refuse_world => {
                return Err(CdpError::Protocol {
                    code: -32000,
                    message: "Could not create isolated world".to_string(),
                });
            }
            "Page.createIsolatedWorld" => json!({"executionContextId": ISOLATED_CONTEXT}),
            "Runtime.evaluate" => {
                if let Ok(mut evaluations) = self.evaluations.lock() {
                    evaluations.push(params.clone());
                }
                let is_script = params["expression"]
                    .as_str()
                    .is_some_and(|expression| expression.contains(SCRIPT));
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
async fn tests_that_evaluate_runs_the_script_in_the_isolated_world() -> TestResult {
    let native = Native::new(false);
    let result = call(&ctx(&native), tools::evaluate::definition(), script_args()).await;

    assert!(!result.is_error, "{}", text(&result));
    assert!(text(&result).contains("42"), "{}", text(&result));
    let scripts = native.scripts();
    assert_eq!(scripts.len(), 1, "{scripts:?}");
    assert_eq!(scripts[0]["contextId"], json!(ISOLATED_CONTEXT));
    assert_eq!(scripts[0]["userGesture"], json!(true));
    Ok(())
}

#[tokio::test]
async fn tests_that_evaluate_never_falls_back_to_the_page_world() -> TestResult {
    let native = Native::new(true);
    let result = call(&ctx(&native), tools::evaluate::definition(), script_args()).await;

    assert!(result.is_error, "{}", text(&result));
    assert!(
        text(&result).contains("evaluate_in_page"),
        "{}",
        text(&result)
    );
    assert!(native.scripts().is_empty(), "{:?}", native.scripts());
    Ok(())
}

#[tokio::test]
async fn tests_that_evaluate_in_page_runs_the_script_in_the_main_world() -> TestResult {
    let native = Native::new(false);
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
async fn tests_that_run_sends_each_sdk_script_to_its_world() -> TestResult {
    let native = Native::new(false);
    let code = format!(
        "const page = browser.page(1);
         const isolated = await page.evaluate(() => {SCRIPT});
         const inPage = await page.evaluateInPage(() => {SCRIPT});
         const direct = await browser.evaluateInPage(1, {{ code: 'return {SCRIPT};' }});
         return [isolated.value, inPage.value, direct.value];"
    );
    let result = call(
        &ctx(&native),
        tools::run::definition(),
        json!({"code": code, "timeout": 5000}),
    )
    .await;

    assert!(!result.is_error, "{}", text(&result));
    let contexts = native
        .scripts()
        .iter()
        .map(|params| params.get("contextId").cloned())
        .collect::<Vec<_>>();
    assert_eq!(contexts, vec![Some(json!(ISOLATED_CONTEXT)), None, None]);
    Ok(())
}
