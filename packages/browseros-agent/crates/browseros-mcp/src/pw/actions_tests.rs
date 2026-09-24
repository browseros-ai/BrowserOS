//! Contract tests run through the real QuickJS bridge so the audit hook ordering
//! is verified at the same seam used by the host, with CDP supplied by a fake.

use super::actions::{redact_args, sensitive};
use crate::{
    framework::{InnerCallHook, InnerCallRecord, ToolCtx},
    tools::run::{
        ScriptSpec, execute_script,
        tests::{RunFakeConnection, test_ctx},
    },
};
use browseros_cdp::{CdpError, CdpEvent, SessionId};
use browseros_core::{BrowserSession, BrowserSessionHooks, CdpConnection};
use futures_util::future::BoxFuture;
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use tokio::sync::broadcast;

#[derive(Default)]
struct Hook {
    events: Mutex<Vec<Value>>,
}
impl Hook {
    fn push(&self, event: Value) {
        if let Ok(mut events) = self.events.lock() {
            events.push(event);
        }
    }
    fn events(&self) -> Vec<Value> {
        self.events.lock().map(|e| e.clone()).unwrap_or_default()
    }
}
impl InnerCallHook for Hook {
    fn authorize<'a>(&'a self, page: Option<u32>) -> BoxFuture<'a, Result<(), String>> {
        Box::pin(async move {
            self.push(json!(["authorize", page]));
            Ok(())
        })
    }
    fn record<'a>(&'a self, r: InnerCallRecord<'a>) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            self.push(json!([
                "record", r.method, r.page, r.args, r.secrets, r.is_error
            ]));
        })
    }
    fn on_page_created<'a>(&'a self, page: u32) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            self.push(json!(["created", page]));
        })
    }
    fn annotate_pages<'a>(&'a self, pages: &'a [Value]) -> BoxFuture<'a, Vec<Value>> {
        Box::pin(async move {
            pages
                .iter()
                .map(|p| {
                    let mut p = p.clone();
                    p["ownership"] = json!("mine");
                    p
                })
                .collect()
        })
    }
}

/// Keep the legacy fake's page lifecycle, overriding only the protocol calls
/// needed for keyboard input and the active-element redaction probe.
struct ActionConnection {
    base: RunFakeConnection,
    probe: Option<Value>,
    calls: Mutex<Vec<(String, Value)>>,
    hang_input: bool,
}
impl ActionConnection {
    fn new(probe: Option<Value>) -> Self {
        Self {
            base: RunFakeConnection::new(),
            probe,
            calls: Mutex::new(Vec::new()),
            hang_input: false,
        }
    }
}
impl CdpConnection for ActionConnection {
    fn send<'a>(
        &'a self,
        method: &'a str,
        params: Value,
        session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<Value, CdpError>> {
        Box::pin(async move {
            if let Ok(mut calls) = self.calls.lock() {
                calls.push((method.into(), params.clone()));
            }
            match method {
                "Input.insertText" | "Input.dispatchKeyEvent" if self.hang_input => {
                    futures_util::future::pending().await
                }
                "Input.insertText" | "Input.dispatchKeyEvent" | "Input.dispatchMouseEvent" => {
                    Ok(json!({}))
                }
                "Runtime.evaluate" => self
                    .probe
                    .clone()
                    .map(|v| json!({"result":{"value":v}}))
                    .ok_or_else(|| CdpError::Protocol {
                        code: -1,
                        message: "probe unavailable".into(),
                    }),
                _ => self.base.send(method, params, session).await,
            }
        })
    }
    fn send_raw_json<'a>(
        &'a self,
        method: &'a str,
        params: &'a str,
        session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<String, CdpError>> {
        self.base.send_raw_json(method, params, session)
    }
    fn events(&self) -> broadcast::Receiver<CdpEvent> {
        self.base.events()
    }
    fn is_connected(&self) -> bool {
        true
    }
    fn connection_epoch(&self) -> u64 {
        1
    }
}

fn context(connection: Arc<ActionConnection>, hook: Arc<Hook>) -> ToolCtx {
    let mut ctx = test_ctx();
    ctx.session = BrowserSession::new(connection, BrowserSessionHooks::default());
    ctx.inner_call_hook = Some(hook);
    ctx
}

async fn script(ctx: &ToolCtx, code: &str, timeout_ms: u64) -> anyhow::Result<Value> {
    // Bootstrap only the stable runtime contract. Calling the bridge directly
    // isolates P4 from the concurrently developed facade's locator syntax.
    let outcome = execute_script(
        ScriptSpec {
            bootstrap_js: super::FACADE_JS,
            code: code.into(),
            timeout_ms,
            helpers: false,
        },
        ctx,
    )
    .await
    .map_err(|e| anyhow::anyhow!("{e:?}"))?;
    outcome
        .into_tool_result()
        .structured_content
        .ok_or_else(|| anyhow::anyhow!("missing structured output"))
}

