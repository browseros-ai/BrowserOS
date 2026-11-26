use super::*;
use crate::api::mcp::service::playwright::{
    REQUEST_HELP_DESCRIPTION as PLAYWRIGHT_REQUEST_HELP_DESCRIPTION, surface,
};
use crate::{
    api::mcp::{dispatch::ToolCall, test_support},
    config::Behavior,
};
use serde_json::json;

async fn desktop_playwright_call(tool: &str, args: Value) -> anyhow::Result<ToolCall> {
    test_support::tool_call_with_behavior(
        tool,
        args,
        ServerMode::Desktop,
        Behavior {
            playwright_run: true,
            ..Behavior::default()
        },
    )
    .await
}

async fn viewport_test_service(mode: ServerMode) -> anyhow::Result<ClawMcpService> {
    if mode.is_hosted() {
        return super::viewport_test_service(mode).await;
    }
    let call = desktop_playwright_call("tabs", json!({})).await?;
    call.state
        .browser
        .set_session_for_testing(test_support::current_session(
            test_support::ProfileConnection::new().with_default_profile(),
        ))
        .await;
    Ok(ClawMcpService::new(call.state))
}

async fn playwright_captcha_test_service(enabled: bool) -> anyhow::Result<ClawMcpService> {
    use crate::services::captcha::{CaptchaService, config};
    let mut call = desktop_playwright_call("tabs", json!({})).await?;
    let config = if enabled {
        let key = tempfile::NamedTempFile::new()?;
        std::fs::write(key.path(), "dummy-test-key")?;
        config::CaptchaConfig::from_lookup(|name| {
            (name == config::CAPSOLVER_KEY_FILE_ENV)
                .then(|| key.path().to_string_lossy().into_owned())
        })
    } else {
        config::CaptchaConfig::default()
    };
    call.state.captcha = CaptchaService::from_config(config, ServerMode::Desktop);
    Ok(ClawMcpService::new(call.state))
}

async fn assert_solve_captcha_is_absent(service: ClawMcpService) -> anyhow::Result<()> {
    assert!(
        !service
            .listed_tools(true)
            .iter()
            .any(|tool| tool.name == "solve_captcha")
    );
    assert!(service.get_tool("solve_captcha").is_none());
    let (_client, transport) = tokio::io::duplex(4096);
    let running = rmcp::service::serve_directly(service, transport, None);
    let context = RequestContext::new(rmcp::model::RequestId::Number(1), running.peer().clone());
    let result = running
        .service()
        .call_tool(
            CallToolRequestParams::new("solve_captcha")
                .with_arguments(serde_json::from_value(json!({"page": 1}))?),
            context,
        )
        .await;
    if running.service().mode().is_hosted() {
        assert!(matches!(result, Err(error) if error.code.0 == -32601));
    } else {
        let Ok(CallToolResponse::Complete(result)) = result else {
            panic!("expected tool refusal")
        };
        assert_eq!(result.is_error, Some(true));
        assert!(text_of(&result).contains("run"));
    }
    running.cancel().await?;
    Ok(())
}

#[tokio::test]
async fn tests_that_desktop_solve_captcha_is_absent_with_a_key() -> anyhow::Result<()> {
    assert_solve_captcha_is_absent(playwright_captcha_test_service(true).await?).await
}

#[tokio::test]
async fn tests_that_unkeyed_solve_captcha_is_absent_in_both_modes() -> anyhow::Result<()> {
    for mode in [ServerMode::Desktop, ServerMode::Hosted] {
        let service = if mode.is_hosted() {
            captcha_test_service(false, mode).await?
        } else {
            playwright_captcha_test_service(false).await?
        };
        assert_solve_captcha_is_absent(service).await?;
    }
    Ok(())
}

#[tokio::test]
async fn desktop_run_keeps_sdk_arguments_and_host_session_decorators() -> anyhow::Result<()> {
    let call = desktop_playwright_call("tabs", json!({})).await?;
    let service = ClawMcpService::new(call.state);
    let sdk = service.state.neo.tool_snapshot();
    for modern in [false, true] {
        let run = service
            .listed_tools(modern)
            .into_iter()
            .find(|tool| tool.name == "run")
            .ok_or_else(|| anyhow::anyhow!("run missing"))?;
        assert_eq!(run.description.as_deref(), Some(sdk.description.as_str()));
        for field in ["code", "timeout"] {
            assert_eq!(
                run.input_schema["properties"][field],
                sdk.input_schema["properties"][field]
            );
        }
        assert_eq!(run.input_schema["properties"]["session"]["type"], "string");
        assert_eq!(run.input_schema["additionalProperties"], false);
        assert_eq!(
            run.input_schema["properties"].get(AGENT_NAME_ARG).is_some(),
            modern
        );
    }
    let run = service
        .get_tool("run")
        .ok_or_else(|| anyhow::anyhow!("run lookup missing"))?;
    assert_eq!(
        run.input_schema["properties"]["code"],
        sdk.input_schema["properties"]["code"]
    );
    assert_eq!(run.input_schema["properties"]["session"]["type"], "string");
    Ok(())
}

