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
    other_pages: Vec<u32>,
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
                    p["ownership"] = if p["pageId"].as_u64().is_some_and(|id| {
                        self.other_pages.iter().any(|other| u64::from(*other) == id)
                    }) {
                        json!("other-agent")
                    } else {
                        json!("mine")
                    };
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
    engine: bool,
    hang_resolve: bool,
    input_error: Option<String>,
    tabs: Option<Mutex<Vec<Value>>>,
    discover_tab: bool,
    open_on_input: bool,
}
impl ActionConnection {
    fn new(probe: Option<Value>) -> Self {
        Self {
            base: RunFakeConnection::new(),
            probe,
            calls: Mutex::new(Vec::new()),
            hang_input: false,
            engine: false,
            hang_resolve: false,
            input_error: None,
            tabs: None,
            discover_tab: false,
            open_on_input: false,
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
            if let Some(tabs) = &self.tabs {
                let mut tabs = tabs.lock().unwrap_or_else(|e| e.into_inner());
                match method {
                    "Input.dispatchMouseEvent"
                        if self.open_on_input && params["type"] == "mouseReleased" =>
                    {
                        for id in [4, 5, 6] {
                            if !tabs.iter().any(|tab| tab["tabId"] == id) {
                                tabs.push(action_tab(id));
                            }
                        }
                    }
                    "Target.getTargetInfo" => {
                        return Ok(
                            json!({"targetInfo":{"openerId":if params["targetId"] == "target-6" { "unrelated" } else { "target-1" }}}),
                        );
                    }
                    "Browser.getTabs" => return Ok(json!({"tabs":*tabs})),
                    "Browser.getTabInfo" => {
                        return Ok(
                            json!({"tab":tabs.iter().find(|tab| tab["tabId"] == params["tabId"])}),
                        );
                    }
                    "Browser.closeTab" => {
                        tabs.retain(|tab| tab["tabId"] != params["tabId"]);
                        return Ok(json!({}));
                    }
                    "Target.setDiscoverTargets" if self.discover_tab => {
                        tabs.push(action_tab(4));
                        return Ok(json!({}));
                    }
                    _ => {}
                }
            }
            if self.engine {
                let value = match method {
                    "Page.getFrameTree" => Some(json!({"frameTree":{"frame":{"id":"root"}}})),
                    "Page.createIsolatedWorld" => Some(json!({"executionContextId":42})),
                    "Emulation.setFocusEmulationEnabled"
                    | "DOM.focus"
                    | "DOM.scrollIntoViewIfNeeded"
                    | "Runtime.releaseObject" => Some(json!({})),
                    "DOM.getBoxModel" => Some(
                        json!({"model":{"content":[10,10,30,10,30,30,10,30],"padding":[10,10,30,10,30,30,10,30]}}),
                    ),
                    "DOM.getContentQuads" => Some(json!({"quads":[[10,10,30,10,30,30,10,30]]})),
                    "DOM.resolveNode" => Some(json!({"object":{"objectId":"main-node"}})),
                    "DOM.describeNode" if params.get("objectId").is_some() => {
                        Some(json!({"node":{"backendNodeId":5}}))
                    }
                    "DOM.describeNode" => {
                        let probe = self.probe.as_ref().ok_or_else(|| CdpError::Protocol {
                            code: -1,
                            message: "describe probe failed".into(),
                        })?;
                        Some(
                            json!({"node":{"backendNodeId":5,"localName":"input","attributes":["type",probe["type"],"autocomplete",probe["autocomplete"]]}}),
                        )
                    }
                    "Runtime.evaluate" if params.get("contextId").is_some() => {
                        Some(json!({"result":{"objectId":"engine"}}))
                    }
                    "Runtime.callFunctionOn" => {
                        let source = params["functionDeclaration"].as_str().unwrap_or("");
                        if source.contains("this.parseSelector") {
                            Some(
                                json!({"result":{"value":{"parts":[{"name":"css","body":[],"source":"input"}]}}}),
                            )
                        } else if source.contains("this.querySelector(parsed, document, strict)") {
                            if self.hang_resolve {
                                return futures_util::future::pending().await;
                            }
                            Some(json!({"result":{"objectId":"element"}}))
                        } else if source.contains("return !!this.ownerDocument") {
                            Some(json!({"result":{"value":true}}))
                        } else if params["arguments"][0]["value"] == "fill" {
                            Some(json!({"result":{"value":"needsinput"}}))
                        } else if params["arguments"][0]["value"] == "hitTarget" {
                            Some(json!({"result":{"value":"done"}}))
                        } else {
                            Some(json!({"result":{"value":null}}))
                        }
                    }
                    _ => None,
                };
                if let Some(value) = value {
                    return Ok(value);
                }
            }
            match method {
                "Input.insertText" | "Input.dispatchKeyEvent" if self.input_error.is_some() => {
                    Err(CdpError::Protocol {
                        code: -1,
                        message: self.input_error.clone().unwrap_or_default(),
                    })
                }
                "Input.insertText" | "Input.dispatchKeyEvent" if self.hang_input => {
                    futures_util::future::pending().await
                }
                "Input.insertText" | "Input.dispatchKeyEvent" | "Input.dispatchMouseEvent" => {
                    Ok(json!({}))
                }
                "Browser.closeTab" | "Target.setDiscoverTargets" => Ok(json!({})),
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

fn action_tab(id: i64) -> Value {
    json!({"tabId":id,"targetId":format!("target-{id}"),"url":"https://example.com","title":"Example","isActive":false,"isLoading":false,"loadProgress":1.0,"isPinned":false,"isHidden":false,"windowId":1,"index":id})
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
async fn actions_bookkeeping_is_silent_but_explicit_reads_are_audited() -> anyhow::Result<()> {
    let hook = Arc::new(Hook::default());
    let ctx = context(
        Arc::new(ActionConnection::new(Some(json!("document content")))),
        hook.clone(),
    );
    let result = script(
        &ctx,
        r#"
        await __browserosCall('context.lastPage','[]',false);
        await __browserosCall('page.info','[1]',false);
        const title = await __browserosCall('page.title','[1]',false);
        const content = await __browserosCall('page.content','[1]',false);
        try { await __browserosCall('page.info','[99]',false); } catch (_) {}
        return [title.value,content.value];
    "#,
        1000,
    )
    .await?;
    assert_eq!(result["ok"], true, "{result}");
    assert_eq!(result["value"], json!(["Example", "document content"]));
    let events = hook.events();
    let records: Vec<_> = events
        .iter()
        .filter(|row| row[0] == "record")
        .map(|row| row[1].clone())
        .collect();
    assert_eq!(records, vec![json!("page.title"), json!("page.content")]);
    assert!(events.contains(&json!(["authorize", 99])));
    assert!(events.contains(&json!(["authorize", 1])));
    Ok(())
}

#[tokio::test]
async fn actions_query_audit_uses_public_name_even_on_failure() -> anyhow::Result<()> {
    let hook = Arc::new(Hook::default());
    let ctx = context(Arc::new(ActionConnection::new(None)), hook.clone());
    let result = script(&ctx,r#"
        try { await __browserosCall('locator.query','[99,"css=input","textContent",null]',false); } catch (_) {}
        try { await __browserosCall('locator.query','[99,"css=input","madeUp",null]',false); } catch (_) {}
    "#,1000).await?;
    assert_eq!(result["ok"], true, "{result}");
    let records: Vec<_> = hook
        .events()
        .into_iter()
        .filter(|row| row[0] == "record")
        .collect();
    assert_eq!(records[0][1], "locator.textContent");
    assert_eq!(records[0][3], json!([99, "css=input", "textContent", null]));
    assert_eq!(records[0][5], true);
    assert_eq!(records[1][1], "locator.query");
    Ok(())
}

#[tokio::test]
async fn actions_named_errors_keep_their_class_in_the_wire_envelope() -> anyhow::Result<()> {
    let ctx = context(
        Arc::new(ActionConnection::new(None)),
        Arc::new(Hook::default()),
    );
    for (source, expected) in [
        ("throw new Error('plain');", "plain"),
        ("throw new TypeError('typed');", "TypeError: typed"),
        (
            "const e = new Error('late'); e.name = 'TimeoutError'; throw e;",
            "TimeoutError: late",
        ),
    ] {
        let result = script(&ctx, source, 1000).await?;
        assert_eq!(result["ok"], false);
        assert_eq!(result["error"], expected);
    }
    Ok(())
}

#[tokio::test]
async fn actions_popups_are_claimed_before_click_resolves_and_sync_pages_reads()
-> anyhow::Result<()> {
    let mut connection = ActionConnection::new(None);
    connection.engine = true;
    connection.open_on_input = true;
    connection.tabs = Some(Mutex::new(vec![action_tab(1)]));
    let hook = Arc::new(Hook {
        other_pages: vec![4],
        ..Default::default()
    });
    let ctx = context(Arc::new(connection), hook.clone());
    let result = script(
        &ctx,
        r#"
        const [source] = await context.pages();
        await source.locator('a').click();
        const sync = context.pages().map(p => p.pageId);
        const refreshed = (await context.pages()).map(p => p.pageId);
        return {sync,refreshed};
    "#,
        1000,
    )
    .await?;
    assert_eq!(result["ok"], true, "{result}");
    assert_eq!(result["value"], json!({"sync":[1,2,3],"refreshed":[1,2,3]}));
    let events = hook.events();
    let click = events
        .iter()
        .position(|row| row[0] == "record" && row[1] == "locator.click")
        .ok_or_else(|| anyhow::anyhow!("missing click audit"))?;
    for id in [2, 3] {
        let claimed = events
            .iter()
            .position(|row| *row == json!(["created", id]))
            .ok_or_else(|| anyhow::anyhow!("missing popup claim"))?;
        assert!(claimed < click);
    }
    assert!(
        !events.contains(&json!(["created", 4])),
        "unrelated tab was claimed"
    );
    Ok(())
}

#[tokio::test]
async fn actions_click_keeps_one_authorize_then_record() -> anyhow::Result<()> {
    let hook = Arc::new(Hook::default());
    let mut connection = ActionConnection::new(None);
    connection.engine = true;
    let connection = Arc::new(connection);
    let ctx = context(connection.clone(), hook.clone());
    let result = script(
        &ctx,
        "await __browserosCall('locator.click', '[1,\"css=button\"]', false); return 1;",
        1000,
    )
    .await?;
    assert_eq!(result["ok"], true);
    let events = hook.events();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0], json!(["authorize", 1]));
    assert_eq!(events[1][0], "record");
    assert_eq!(events[1][1], "locator.click");
    assert_eq!(events[1][2], 1);
    assert_eq!(events[1][3], json!([1, "css=button"]));
    assert_eq!(
        events[1][5], false,
        "click must succeed against the fake engine"
    );
    let calls = connection
        .calls
        .lock()
        .map_err(|_| anyhow::anyhow!("poisoned calls"))?;
    let scroll = calls
        .iter()
        .position(|(m, _)| m == "DOM.scrollIntoViewIfNeeded")
        .ok_or_else(|| anyhow::anyhow!("no scroll"))?;
    let states = calls
        .iter()
        .position(|(m, p)| {
            m == "Runtime.callFunctionOn"
                && p["functionDeclaration"]
                    .as_str()
                    .is_some_and(|s| s.contains("this.checkElementStates(el, states)"))
        })
        .ok_or_else(|| anyhow::anyhow!("no state wait"))?;
    let hit = calls
        .iter()
        .position(|(m, p)| {
            m == "Runtime.callFunctionOn" && p["arguments"][0]["value"] == "hitTarget"
        })
        .ok_or_else(|| anyhow::anyhow!("no hit target"))?;
    let input = calls
        .iter()
        .position(|(m, p)| m == "Input.dispatchMouseEvent" && p["type"] == "mousePressed")
        .ok_or_else(|| anyhow::anyhow!("no trusted click"))?;
    assert!(scroll < states && states < hit && hit < input);
    assert!(
        calls
            .iter()
            .any(|(m, p)| m == "Runtime.releaseObject" && p["objectId"] == "element")
    );
    Ok(())
}

#[tokio::test]
async fn actions_click_and_explicit_popup_wait_share_one_claim() -> anyhow::Result<()> {
    let mut connection = ActionConnection::new(None);
    connection.engine = true;
    connection.open_on_input = true;
    connection.tabs = Some(Mutex::new(vec![action_tab(1)]));
    let hook = Arc::new(Hook::default());
    let ctx = context(Arc::new(connection), hook.clone());
    let result = script(
        &ctx,
        r#"
        const [source] = await context.pages();
        const [popup] = await Promise.all([
            context.waitForEvent('page', {timeout:500}),
            source.locator('a').click()
        ]);
        return popup.pageId;
    "#,
        1000,
    )
    .await?;
    assert_eq!(result["ok"], true, "{result}");
    assert_eq!(result["value"], 2);
    assert_eq!(
        hook.events()
            .iter()
            .filter(|row| **row == json!(["created", 2]))
            .count(),
        1
    );
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
async fn actions_failed_typing_preserves_audit_and_error_redaction() -> anyhow::Result<()> {
    // Quotes/newlines exercise both plaintext and JSON-escaped echoes from CDP.
    let secret = "keep-me-\n\\secret\"";
    for method in [
        "locator.fill",
        "locator.type",
        "keyboard.type",
        "keyboard.insertText",
    ] {
        for probe in [Some(json!({"type":"password","autocomplete":""})), None] {
            let hook = Arc::new(Hook::default());
            let mut connection = ActionConnection::new(probe);
            connection.engine = true;
            connection.input_error = Some(format!(
                "rejected {secret} encoded {}",
                serde_json::to_string(secret)?
            ));
            let ctx = context(Arc::new(connection), hook.clone());
            let (args, value_index) = if method.starts_with("locator.") {
                (json!([1,"css=input",secret,{"force":true}]), 2)
            } else {
                (json!([1, secret]), 1)
            };
            let code = format!("await __browserosCall('{method}', JSON.stringify({args}), false);");
            let result = script(&ctx, &code, 1000).await?;
            assert_eq!(result["ok"], false, "{method}: {result}");
            let error = result["error"].to_string();
            assert!(error.contains("rejected"), "{method}: {result}");
            assert!(!error.contains("keep-me-"), "{method}: {result}");
            let events = hook.events();
            assert_eq!(events.len(), 2);
            assert_eq!(events[0], json!(["authorize", 1]));
            let mut masked = args;
            masked[value_index] = json!("[redacted]");
            assert_eq!(
                events[1],
                json!(["record", method, 1, masked, [secret], true])
            );
        }
    }
    Ok(())
}

#[tokio::test]
async fn actions_failed_keyboard_page_validation_still_masks() -> anyhow::Result<()> {
    let hook = Arc::new(Hook::default());
    let ctx = context(Arc::new(ActionConnection::new(None)), hook.clone());
    let result = script(
        &ctx,
        r#"await __browserosCall('keyboard.insertText','["invalid","secret"]',false);"#,
        1000,
    )
    .await?;
    assert_eq!(result["ok"], false);
    assert_eq!(
        hook.events()[1],
        json!([
            "record",
            "keyboard.insertText",
            null,
            ["invalid", "[redacted]"],
            ["secret"],
            true
        ])
    );
    Ok(())
}

#[tokio::test]
async fn actions_operation_timeout_preserves_failed_probe_redaction() -> anyhow::Result<()> {
    let hook = Arc::new(Hook::default());
    let mut connection = ActionConnection::new(Some(json!({"type":"text","autocomplete":""})));
    connection.hang_input = true;
    let ctx = context(Arc::new(connection), hook.clone());
    let result = script(
        &ctx,
        r#"await __browserosCall('keyboard.insertText','[1,"secret",{"timeout":20}]',false);"#,
        1000,
    )
    .await?;
    assert_eq!(result["ok"], false);
    assert!(result.to_string().contains("TimeoutError"));
    assert_eq!(
        hook.events()[1],
        json!(["record","keyboard.insertText",1,[1,"[redacted]",{"timeout":20}],["secret"],true])
    );
    Ok(())
}

#[tokio::test]
async fn actions_query_uses_the_facades_fifth_options_argument() -> anyhow::Result<()> {
    let mut connection = ActionConnection::new(None);
    connection.engine = true;
    connection.hang_resolve = true;
    let ctx = context(Arc::new(connection), Arc::new(Hook::default()));
    let result = script(&ctx, r#"return await __browserosCall('locator.query','[1,"css=input","textContent",null,{"timeout":20}]',false);"#, 1000).await?;
    assert_eq!(result["ok"], false);
    assert!(
        result.to_string().contains("Timeout 20ms exceeded"),
        "{result}"
    );
    Ok(())
}

#[tokio::test]
async fn actions_context_page_wait_can_start_without_tabs() -> anyhow::Result<()> {
    let mut connection = ActionConnection::new(None);
    connection.tabs = Some(Mutex::new(Vec::new()));
    let connection = Arc::new(connection);
    let hook = Arc::new(Hook::default());
    let ctx = context(connection.clone(), hook.clone());
    let (result, ()) = tokio::join!(
        script(
            &ctx,
            "const p = await context.waitForEvent('page', {timeout:500}); return p.pageId;",
            1000
        ),
        async {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            if let Some(tabs) = &connection.tabs {
                tabs.lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .push(action_tab(4));
            }
        }
    );
    let result = result?;
    assert_eq!(result["ok"], true, "{result}");
    assert_eq!(result["value"], 1);
    assert_eq!(hook.events()[1], json!(["created", 1]));
    Ok(())
}

#[tokio::test]
async fn actions_close_and_cdp_attribute_the_optional_page() -> anyhow::Result<()> {
    for (method, args, page) in [
        ("context.close", json!([1]), json!(1)),
        ("neo.cdp", json!(["Runtime.evaluate", {}, 1]), json!(1)),
        ("neo.cdp", json!(["Runtime.evaluate", {}]), Value::Null),
        (
            "neo.cdp",
            json!(["Runtime.evaluate", {}, null]),
            Value::Null,
        ),
    ] {
        let hook = Arc::new(Hook::default());
        let ctx = context(
            Arc::new(ActionConnection::new(Some(json!({})))),
            hook.clone(),
        );
        ctx.session.pages.list().await?;
        let result = script(
            &ctx,
            &format!("return await __browserosCall('{method}',JSON.stringify({args}),false);"),
            1000,
        )
        .await?;
        assert_eq!(result["ok"], true, "{method}: {result}");
        assert_eq!(hook.events()[0], json!(["authorize", page]));
        assert_eq!(
            hook.events()[1],
            json!(["record", method, page, args, [], false])
        );
    }
    Ok(())
}

#[tokio::test]
async fn actions_last_page_checks_liveness_ownership_and_run_activity() -> anyhow::Result<()> {
    let mut connection = ActionConnection::new(None);
    connection.tabs = Some(Mutex::new(vec![
        action_tab(1),
        action_tab(2),
        action_tab(3),
    ]));
    let hook = Arc::new(Hook {
        other_pages: vec![3],
        ..Default::default()
    });
    let mut ctx = context(Arc::new(connection), hook);
    let result = script(
        &ctx,
        r#"
        const call = (m,a=[]) => __browserosCall(m,JSON.stringify(a),false);
        const values = [await call('context.lastPage')];
        await call('page.info',[1]);
        values.push(await call('context.lastPage'));
        await call('page.info',[3]);
        values.push(await call('context.lastPage'));
        await call('context.close',[1]);
        values.push(await call('context.lastPage'));
        await call('context.close',[2]);
        values.push(await call('context.lastPage'));
        return values;
    "#,
        1000,
    )
    .await?;
    assert_eq!(result["ok"], true, "{result}");
    assert_eq!(result["value"], json!([2, 1, 1, 2, null]));
    ctx.inner_call_hook = None;
    assert_eq!(
        script(
            &ctx,
            "return await __browserosCall('context.lastPage','[]',false);",
            1000
        )
        .await?["value"],
        Value::Null
    );
    Ok(())
}

#[tokio::test]
async fn actions_context_page_wait_reuses_waits_and_claims_once() -> anyhow::Result<()> {
    let mut connection = ActionConnection::new(None);
    connection.tabs = Some(Mutex::new(vec![action_tab(1)]));
    connection.discover_tab = true;
    let hook = Arc::new(Hook::default());
    let ctx = context(Arc::new(connection), hook.clone());
    let result = script(
        &ctx,
        "const p = await context.waitForEvent('page', {timeout:100}); return p.pageId;",
        1000,
    )
    .await?;
    assert_eq!(result["ok"], true, "{result}");
    assert_eq!(result["value"], 2);
    let events = hook.events();
    assert_eq!(events.len(), 3);
    assert_eq!(events[0], json!(["authorize", null]));
    assert_eq!(events[1], json!(["created", 2]));
    assert_eq!(
        events[2],
        json!(["record","context.waitForEvent",null,["page",null,{"timeout":100}],[],false])
    );
    Ok(())
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

#[tokio::test]
async fn actions_locator_type_masks_describe_password_and_failure() -> anyhow::Result<()> {
    for probe in [Some(json!({"type":"password","autocomplete":""})), None] {
        let hook = Arc::new(Hook::default());
        let mut connection = ActionConnection::new(probe);
        connection.engine = true;
        let ctx = context(Arc::new(connection), hook.clone());
        let result=script(&ctx,r#"await __browserosCall('locator.type','[1,"css=input","keep-me-secret"]',false); return true;"#,1000).await?;
        assert_eq!(result["ok"], true, "{result}");
        let events = hook.events();
        let row = events
            .last()
            .ok_or_else(|| anyhow::anyhow!("no audit row"))?;
        assert_eq!(row[3], json!([1, "css=input", "[redacted]"]));
        assert_eq!(row[4], json!(["keep-me-secret"]));
    }
    Ok(())
}

#[tokio::test]
async fn actions_empty_fill_uses_delete_and_force_skips_states() -> anyhow::Result<()> {
    for force in [false, true] {
        let mut connection = ActionConnection::new(Some(json!({"type":"text","autocomplete":""})));
        connection.engine = true;
        let connection = Arc::new(connection);
        let ctx = context(connection.clone(), Arc::new(Hook::default()));
        let args = serde_json::to_string(&json!([1,"css=input","",{"force":force}]))?;
        let code = format!(
            "await __browserosCall('locator.fill',{},false);return true;",
            serde_json::to_string(&args)?
        );
        let result = script(&ctx, &code, 1000).await?;
        assert_eq!(result["ok"], true, "{result}");
        let calls = connection
            .calls
            .lock()
            .map_err(|_| anyhow::anyhow!("poisoned calls"))?;
        assert!(
            calls
                .iter()
                .any(|(m, p)| m == "Input.dispatchKeyEvent" && p["key"] == "Delete")
        );
        assert!(
            !calls
                .iter()
                .any(|(m, p)| m == "Input.dispatchKeyEvent" && p["key"] == "Backspace")
        );
        let waited = calls.iter().any(|(m, p)| {
            m == "Runtime.callFunctionOn"
                && p["functionDeclaration"]
                    .as_str()
                    .is_some_and(|s| s.contains("this.checkElementStates(el, states)"))
        });
        assert_eq!(waited, !force);
    }
    Ok(())
}