#[test]
fn actions_method_prefixes() {
    for method in [
        "page.goto",
        "locator.click",
        "expect.x",
        "context.pages",
        "keyboard.type",
        "mouse.move",
        "frame.info",
        "neo.read",
    ] {
        assert!(super::is_pw_method(method));
    }
    for method in ["pages.list", "input.click", "tool:read", "pageant.x"] {
        assert!(!super::is_pw_method(method));
    }
}

#[tokio::test]
async fn actions_click_keeps_one_authorize_then_record() -> anyhow::Result<()> {
    let hook = Arc::new(Hook::default());
    let ctx = context(Arc::new(ActionConnection::new(None)), hook.clone());
    let result=script(&ctx,"try { await __browserosCall('locator.click', '[1,\"css=button\"]', false); } catch (e) {} return 1;",1000).await?;
    assert_eq!(result["ok"], true);
    let events = hook.events();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0], json!(["authorize", 1]));
    assert_eq!(events[1][0], "record");
    assert_eq!(events[1][1], "locator.click");
    assert_eq!(events[1][2], 1);
    assert_eq!(events[1][3], json!([1, "css=button"]));
    Ok(())
}

#[tokio::test]
async fn actions_new_page_is_claimed_before_recording() -> anyhow::Result<()> {
    let hook = Arc::new(Hook::default());
    let connection = Arc::new(ActionConnection::new(None));
    let ctx = context(connection.clone(), hook.clone());
    let result = script(
        &ctx,
        "return await __browserosCall('context.newPage', '[\"https://new.example\"]', false);",
        1000,
    )
    .await?;
    assert_eq!(result["ok"], true);
    let id = result["value"]
        .as_u64()
        .ok_or_else(|| anyhow::anyhow!("missing page id: {result}"))?;
    let events = hook.events();
    assert_eq!(events.len(), 3);
    assert_eq!(events[0], json!(["authorize", null]));
    assert_eq!(events[1], json!(["created", id]));
    assert_eq!(events[2][1], "context.newPage");
    let calls = connection
        .calls
        .lock()
        .map_err(|_| anyhow::anyhow!("poisoned calls"))?;
    let create = calls
        .iter()
        .find(|(m, _)| m == "Browser.createTab")
        .ok_or_else(|| anyhow::anyhow!("no tab creation"))?;
    assert_eq!(create.1["background"], true);
    Ok(())
}

#[tokio::test]
async fn actions_keyboard_masks_password_and_failed_probes() -> anyhow::Result<()> {
    for probe in [Some(json!({"type":"password","autocomplete":""})), None] {
        let hook = Arc::new(Hook::default());
        let ctx = context(Arc::new(ActionConnection::new(probe)), hook.clone());
        let result=script(&ctx,"await __browserosCall('keyboard.insertText', '[1,\"keep-me-secret\"]', false); return true;",1000).await?;
        assert_eq!(result["ok"], true, "{result}");
        let events = hook.events();
        let row = events
            .last()
            .ok_or_else(|| anyhow::anyhow!("no audit row"))?;
        assert_eq!(row[3], json!([1, "[redacted]"]));
        assert_eq!(row[4], json!(["keep-me-secret"]));
    }
    Ok(())
}

#[test]
fn actions_describe_redaction_policy() {
    for description in [
        None,
        Some(json!({})),
        Some(json!({"tag":"INPUT","type":"password","autocomplete":"off"})),
        Some(json!({"type":"text","autocomplete":"section-login one-time-code"})),
        Some(json!({"type":"tel","autocomplete":"cc-number"})),
    ] {
        assert!(sensitive(description.as_ref()));
    }
    assert!(!sensitive(Some(
        &json!({"type":"text","autocomplete":"name"})
    )));
    let (args, secrets) = redact_args(&[json!(1), json!("input"), json!("secret")], 2);
    assert_eq!(args, Some(json!([1, "input", "[redacted]"])));
    assert_eq!(secrets, vec!["secret"]);
}

#[tokio::test]
async fn actions_operation_timeout_is_bounded_by_run() -> anyhow::Result<()> {
    let hook = Arc::new(Hook::default());
    let mut connection = ActionConnection::new(None);
    connection.hang_input = true;
    let ctx = context(Arc::new(connection), hook);
    let started = tokio::time::Instant::now();
    let result=script(&ctx,"await __browserosCall('keyboard.insertText', '[1,\"secret\",{\"timeout\":60000}]', false);",80).await?;
    assert_eq!(result["ok"], false);
    assert!(started.elapsed() < std::time::Duration::from_secs(1));
    Ok(())
}