#[tokio::test]
async fn desktop_run_strips_host_fields_before_the_sdk_wire() -> anyhow::Result<()> {
    use crate::services::neo::NeoEngine;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let port = listener.local_addr()?.port();
    let (sent, mut received) = tokio::sync::mpsc::unbounded_channel();
    let app = axum::Router::new().route(
        "/mcp",
        axum::routing::post(move |axum::Json(body): axum::Json<Value>| {
            let sent = sent.clone();
            async move {
                let _ = sent.send(body["params"]["arguments"].clone());
                axum::Json(json!({
                    "jsonrpc":"2.0", "id":body["id"], "result": {
                        "content":[], "structuredContent":{"value":42}, "isError":false,
                        "_meta":{"com.browseros.neo/run":{
                            "id":"fake-run", "status":"ok", "effects":"none", "tasks":[]
                        }}
                    }
                }))
            }
        }),
    );
    let server = tokio::spawn(async move { axum::serve(listener, app).await });
    let mut call = desktop_playwright_call("tabs", json!({})).await?;
    call.state.neo = NeoEngine::for_test_endpoint(port)?;
    let service = ClawMcpService::new(call.state);
    let result = call_tool_value(service, "run", json!({
            "code":"return 42", "timeout":1000, "session":"old-handle", "agentName":"test-agent", "taskName":"test task"
        })).await?.map_err(|error| anyhow::anyhow!("{error:?}"))?;
    server.abort();
    assert_eq!(result["structuredContent"]["value"], 42);
    let forwarded = received
        .recv()
        .await
        .ok_or_else(|| anyhow::anyhow!("no forwarded arguments"))?;
    assert_eq!(forwarded, json!({"code":"return 42", "timeout":1000}));
    Ok(())
}

#[tokio::test]
async fn tests_that_only_hosted_servers_offer_set_viewport() -> anyhow::Result<()> {
    let hosted = viewport_test_service(ServerMode::Hosted).await?;
    let tool = hosted
        .listed_tools(true)
        .into_iter()
        .find(|tool| tool.name == "setViewport")
        .ok_or_else(|| anyhow::anyhow!("setViewport missing from the hosted list"))?;
    assert!(tool.input_schema["properties"].get("taskName").is_some());
    assert!(hosted.get_tool("setViewport").is_some());
    let run = hosted
        .listed_tools(true)
        .into_iter()
        .find(|tool| tool.name == "run")
        .ok_or_else(|| anyhow::anyhow!("run missing from the hosted list"))?;
    let run_description = run.description.as_deref().unwrap_or_default();
    assert!(
        run_description.contains(
            "page.setViewport({ mode: 'mobile' | 'desktop' | 'default', zoom?, width?, height? })"
        ),
        "{run_description}"
    );
    // Dispatched to the tool, which admits the page before anything else.
    let refused = call_tool_value(hosted, "setViewport", json!({"page": 1, "mode": "mobile"}))
        .await?
        .map_err(|error| anyhow::anyhow!("{error:?}"))?;
    assert_eq!(refused["isError"], true);
    let text = refused["content"][0]["text"].as_str().unwrap_or_default();
    assert!(
        text.starts_with("setViewport failed: page_out_of_scope"),
        "{text}"
    );

    let desktop = viewport_test_service(ServerMode::Desktop).await?;
    assert!(
        !desktop
            .listed_tools(true)
            .iter()
            .any(|tool| tool.name == "setViewport")
    );
    assert!(desktop.get_tool("setViewport").is_none());
    let missing =
        call_tool_value(desktop, "setViewport", json!({"page": 1, "mode": "mobile"})).await?;
    let missing = missing.map_err(|error| anyhow::anyhow!("{error:?}"))?;
    assert_eq!(missing["isError"], true);
    assert!(missing.to_string().contains("run"));
    Ok(())
}

#[tokio::test]
async fn tests_that_only_hosted_run_scripts_get_page_set_viewport() -> anyhow::Result<()> {
    let code = "try { return typeof browser.page(1).setViewport } catch (e) { return 'refused: ' + e.message }";
    for mode in [ServerMode::Hosted, ServerMode::Desktop] {
        let service = viewport_test_service(mode).await?;
        let result = call_tool_value(service, "run", json!({"code": code}))
            .await?
            .map_err(|error| anyhow::anyhow!("{error:?}"))?;
        if mode == ServerMode::Desktop {
            assert_eq!(result["isError"], true);
            assert!(
                result
                    .to_string()
                    .contains("Playwright engine has not been started")
            );
            continue;
        }
        let value = result["structuredContent"]["value"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        assert!(value.starts_with("function"), "{mode:?}: {result}");
    }
    Ok(())
}

#[tokio::test]
async fn initialize_info_uses_browseros_neo_identity_and_prompt() -> anyhow::Result<()> {
    let call = desktop_playwright_call("tabs", json!({})).await?;
    let service = ClawMcpService::new(call.state);
    let info = service.get_info();
    assert_eq!(info.server_info.name, SERVER_NAME);
    assert_eq!(info.server_info.version, VERSION);
    assert_eq!(info.server_info.title.as_deref(), Some(SERVER_TITLE));
    let instructions = info.instructions.unwrap_or_default();
    assert!(instructions.contains("Playwright API"));
    assert!(instructions.contains("page.tabId"));
    Ok(())
}

#[tokio::test]
async fn instructions_use_the_mode_and_grouping_policy() -> anyhow::Result<()> {
    use crate::config::{Behavior, TabGrouping};
    for mode in [ServerMode::Desktop, ServerMode::Hosted] {
        for tab_grouping in [TabGrouping::Automatic, TabGrouping::Explicit] {
            let call = crate::api::mcp::test_support::tool_call_with_behavior(
                "tabs",
                json!({}),
                mode,
                Behavior {
                    tab_grouping,
                    playwright_run: true,
                    ..Behavior::default()
                },
            )
            .await?;
            let service = ClawMcpService::new(call.state);
            let instructions = service.get_info().instructions.unwrap_or_default();
            assert_eq!(instructions.contains("page.tabId"), !mode.is_hosted());
            if !mode.is_hosted() {
                assert_eq!(
                    instructions.contains("Tabs are grouped as"),
                    tab_grouping.is_automatic()
                );
            }
            for tool in service.listed_tools(true) {
                assert_eq!(
                    tool.input_schema["properties"]
                        .get(hosted::TASK_NAME_ARG)
                        .is_some(),
                    mode.is_hosted()
                );
            }
        }
    }
    Ok(())
}

#[tokio::test]
async fn desktop_surface_never_advertises_task_name() -> anyhow::Result<()> {
    let call = desktop_playwright_call("tabs", json!({})).await?;
    let service = ClawMcpService::new(call.state);
    for tool in service.listed_tools(true) {
        let schema = Value::Object(tool.input_schema.as_ref().clone());
        assert_eq!(schema["properties"][hosted::TASK_NAME_ARG], Value::Null);
        let agent_name = schema["properties"][AGENT_NAME_ARG]["description"]
            .as_str()
            .unwrap_or_default();
        assert!(
            agent_name.contains("Name your task with neo.createTask(name) in run."),
            "{agent_name}"
        );
    }
    Ok(())
}

#[tokio::test]
async fn desktop_lists_only_run_and_gated_exceptions() -> anyhow::Result<()> {
    let call = desktop_playwright_call("tabs", json!({})).await?;
    let service = ClawMcpService::new(call.state);
    for exceptions in [false, true] {
        for pursue in [false, true] {
            for help in [HumanHelp::Off, HumanHelp::On] {
                let policy = surface::Surface {
                    list_exceptions: exceptions,
                    human_help: help,
                    pursue_offered: pursue,
                };
                let tools = service
                    .playwright
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("Playwright missing"))?
                    .listed_tools_for(&service, true, policy);
                let names = tools
                    .iter()
                    .map(|tool| tool.name.as_ref())
                    .collect::<Vec<_>>();
                let mut expected = vec!["run"];
                if exceptions {
                    if help == HumanHelp::On {
                        expected.extend([REQUEST_HELP_TOOL_NAME, AWAIT_HELP_TOOL_NAME]);
                    }
                    if pursue {
                        expected.push(PURSUE_TOOL_NAME);
                    }
                }
                assert_eq!(names, expected);
            }
        }
    }
    for tool in service.catalog.iter().filter(|tool| tool.name != "run") {
        assert!(
            service.get_tool(tool.name).is_none(),
            "{} leaked through lookup",
            tool.name
        );
    }
    Ok(())
}

#[tokio::test]
async fn desktop_host_descriptions_do_not_teach_removed_tools() -> anyhow::Result<()> {
    fn descriptions(value: &Value, texts: &mut Vec<String>) {
        match value {
            Value::Object(fields) => {
                for (key, value) in fields {
                    if key == "description"
                        && let Some(text) = value.as_str()
                    {
                        texts.push(text.to_owned());
                    } else {
                        descriptions(value, texts);
                    }
                }
            }
            Value::Array(values) => {
                for value in values {
                    descriptions(value, texts);
                }
            }
            _ => {}
        }
    }
    let call = desktop_playwright_call("run", json!({})).await?;
    let mut state = call.state;
    state.ultrafast.set(true);
    state.decisions = Arc::new(crate::services::decisions::Decisions::new(Some(
        "http://127.0.0.1:9/".parse()?,
    )));
    let service = ClawMcpService::new(state);
    let mut texts = vec![service.state.neo.tool_snapshot().description];
    for modern in [false, true] {
        for tool in service.listed_tools(modern) {
            let mut input = Value::Object(tool.input_schema.as_ref().clone());
            if tool.name == "run" {
                // SDK fields and prose are upstream-owned. Only host decorators
                // belong to this policy's wording contract.
                if let Some(Value::Object(properties)) = input.get_mut("properties") {
                    properties.remove("code");
                    properties.remove("timeout");
                }
            } else {
                texts.push(tool.description.unwrap_or_default().into_owned());
            }
            descriptions(&input, &mut texts);
            if let Some(schema) = tool.output_schema {
                descriptions(&Value::Object(schema.as_ref().clone()), &mut texts);
            }
        }
    }
    for text in texts {
        assert_eq!(surface::removed_tool_reference(&text), None, "{text}");
    }
    let off = surface::Surface {
        list_exceptions: false,
        human_help: HumanHelp::Off,
        pursue_offered: false,
    };
    for name in ["snapshot", REQUEST_HELP_TOOL_NAME] {
        if let surface::Admission::Refuse(text) = off.admission(name) {
            assert_eq!(surface::removed_tool_reference(text), None, "{text}");
        }
    }
    Ok(())
}