#[test]
fn actions_protocol_exception_and_cover_error_are_not_success() {
    assert!(
        super::actions::evaluated_value(
            json!({"exceptionDetails":{"exception":{"description":"boom"}}})
        )
        .is_err()
    );
    let error = super::action_error(
        "locator.click",
        Some("css=button"),
        10000,
        browseros_core::CoreError::ElementCovered {
            target: Default::default(),
            blocker: "div.overlay".into(),
        },
    );
    assert!(error.contains("intercepts pointer events"));
    assert!(error.contains("Call log:\n  - waiting for css=button"));
}

#[test]
fn actions_routes_waits_expect_and_action_leaves() {
    assert_eq!(super::route("page.goto"), super::Route::Waits);
    assert_eq!(super::route("expect.x"), super::Route::Expect);
    assert_eq!(super::route("locator.click"), super::Route::Actions);
    assert_eq!(super::route("context.waitForEvent"), super::Route::Waits);
}

#[tokio::test(start_paused = true)]
async fn actions_deadline_clamps_exactly_to_remaining_run_budget() {
    let now = tokio::time::Instant::now();
    let end = now + std::time::Duration::from_millis(700);
    assert_eq!(super::clamp_deadline(now, end, 60000), end);
    assert_eq!(super::clamp_deadline(now, end, 0), end);
    assert_eq!(
        super::clamp_deadline(now, end, 100),
        now + std::time::Duration::from_millis(100)
    );
    assert_eq!(super::clamp_deadline(now, end, u64::MAX), end);
}

#[tokio::test]
async fn actions_mouse_click_preserves_count_button_and_modifiers() -> anyhow::Result<()> {
    let connection = Arc::new(ActionConnection::new(None));
    let ctx = context(connection.clone(), Arc::new(Hook::default()));
    let result=script(&ctx,r#"await __browserosCall('mouse.click','[1,25,40,{"button":"right","clickCount":2,"modifiers":["Alt","Shift"]}]',false); return true;"#,1000).await?;
    assert_eq!(result["ok"], true, "{result}");
    let calls = connection
        .calls
        .lock()
        .map_err(|_| anyhow::anyhow!("poisoned calls"))?;
    let pressed: Vec<_> = calls
        .iter()
        .filter(|(method, params)| {
            method == "Input.dispatchMouseEvent" && params["type"] == "mousePressed"
        })
        .map(|(_, p)| p)
        .collect();
    assert_eq!(pressed.len(), 2);
    for (i, p) in pressed.iter().enumerate() {
        assert_eq!(p["clickCount"], i + 1);
        assert_eq!(p["button"], "right");
        assert_eq!(p["modifiers"], 9);
        assert_eq!(p["x"], 25.0);
        assert_eq!(p["y"], 40.0);
    }
    Ok(())
}

#[tokio::test]
async fn actions_evaluate_awaits_main_world_and_wraps_user_values() -> anyhow::Result<()> {
    let value = json!({"value":7,"url":"user data"});
    let connection = Arc::new(ActionConnection::new(Some(value.clone())));
    let ctx = context(connection.clone(), Arc::new(Hook::default()));
    let result=script(&ctx,r#"return await __browserosCall('page.evaluate','[1,"async arg => ({value:arg,url:window.location.href})",7]',false);"#,1000).await?;
    assert_eq!(result["ok"], true, "{result}");
    assert_eq!(result["value"]["value"], value);
    assert_eq!(result["value"]["url"], "https://example.com");
    let calls = connection
        .calls
        .lock()
        .map_err(|_| anyhow::anyhow!("poisoned calls"))?;
    let params = &calls
        .iter()
        .find(|(m, _)| m == "Runtime.evaluate")
        .ok_or_else(|| anyhow::anyhow!("missing evaluation"))?
        .1;
    assert_eq!(params["awaitPromise"], true);
    assert_eq!(params["returnByValue"], true);
    assert!(params.get("contextId").is_none());
    Ok(())
}

#[tokio::test]
async fn actions_context_pages_uses_hook_ownership() -> anyhow::Result<()> {
    let connection = Arc::new(ActionConnection::new(None));
    let mut ctx = context(connection, Arc::new(Hook::default()));
    let code = "return await __browserosCall('context.pages','[]',false);";
    let result = script(&ctx, code, 1000).await?;
    assert_eq!(result["value"][0]["ownership"], "mine");
    ctx.inner_call_hook = None;
    assert_eq!(script(&ctx, code, 1000).await?["value"], json!([]));
    Ok(())
}