#[tokio::test]
async fn human_help_reaches_a_tab_in_another_profile() -> anyhow::Result<()> {
    use crate::api::mcp::test_support::{ProfileConnection, current_session};
    let mut call = desktop_playwright_call("run", json!({})).await?;
    let (engine, _host) = tab_inventory(json!([
        {"tabId": 102, "targetId": "T102"},
    ]))
    .await?;
    call.state.neo = engine;
    // Tab 102 is in the ops profile while Default is loaded in a separate window.
    let browser = current_session(ProfileConnection::new().with_default_profile());
    call.state.browser.set_session_for_testing(browser).await;
    let service = ClawMcpService::new(call.state.clone());
    let started = service
        .ensure_session_started(SessionId::new("help-another-profile"))
        .await?;
    let cancelled = CancellationToken::new();
    cancelled.cancel();

    let result = service
        .call_request_human_help(
            &started,
            &json!({"page": 102, "reason": "Enter the code"}),
            cancelled,
        )
        .await;

    assert_ne!(result.is_error, Some(true), "{}", text_of(&result));
    let entry = call
        .state
        .help
        .get(started.session.id())
        .await
        .ok_or_else(|| anyhow::anyhow!("help entry missing"))?;
    assert_eq!(entry.browser_tab_id(), Some(102));
    Ok(())
}

#[tokio::test]
async fn human_help_accepts_a_tab_opened_by_the_same_sessions_run() -> anyhow::Result<()> {
    use crate::api::mcp::test_support::{ProfileConnection, current_session};
    use crate::services::neo::TaskKey;
    use axum::http::StatusCode;

    const CODE: &str = "return (await context.newPage()).tabId";
    let mut call = desktop_playwright_call("run", json!({})).await?;
    let session = call
        .state
        .sessions
        .lookup(&call.session_id)
        .await
        .ok_or_else(|| anyhow::anyhow!("fixture session missing"))?;
    let task = TaskKey::for_session(session.convo_id());
    let browser = current_session(ProfileConnection::new().with_default_profile());
    let sdk_browser = browser.clone();
    let (inventory, inventory_reads) = tokio::sync::watch::channel(json!([]));
    let host = tab_inventory_router(inventory_reads).route(
        "/mcp",
        axum::routing::post(move |axum::Json(body): axum::Json<Value>| async move {
            let request = &body["params"];
            let request_task = &request["_meta"]["com.browseros.neo/run"]["taskId"];
            let value = if request["arguments"]["code"] == CODE && request_task == task.as_str() {
                // The SDK has its own CDP connection. The host discovers the new tab by
                // native ID after run returns, just as it does for a real context.newPage().
                let opened = sdk_browser
                    .cdp(
                        "Browser.createTab",
                        json!({"windowId": 4, "url": "about:blank"}),
                        None,
                    )
                    .await
                    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
                let tab = &opened["tab"];
                inventory.send_replace(json!([{
                    "tabId": tab["tabId"], "targetId": tab["targetId"], "taskId": task,
                    "url": tab["url"], "title": tab["title"], "taskIds": [task],
                }]));
                tab["tabId"].clone()
            } else if request_task.is_null() {
                // The host's follow-up census has no task and reads the holder.
                inventory.borrow().clone()
            } else {
                return Err(StatusCode::BAD_REQUEST);
            };
            Ok(axum::Json(json!({
                "jsonrpc": "2.0", "id": body["id"], "result": {
                    "content": [], "structuredContent": {"value": value}, "isError": false,
                    "_meta": {"com.browseros.neo/run": {
                        "id": "fake-run", "status": "ok", "effects": "none", "tasks": []
                    }}
                }
            })))
        }),
    );
    let (engine, _host) = start_test_sdk(host).await?;
    call.state.neo = engine;
    call.state.browser.set_session_for_testing(browser).await;
    let service = ClawMcpService::new(call.state);
    let started = service.ensure_session_started(call.session_id).await?;
    let run = crate::api::mcp::service::playwright::run::call(
        &service.state,
        &started,
        serde_json::from_value(json!({"code": CODE}))?,
        CancellationToken::new(),
    )
    .await;
    assert_ne!(run.is_error, Some(true), "{}", text_of(&run));
    let tab = run
        .structured_content
        .and_then(|body| body["value"].as_i64())
        .ok_or_else(|| anyhow::anyhow!("run did not return a tab ID"))?;
    let cancelled = CancellationToken::new();
    cancelled.cancel();
    let help = service
        .call_request_human_help(
            &started,
            &json!({"page": tab, "reason": "Enter the code"}),
            cancelled,
        )
        .await;
    assert_ne!(help.is_error, Some(true), "{}", text_of(&help));
    let returned_page = help
        .structured_content
        .and_then(|body| body["page"].as_i64());
    assert_eq!(returned_page, Some(tab));
    Ok(())
}

#[tokio::test]
async fn human_help_refuses_reserved_and_unreachable_tabs() -> anyhow::Result<()> {
    use crate::api::mcp::test_support::{ProfileConnection, current_session};
    let mut call = desktop_playwright_call("run", json!({})).await?;
    let connection = ProfileConnection::new().with_default_profile();
    for tab in connection
        .tabs
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .iter_mut()
    {
        match tab["tabId"].as_i64() {
            Some(103) => tab["url"] = json!("chrome-extension://excluded/tab.html"),
            Some(104) => tab["url"] = json!("about:blank"),
            _ => {}
        }
    }
    let browser = current_session(connection);
    let (engine, _host) = tab_inventory(json!([
        {"tabId": 101, "targetId": "previous-target"},
        {"tabId": 103, "targetId": "T103"},
        {"tabId": 104, "targetId": "T104", "owner": "neo"},
    ]))
    .await?;
    call.state.neo = engine;
    call.state.browser.set_session_for_testing(browser).await;
    let service = ClawMcpService::new(call.state.clone());
    let started = service
        .ensure_session_started(SessionId::new("help-unreachable-tab"))
        .await?;
    let cancelled = CancellationToken::new();
    cancelled.cancel();

    for (tab, message) in [
        (104, "not one of your open pages"),
        (103, "not open in the connected browser"),
        (999, "not open in the connected browser"),
        (101, "not one of your open pages"),
    ] {
        let result = service
            .call_request_human_help(
                &started,
                &json!({"page": tab, "reason": "Enter the code"}),
                cancelled.clone(),
            )
            .await;
        assert_eq!(result.is_error, Some(true), "tab {tab}: {result:?}");
        assert!(text_of(&result).contains(message), "tab {tab}: {result:?}");
        assert!(call.state.help.get(started.session.id()).await.is_none());
    }
    Ok(())
}

#[tokio::test]
async fn playwright_pursue_view_refuses_a_reused_tab_after_reconnect() -> anyhow::Result<()> {
    use crate::api::mcp::test_support::{ProfileConnection, current_session};
    use browseros_core::{CoreError, TargetError};

    let mut call = desktop_playwright_call("run", json!({})).await?;
    let (engine, _host) = tab_inventory(json!([
        {"tabId": 102, "targetId": "T102"},
    ]))
    .await?;
    call.state.neo = engine;
    let connection = ProfileConnection::new().with_default_profile();
    let browser = current_session(connection.clone());
    call.state
        .browser
        .set_session_for_testing(browser.clone())
        .await;
    let state = call.state;
    let service = ClawMcpService::new(state.clone());
    let playwright = service
        .playwright
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("Playwright is off"))?;
    let page = PageNaming::NativeTab
        .host_page(&state, 102)
        .await
        .ok_or_else(|| anyhow::anyhow!("host did not discover tab 102"))?;
    let (_, view) = playwright
        .resolve_page(
            &state,
            browser.clone(),
            &NamedPage {
                said: 102,
                page: page.clone(),
            },
        )
        .await?
        .ok_or_else(|| anyhow::anyhow!("pursue could not admit tab 102"))?;

    // A decision can outlive the browser connection. Reconciliation reuses the
    // host PageId when the replacement connection reports the same native tab id.
    connection.reconnect();
    {
        let mut tabs = connection
            .tabs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let replacement = tabs
            .iter_mut()
            .find(|tab| tab["tabId"] == 102)
            .ok_or_else(|| anyhow::anyhow!("fixture tab is missing"))?;
        replacement["targetId"] = json!("replacement-target");
        replacement["url"] = json!("about:blank");
    }
    browser.pages.list().await?;
    let replacement = browser
        .pages
        .get_info(page.clone())
        .await
        .ok_or_else(|| anyhow::anyhow!("fixture did not reuse the old page id"))?;
    assert_eq!(replacement.target_id.0, "replacement-target");

    let result = view.page_session(page).await;
    assert!(
        matches!(
            result,
            Err(CoreError::Target(TargetError::PageTargetChanged(_)))
        ),
        "{result:?}"
    );
    Ok(())
}

async fn tab_inventory(
    items: Value,
) -> anyhow::Result<(crate::services::neo::NeoEngine, tokio::task::JoinSet<()>)> {
    let (_inventory, reads) = tokio::sync::watch::channel(items);
    start_test_sdk(tab_inventory_router(reads)).await
}

fn tab_inventory_router(inventory: tokio::sync::watch::Receiver<Value>) -> axum::Router {
    use axum::http::{HeaderMap, StatusCode, header};
    axum::Router::new().route(
        "/v1/tabs",
        axum::routing::get(move |headers: HeaderMap| async move {
            if headers
                .get(header::AUTHORIZATION)
                .is_none_or(|value| value != "Bearer test-host-token")
            {
                return Err(StatusCode::UNAUTHORIZED);
            }
            Ok(axum::Json(json!({"items": inventory.borrow().clone()})))
        }),
    )
}

async fn start_test_sdk(
    host: axum::Router,
) -> anyhow::Result<(crate::services::neo::NeoEngine, tokio::task::JoinSet<()>)> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let engine =
        crate::services::neo::NeoEngine::for_test_host_endpoint(listener.local_addr()?.port())?;
    let mut tasks = tokio::task::JoinSet::new();
    tasks.spawn(async move {
        let _ = axum::serve(listener, host).await;
    });
    Ok((engine, tasks))
}

#[tokio::test]
async fn human_help_translates_native_tab_ids_and_returns_them_after_resolution()
-> anyhow::Result<()> {
    use crate::api::mcp::test_support::{ProfileConnection, current_session};
    let mut call = desktop_playwright_call("run", json!({})).await?;
    let (engine, _host) = tab_inventory(json!([
        {"tabId": 104, "targetId": "T104"},
    ]))
    .await?;
    call.state.neo = engine;
    let browser = current_session(ProfileConnection::new().with_default_profile());
    assert!(
        browser
            .pages
            .get_info_by_tab(browseros_core::TabId(104))
            .await
            .is_none()
    );
    call.state
        .browser
        .set_session_for_testing(browser.clone())
        .await;
    let service = ClawMcpService::new(call.state.clone());
    let started = service
        .ensure_session_started(SessionId::new("help-native-tab"))
        .await?;
    let cancelled = CancellationToken::new();
    cancelled.cancel();
    let result = service
        .call_request_human_help(
            &started,
            &json!({"page":104, "reason":"Enter the code"}),
            cancelled,
        )
        .await;
    assert_ne!(result.is_error, Some(true), "{result:?}");
    assert_eq!(
        result
            .structured_content
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("help result missing"))?["page"],
        104
    );
    assert_eq!(
        call.state
            .help
            .get(started.session.id())
            .await
            .ok_or_else(|| anyhow::anyhow!("help entry missing"))?
            .browser_tab_id(),
        Some(104)
    );
    let page = browser
        .pages
        .get_info_by_tab(browseros_core::TabId(104))
        .await
        .ok_or_else(|| anyhow::anyhow!("tab was not discovered"))?;
    assert_ne!(i64::from(page.page_id.0), 104);
    assert!(call.state.help.resolve(started.session.id(), None).await);
    let resolved = service
        .call_await_human_help(&started, CancellationToken::new())
        .await;
    let body = resolved
        .structured_content
        .ok_or_else(|| anyhow::anyhow!("resolved body missing"))?;
    assert_eq!(body["status"], "resolved");
    assert_eq!(body["page"], 104);
    Ok(())
}

/// The hand-back path end to end: an agent opens a request, a human resolves it
/// with a note from the cockpit, and the next wait call reports resolved with the
/// note and clears the entry so the agent resumes exactly once.
#[tokio::test]
async fn await_human_help_reports_the_hand_back_note_and_clears_it() -> anyhow::Result<()> {
    let call = desktop_playwright_call("tabs", json!({})).await?;
    let service = ClawMcpService::new(call.state.clone());
    let started = service
        .ensure_session_started(SessionId::new("mcp-help-resolved"))
        .await
        .map_err(|error| anyhow::anyhow!("{error:?}"))?;
    let session_id = started.session.id().clone();

    call.state
        .help
        .open(
            &session_id,
            HelpOpenParams {
                request_id: "help-test".to_string(),
                reason: "Enter the code".to_string(),
                details: None,
                resume_hint: None,
                kind: None,
                browser_tab_id: Some(5000000000),
                url: None,
                title: None,
            },
        )
        .await;
    assert!(
        call.state
            .help
            .resolve(&session_id, Some("2fa done".to_string()))
            .await
    );

    let result = service
        .call_await_human_help(&started, CancellationToken::new())
        .await;

    let Some(structured) = result.structured_content.clone() else {
        panic!("structured content");
    };
    assert_eq!(
        structured.get("status").and_then(Value::as_str),
        Some("resolved")
    );
    assert_eq!(
        structured.get("note").and_then(Value::as_str),
        Some("2fa done")
    );
    assert_eq!(structured["page"], 5000000000i64);
    assert!(call.state.help.get(&session_id).await.is_none());
    Ok(())
}

#[tokio::test]
async fn operator_stop_keeps_the_original_native_help_tab() -> anyhow::Result<()> {
    let call = desktop_playwright_call("run", json!({})).await?;
    let service = ClawMcpService::new(call.state.clone());
    let started = service
        .ensure_session_started(SessionId::new("help-stop-tab"))
        .await?;
    let entry = call
        .state
        .help
        .open(
            started.session.id(),
            HelpOpenParams {
                request_id: "help-stop-tab".to_string(),
                reason: "Enter the code".to_string(),
                details: None,
                resume_hint: None,
                kind: None,
                browser_tab_id: Some(5000000000),
                url: None,
                title: None,
            },
        )
        .await;
    let (result, stopped) = tokio::join!(
        service.run_help_wait(&started, &entry, CancellationToken::new()),
        async {
            tokio::time::timeout(std::time::Duration::from_secs(5), async {
                while started.session.active_dispatch_count().await == 0 {
                    tokio::task::yield_now().await;
                }
            })
            .await?;
            started.session.request_operator_stop();
            started.session.stop_dispatches().await;
            Ok::<(), anyhow::Error>(())
        }
    );
    stopped?;
    assert_eq!(result.is_error, Some(true));
    let body = result
        .structured_content
        .ok_or_else(|| anyhow::anyhow!("Stop result missing"))?;
    assert_eq!(body["cancellationKind"], "cockpit.operator-cancelled");
    assert_eq!(body["page"], 5000000000i64);
    Ok(())
}

#[tokio::test]
async fn help_tools_are_listed_with_their_descriptions_and_are_safe() -> anyhow::Result<()> {
    let call = desktop_playwright_call("tabs", json!({})).await?;
    let service = ClawMcpService::new(call.state);
    let listed = service.listed_tools(false);

    // request_human_help changes state (it creates a request) so it is not
    // read-only; await_human_help only waits, so it is. Neither is destructive.
    for (name, description, read_only) in [
        (
            REQUEST_HELP_TOOL_NAME,
            PLAYWRIGHT_REQUEST_HELP_DESCRIPTION,
            false,
        ),
        (AWAIT_HELP_TOOL_NAME, AWAIT_HELP_DESCRIPTION, true),
    ] {
        let Some(tool) = listed.iter().find(|tool| tool.name == name) else {
            panic!("{name} missing from list");
        };
        assert_eq!(tool.description.as_deref(), Some(description));
        let annotations = tool
            .annotations
            .as_ref()
            .unwrap_or_else(|| panic!("{name} annotations"));
        assert_eq!(annotations.read_only_hint, Some(read_only));
        assert_eq!(annotations.destructive_hint, Some(false));
    }
    Ok(())
}

/// The service is built before the switch flips, so the change has to reach the next
/// tools/list without a reconnect. Every other tool stays listed.
#[tokio::test]
async fn tests_that_turning_human_help_off_hides_both_help_tools() -> anyhow::Result<()> {
    let call = desktop_playwright_call("tabs", json!({})).await?;
    let service = ClawMcpService::new(call.state.clone());
    for declare_agent_name in [false, true] {
        assert_eq!(
            tool_names(&service, declare_agent_name),
            ["run", REQUEST_HELP_TOOL_NAME, AWAIT_HELP_TOOL_NAME]
        );
    }

    call.state
        .agent_settings
        .set_human_help(HumanHelp::Off)
        .await?;

    let expected = vec!["run".to_string()];
    for declare_agent_name in [false, true] {
        assert_eq!(tool_names(&service, declare_agent_name), expected);
    }
    Ok(())
}

#[tokio::test]
async fn internal_name_session_schema_and_annotations_remain_available() -> anyhow::Result<()> {
    let call = desktop_playwright_call("tabs", json!({})).await?;
    let service = ClawMcpService::new(call.state);
    assert!(service.get_tool(NAME_SESSION_TOOL_NAME).is_none());
    let listed = session_handle::offer(service.name_session_tool.clone());
    assert_eq!(
        listed.description.as_deref(),
        Some(NAME_SESSION_DESCRIPTION)
    );
    assert_eq!(
        Value::Object(listed.input_schema.as_ref().clone()),
        json!({
            "type": "object",
            "properties": {
                "name": { "type": "string", "maxLength": 64 },
                "category": {
                    "type": "string",
                    "enum": crate::analytics::events::TASK_CATEGORY_VALUES,
                    "description": NAME_SESSION_CATEGORY_DESCRIPTION
                },
                "summary": {
                    "type": "string",
                    "maxLength": SUMMARY_MAX_LEN,
                    "description": NAME_SESSION_SUMMARY_DESCRIPTION
                },
                "session": { "type": "string", "description": session_handle::SESSION_ARG_DESCRIPTION }
            },
            "required": ["name"]
        })
    );
    assert_eq!(
        listed.annotations,
        Some(
            ToolAnnotations::with_title("Name session")
                .read_only(false)
                .destructive(false)
                .idempotent(true)
        )
    );
    Ok(())
}

#[tokio::test]
async fn internal_save_skill_keeps_its_annotations() -> anyhow::Result<()> {
    let call = desktop_playwright_call("tabs", json!({})).await?;
    let service = ClawMcpService::new(call.state);
    assert!(service.get_tool(SAVE_SKILL_TOOL_NAME).is_none());
    let listed = service.save_skill_offer();
    // Ultrafast extends the description and schema; the base tool is the same either way.
    assert!(
        listed
            .description
            .as_deref()
            .is_some_and(|description| description.starts_with(SAVE_SKILL_DESCRIPTION))
    );
    let properties = &listed.input_schema["properties"];
    assert_eq!(properties.get("flow").is_some(), service.ultrafast());
    assert_eq!(
        listed.annotations,
        Some(
            ToolAnnotations::with_title("Save skill")
                .read_only(false)
                .destructive(false)
                .idempotent(true)
        )
    );
    Ok(())
}

#[tokio::test]
async fn internal_mark_skill_run_keeps_its_annotations() -> anyhow::Result<()> {
    let call = desktop_playwright_call("tabs", json!({})).await?;
    let service = ClawMcpService::new(call.state);
    assert!(service.get_tool(MARK_SKILL_RUN_TOOL_NAME).is_none());
    let listed = service.mark_skill_run_tool.clone();
    assert_eq!(
        listed.description.as_deref(),
        Some(MARK_SKILL_RUN_DESCRIPTION)
    );
    assert_eq!(
        listed.annotations,
        Some(
            ToolAnnotations::with_title("Mark skill run")
                .read_only(false)
                .destructive(false)
                .idempotent(true)
        )
    );
    Ok(())
}

#[tokio::test]
async fn playwright_keeps_skill_tools_private_when_ultrafast_changes() -> anyhow::Result<()> {
    let call = desktop_playwright_call("tabs", json!({})).await?;
    let service = ClawMcpService::new(call.state.clone());
    for ultrafast in [false, true] {
        call.state.ultrafast.set(ultrafast);
        let names = tool_names(&service, false);
        for name in [SAVE_SKILL_TOOL_NAME, RUN_SKILL_TOOL_NAME] {
            assert!(!names.iter().any(|listed| listed == name));
            assert!(service.get_tool(name).is_none());
        }
        // The shared catalog offer gains flow even though Playwright never exposes it.
        let save = service.save_skill_offer();
        assert_eq!(
            save.input_schema["properties"].get("flow").is_some(),
            ultrafast
        );
    }
    Ok(())
}

#[tokio::test]
async fn pursue_and_its_guidance_follow_the_ultrafast_switch() -> anyhow::Result<()> {
    let call = desktop_playwright_call("tabs", json!({})).await?;
    let mut state = call.state.clone();
    state.decisions = Arc::new(crate::services::decisions::Decisions::new(Some(
        "http://127.0.0.1:9/".parse()?,
    )));
    let service = ClawMcpService::new(state.clone());
    let check = |service: &ClawMcpService, expected: bool| -> anyhow::Result<()> {
        let tools = service.listed_tools(false);
        assert_eq!(
            tools.iter().any(|tool| tool.name == PURSUE_TOOL_NAME),
            expected
        );
        assert_eq!(service.get_tool(PURSUE_TOOL_NAME).is_some(), expected);
        let run = tools
            .iter()
            .find(|tool| tool.name == "run")
            .ok_or_else(|| anyhow::anyhow!("run missing"))?;
        let description = run.description.as_deref().unwrap_or_default();
        let sdk = service.state.neo.tool_snapshot();
        assert!(description.ends_with(&sdk.description));
        assert_eq!(description.starts_with("With pursue listed"), expected);
        let instructions = service.get_info().instructions.unwrap_or_default();
        assert_eq!(
            instructions.contains("give pursue the whole outcome"),
            expected
        );
        Ok(())
    };
    check(&service, false)?;
    state.ultrafast.set(true);
    check(&service, true)?;
    state.ultrafast.set(false);
    check(&service, false)?;
    state.decisions = Arc::new(crate::services::decisions::Decisions::new(None));
    state.ultrafast.set(true);
    check(&ClawMcpService::new(state), false)?;
    Ok(())
}

/// The tools, descriptions and instructions an agent without pursue sees stay as they were
/// before pursue steering: off the plan, and on it without a decisions worker.
#[tokio::test]
async fn tools_without_pursue_stay_as_they_were() -> anyhow::Result<()> {
    let call = desktop_playwright_call("tabs", json!({})).await?;
    let mut state = call.state.clone();
    let surface = |service: &ClawMcpService| -> anyhow::Result<String> {
        let mut text = String::new();
        for tool in service.listed_tools(false) {
            text.push_str(&format!(
                "## {}\n{}\n{}\n\n",
                tool.name,
                tool.description.as_deref().unwrap_or_default(),
                serde_json::to_string(&tool.input_schema)?
            ));
        }
        let instructions = service.get_info().instructions.unwrap_or_default();
        Ok(format!("{text}=== instructions\n{instructions}\n"))
    };

    state.decisions = Arc::new(crate::services::decisions::Decisions::new(Some(
        "http://127.0.0.1:9/".parse()?,
    )));
    let free = ClawMcpService::new(state.clone());
    assert!(free.get_tool(PURSUE_TOOL_NAME).is_none());
    crate::free_path::assert_unchanged("playwright-tools.txt", &surface(&free)?)
        .map_err(|error| anyhow::anyhow!("{error}"))?;

    state.decisions = Arc::new(crate::services::decisions::Decisions::new(None));
    state.ultrafast.set(true);
    let without_worker = ClawMcpService::new(state);
    assert!(without_worker.get_tool(PURSUE_TOOL_NAME).is_none());
    crate::free_path::assert_unchanged(
        "playwright-tools-ultrafast-without-pursue.txt",
        &surface(&without_worker)?,
    )
    .map_err(|error| anyhow::anyhow!("{error}"))
}
