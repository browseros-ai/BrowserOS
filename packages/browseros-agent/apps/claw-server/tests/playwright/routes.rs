//! MCP HTTP coverage for desktop servers that opt into the Playwright run.
//! The shared route fixtures leave the SDK child stopped so initialization and refusal
//! assertions do not depend on a live browser.

use super::*;
use claw_server::{api::mcp::ClawMcpService, config::Behavior};
use rmcp::handler::server::ServerHandler;

async fn playwright_app() -> anyhow::Result<TestApp> {
    playwright_app_with_native(current_native()).await
}

async fn playwright_app_with_native(native: NativeCapabilities) -> anyhow::Result<TestApp> {
    test_app_with_behavior(
        49337,
        false,
        native,
        ServerMode::Desktop,
        Behavior {
            playwright_run: true,
            ..Behavior::defaults(ServerMode::Desktop)
        },
    )
    .await
}

async fn initialize_playwright_mcp(app: &TestApp) -> anyhow::Result<String> {
    initialize_mcp_with(app, |body| {
        assert_playwright_instructions(
            app,
            body["result"]["instructions"].as_str().unwrap_or_default(),
        );
    })
    .await
}

fn assert_playwright_instructions(app: &TestApp, instructions: &str) {
    assert!(
        instructions.contains("Use run for browser work."),
        "{instructions}"
    );
    assert_eq!(
        Some(instructions),
        ClawMcpService::new(app.state.clone())
            .get_info()
            .instructions
            .as_deref()
    );
}

/// Check first discovery when the launch config reports an older or unknown native version.
#[tokio::test]
async fn older_native_lists_and_teaches_no_profile_surface() -> anyhow::Result<()> {
    for native in [
        NativeCapabilities::from_browseros_version(Some("0.51.1.2")),
        NativeCapabilities::default(),
    ] {
        let app = playwright_app_with_native(native).await?;
        let initialize = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": { "name": "Codex", "version": "1.0" }
            }
        });
        let (status, headers, body) =
            request_json_with_headers(&app.router, "POST", "/mcp", Some(initialize), &[]).await?;
        assert_eq!(status, StatusCode::OK, "{body:?}");
        let instructions = body["result"]["instructions"].as_str().unwrap_or_default();
        assert_playwright_instructions(&app, instructions);
        assert!(!instructions.contains("browser_profiles"), "{native}");
        assert!(!instructions.contains("browser_target"), "{native}");
        let session_id = headers
            .get("mcp-session-id")
            .and_then(|value| value.to_str().ok())
            .map(str::to_string)
            .ok_or_else(|| anyhow::anyhow!("missing mcp-session-id"))?;
        send_initialized(&app.router, &session_id).await?;
        wait_for_session_registration(&app, &session_id).await?;

        let list = json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {} });
        let (status, _headers, body) = request_json_with_headers(
            &app.router,
            "POST",
            "/mcp",
            Some(list),
            &[("mcp-session-id", &session_id)],
        )
        .await?;
        assert_eq!(status, StatusCode::OK);
        let tools = body["result"]["tools"]
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("tools not array"))?;
        let names = tools
            .iter()
            .filter_map(|tool| tool["name"].as_str())
            .collect::<Vec<_>>();
        assert!(!names.contains(&"browser_profiles"), "{native}: {names:?}");
        assert!(!names.contains(&"browser_target"), "{native}: {names:?}");
        assert_eq!(names, ["run", "request_human_help", "await_human_help"]);
        for tool in tools {
            let description = tool["description"].as_str().unwrap_or_default();
            assert!(
                !description.contains("browser_profiles")
                    && !description.contains("browser_target"),
                "{native}: {} still teaches profile tools",
                tool["name"]
            );
        }
    }
    Ok(())
}

#[tokio::test]
async fn mcp_initialize_list_guard_audit_and_delete() -> anyhow::Result<()> {
    let app = playwright_app().await?;
    let session_id = initialize_playwright_mcp(&app).await?;

    let list = json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/list",
        "params": {}
    });
    let (status, _headers, body) = request_json_with_headers(
        &app.router,
        "POST",
        "/mcp",
        Some(list),
        &[("mcp-session-id", &session_id)],
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    let listed: Vec<&str> = body["result"]["tools"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("tools not array"))?
        .iter()
        .filter_map(|tool| tool["name"].as_str())
        .collect();
    let expected = vec!["run", "request_human_help", "await_human_help"];
    assert_eq!(listed, expected);

    let blocked = json!({
        "jsonrpc": "2.0",
        "id": 3,
        "method": "tools/call",
        "params": {
            "name": "navigate",
            "arguments": { "page": 1, "url": "javascript:alert(1)" }
        }
    });
    let (status, _headers, body) = request_json_with_headers(
        &app.router,
        "POST",
        "/mcp",
        Some(blocked),
        &[("mcp-session-id", &session_id)],
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["result"]["isError"], true, "navigate body: {body:?}");
    assert!(
        body["result"]["content"][0]["text"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("missing text"))?
            .contains("run")
    );

    let dispatches = app
        .state
        .audit_log
        .list_dispatches(Default::default())
        .await?;
    assert!(
        dispatches.rows.is_empty(),
        "removed tools must skip audit effects"
    );

    let (status, _headers, _body) = request_json_with_headers(
        &app.router,
        "DELETE",
        "/mcp",
        None,
        &[("mcp-session-id", &session_id)],
    )
    .await?;
    assert_eq!(status, StatusCode::ACCEPTED);

    let (status, _headers, stale) = request_json_with_headers(
        &app.router,
        "POST",
        "/mcp",
        Some(json!({"jsonrpc":"2.0","id":4,"method":"tools/list","params":{}})),
        &[("mcp-session-id", &session_id)],
    )
    .await?;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(
        stale
            .as_str()
            .is_some_and(|body| body.contains("Session not found"))
    );
    Ok(())
}

/// The cockpit switch and MCP share one setting: turning it off over HTTP is what the next
/// initialize, tools/list, and tools/call see.
#[tokio::test]
async fn tests_that_turning_human_help_off_over_http_reaches_mcp_agents() -> anyhow::Result<()> {
    let app = playwright_app().await?;
    let (status, _headers, body) = request_json_with_headers(
        &app.router,
        "PUT",
        "/api/v1/settings/agent",
        Some(json!({ "humanHelpEnabled": false })),
        &[],
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "settings body: {body:?}");

    let initialize = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": { "name": "Codex", "version": "1.0" }
        }
    });
    let (status, headers, body) =
        request_json_with_headers(&app.router, "POST", "/mcp", Some(initialize), &[]).await?;
    assert_eq!(status, StatusCode::OK, "initialize body: {body:?}");
    let instructions = body["result"]["instructions"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("missing instructions"))?;
    assert_playwright_instructions(&app, instructions);
    assert!(instructions.contains("ask in chat and stop work on that page"));
    assert!(!instructions.contains("request_human_help"));
    let session_id = headers
        .get("mcp-session-id")
        .and_then(|value| value.to_str().ok())
        .map(str::to_string)
        .ok_or_else(|| anyhow::anyhow!("missing mcp-session-id"))?;
    send_initialized(&app.router, &session_id).await?;
    wait_for_session_registration(&app, &session_id).await?;

    let list = json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {} });
    let (status, _headers, body) = request_json_with_headers(
        &app.router,
        "POST",
        "/mcp",
        Some(list),
        &[("mcp-session-id", &session_id)],
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    let listed: Vec<&str> = body["result"]["tools"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("tools not array"))?
        .iter()
        .filter_map(|tool| tool["name"].as_str())
        .collect();
    assert!(listed.contains(&"run"), "{listed:?}");
    assert!(!listed.contains(&"request_human_help"), "{listed:?}");
    assert!(!listed.contains(&"await_human_help"), "{listed:?}");

    let ask = json!({
        "jsonrpc": "2.0",
        "id": 3,
        "method": "tools/call",
        "params": {
            "name": "request_human_help",
            "arguments": { "reason": "Enter the code" }
        }
    });
    let (status, _headers, body) = request_json_with_headers(
        &app.router,
        "POST",
        "/mcp",
        Some(ask),
        &[("mcp-session-id", &session_id)],
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["result"]["isError"], true, "call body: {body:?}");
    assert!(
        body["result"]["content"][0]["text"]
            .as_str()
            .is_some_and(|text| text.contains("turned off")),
        "call body: {body:?}"
    );
    Ok(())
}

#[tokio::test]
async fn run_skill_is_refused_even_for_a_skill_with_a_verified_flow() -> anyhow::Result<()> {
    let app = playwright_app().await?;
    app.state
        .skills
        .create(claw_server::services::skills::CreateSkill {
            name: "neo-shop-add".to_string(),
            description: "Add a product to the cart".to_string(),
            site: None,
            steps: vec!["Open the search".to_string(), "Add it".to_string()],
            learned_notes: vec![],
            origin: claw_server::services::skills::SkillOrigin::Agent,
            source_session_id: None,
        })
        .await?;
    app.state
        .skills
        .put_flow_row(claw_server::services::flow::new_row(
            "neo-shop-add",
            claw_server::services::flow::FlowStatus::Verified,
        ))
        .await?;
    let session_id = initialize_playwright_mcp(&app).await?;

    let call = json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/call",
        "params": { "name": "run_skill", "arguments": { "name": "neo-shop-add" } }
    });
    let (status, _headers, body) = request_json_with_headers(
        &app.router,
        "POST",
        "/mcp",
        Some(call),
        &[("mcp-session-id", &session_id)],
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["result"]["isError"], true, "{body}");
    assert!(
        body["result"]["content"][0]["text"]
            .as_str()
            .is_some_and(|text| text.contains("run")),
        "{body}"
    );

    let runs = app.state.skills.get("neo-shop-add").await?.runs;
    assert!(runs.is_empty());
    Ok(())
}

#[tokio::test]
async fn tests_that_explicit_grouping_never_sends_the_cleanup_tip() -> anyhow::Result<()> {
    use claw_server::config::TabGrouping;
    let app = test_app_with_behavior(
        49337,
        false,
        current_native(),
        ServerMode::Desktop,
        Behavior {
            playwright_run: true,
            tab_grouping: TabGrouping::Explicit,
            ..Behavior::default()
        },
    )
    .await?;
    let (status, _) = request_json(
        &app.router,
        "PUT",
        "/api/v1/settings/agent",
        Some(json!({ "taskCleanupTipEnabled": true })),
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    let reply = stateless_mcp_request(
        &app.router,
        "tools/call",
        json!({ "name": "await_human_help", "arguments": {} }),
        Some("Codex"),
    )
    .await?;
    assert_eq!(reply["result"]["isError"], false, "{reply}");
    let text = reply["result"]["content"].to_string();
    assert!(!text.contains("tab group"), "{reply}");
    Ok(())
}

#[tokio::test]
async fn mcp_tool_calls_are_counted_for_the_day() -> anyhow::Result<()> {
    let app = playwright_app().await?;
    let started_ms = epoch_ms()?;
    for name in ["await_human_help", "await_human_help", "request_human_help"] {
        stateless_mcp_request(
            &app.router,
            "tools/call",
            json!({ "name": name, "arguments": {} }),
            Some("Codex"),
        )
        .await?;
    }
    // A test that crosses UTC midnight records on both days.
    let mut days = vec![
        claw_server::db::run_error_budget::utc_day(started_ms),
        claw_server::db::run_error_budget::utc_day(epoch_ms()?),
    ];
    days.dedup();
    let (mut calls, mut error_calls) = (0, 0);
    for day in days {
        for count in app.state.tool_calls_daily.counts(&day).await? {
            assert!(["await_human_help", "request_human_help"].contains(&count.tool_name.as_str()));
            calls += count.calls;
            error_calls += count.error_calls;
        }
    }
    assert_eq!((calls, error_calls), (3, 1));
    Ok(())
}

#[tokio::test]
async fn mcp_stateless_product_precedence_fallback_and_reuse() -> anyhow::Result<()> {
    let app = playwright_app().await?;
    for (reported, declared, expected) in [
        (Some("Claude Code"), Some("cld-messaging"), "claude/task"),
        (Some("codex-mcp-client"), Some("claude"), "codex/task"),
        (Some("unknown-proxy"), Some("OpenCode"), "opencode/task"),
        (None, Some("VS Code"), "vscode/task"),
        (None, Some("cld-account2"), "cld-account2/task"),
        (Some("custom-mcp-proxy"), None, "custom-mcp-proxy/task"),
        (None, None, "agent/task"),
    ] {
        let mut arguments = json!({});
        if let Some(name) = declared {
            arguments["agentName"] = json!(name);
        }
        let body = stateless_mcp_request(
            &app.router,
            "tools/call",
            json!({ "name": "await_human_help", "arguments": arguments }),
            reported,
        )
        .await?;
        let session = session_from_result(&app, &body).await?;
        assert_eq!(
            claw_server::api::mcp::naming::session_group_title(&session, "task"),
            expected
        );
    }
    let first = stateless_mcp_request(
        &app.router,
        "tools/call",
        json!({ "name": "await_human_help", "arguments": { "agentName": "cld-messaging" } }),
        Some("Claude Code"),
    )
    .await?;
    let handle = first["result"]["_meta"]["com.browseros.neo/session"].clone();
    for reported in [None, Some("codex-mcp-client")] {
        let reused = stateless_mcp_request(&app.router, "tools/call",
            json!({ "name": "await_human_help", "arguments": { "session": handle, "agentName": "different-worker" } }), reported).await?;
        let session = session_from_result(&app, &reused).await?;
        assert_eq!(
            claw_server::api::mcp::naming::session_group_title(&session, "task"),
            "claude/task"
        );
        assert_eq!(
            reused["result"]["_meta"]["com.browseros.neo/session"],
            handle
        );
    }
    Ok(())
}

async fn session_from_result(app: &TestApp, result: &Value) -> anyhow::Result<Arc<Session>> {
    let handle = result["result"]["_meta"]["com.browseros.neo/session"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("missing session handle: {result}"))?;
    app.state
        .sessions
        .lookup(&SessionId::new(handle))
        .await
        .ok_or_else(|| anyhow::anyhow!("missing session {handle}"))
}

#[tokio::test]
async fn mcp_declared_product_survives_profile_resolution_and_reuse() -> anyhow::Result<()> {
    let app = playwright_app().await?;
    let profiles_dir = app.state.config.browserclaw_dir.join("agents");
    tokio::fs::create_dir_all(&profiles_dir).await?;
    tokio::fs::write(
        profiles_dir.join("codex-profile.json"),
        json!({
            "id": "codex-profile", "name": "Codex", "slug": "mcp",
            "harness": "Codex", "loginMode": "profile", "selectedSites": [],
            "approvals": {}, "aclRuleIds": [], "customAclRules": [],
            "mcpUrl": "http://127.0.0.1:9200/mcp", "status": "configured",
            "createdAt": "now", "updatedAt": "now"
        })
        .to_string(),
    )
    .await?;

    let first = stateless_mcp_request(
        &app.router,
        "tools/call",
        json!({ "name": "await_human_help", "arguments": { "agentName": "Codex" } }),
        Some("unknown-proxy"),
    )
    .await?;

    let handle = first["result"]["_meta"]["com.browseros.neo/session"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("missing session handle: {first}"))?;
    let session = app
        .state
        .sessions
        .lookup(&SessionId::new(handle))
        .await
        .ok_or_else(|| anyhow::anyhow!("missing session"))?;
    assert_eq!(
        session.agent().profile_id().map(ProfileId::as_str),
        Some("codex-profile")
    );
    assert_eq!(session.agent().slug(), "mcp");
    assert!(session.convo_id().as_str().starts_with("mcp-"));
    assert_eq!(session.client_name(), "unknown-proxy");

    let reused = stateless_mcp_request(
        &app.router,
        "tools/call",
        json!({ "name": "await_human_help", "arguments": { "session": handle, "agentName": "claude" } }),
        None,
    ).await?;
    assert_eq!(
        reused["result"]["_meta"]["com.browseros.neo/session"],
        handle
    );
    assert_eq!(
        claw_server::api::mcp::naming::session_group_title(&session, "continued-task"),
        "codex/continued-task"
    );
    Ok(())
}

/// Even an unavailable SDK run must return the model's task handle. The live
/// contract covers structured SDK values; this route stays independent of a child process.
#[tokio::test]
async fn mcp_stateless_disabled_run_keeps_the_session_handle() -> anyhow::Result<()> {
    let app = playwright_app().await?;
    let listed =
        stateless_mcp_request(&app.router, "tools/list", json!({}), Some("claude-code")).await?;
    let run = listed["result"]["tools"]
        .as_array()
        .and_then(|tools| tools.iter().find(|tool| tool["name"] == "run"))
        .ok_or_else(|| anyhow::anyhow!("run missing"))?;
    for (field, kind) in [
        ("code", "string"),
        ("timeout", "integer"),
        ("session", "string"),
        ("agentName", "string"),
    ] {
        assert_eq!(run["inputSchema"]["properties"][field]["type"], kind);
    }
    assert_eq!(run["inputSchema"]["additionalProperties"], false);
    assert_eq!(run["outputSchema"], Value::Null);

    let first = stateless_mcp_request(
        &app.router,
        "tools/call",
        json!({ "name": "run", "arguments": { "code": "return 1" } }),
        Some("claude-code"),
    )
    .await?;
    assert_eq!(first["result"]["isError"], true);
    let handle = first["result"]["_meta"]["com.browseros.neo/session"].clone();
    assert!(handle.is_string());
    assert_eq!(handle.as_str().map(str::len), Some(8));
    let first_text = first["result"]["content"].to_string();
    assert!(first_text.contains("Playwright engine has not been started"));
    assert!(first_text.contains("Pass this exact value"));
    assert!(first_text.contains(handle.as_str().unwrap_or_default()));

    let second = stateless_mcp_request(
        &app.router,
        "tools/call",
        json!({
            "name": "run",
            "arguments": { "code": "return 2", "session": handle }
        }),
        Some("claude-code"),
    )
    .await?;
    assert_eq!(second["result"]["isError"], true);
    assert_eq!(
        second["result"]["_meta"]["com.browseros.neo/session"],
        handle
    );
    let compact = format!(
        "[browseros-neo session: {}]",
        handle.as_str().unwrap_or_default()
    );
    assert!(
        second["result"]["content"]
            .as_array()
            .is_some_and(|blocks| blocks.iter().any(|block| block["text"] == compact))
    );
    assert!(
        !second["result"]["content"]
            .to_string()
            .contains("Pass this exact value")
    );
    assert_eq!(app.state.sessions.count().await, 1);
    Ok(())
}

#[tokio::test]
async fn mcp_stateless_tool_schema_has_one_optional_product_fallback() -> anyhow::Result<()> {
    let app = playwright_app().await?;
    let anonymous = stateless_mcp_request(&app.router, "tools/list", json!({}), None).await?;
    let identified =
        stateless_mcp_request(&app.router, "tools/list", json!({}), Some("Claude Code")).await?;
    assert_eq!(anonymous["result"]["tools"], identified["result"]["tools"]);
    let tools = anonymous["result"]["tools"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("missing tools: {anonymous}"))?;
    assert!(!tools.is_empty());
    for tool in tools {
        let schema = &tool["inputSchema"];
        assert_eq!(schema["properties"]["agentName"]["type"], "string");
        assert!(
            !schema["required"]
                .as_array()
                .is_some_and(|required| required.contains(&json!("agentName"))),
            "{tool}"
        );
        let description = schema["properties"]["agentName"]["description"]
            .as_str()
            .unwrap_or_default();
        assert!(description.contains("client application"), "{description}");
        assert!(description.contains("neo.createTask"), "{description}");
    }
    Ok(())
}

#[tokio::test]
async fn mcp_stateless_metadata_names_session_without_agent_argument() -> anyhow::Result<()> {
    let app = playwright_app().await?;
    // Exercise the real JSON decoding and rmcp dispatch seam: incoming _meta is
    // moved into RequestContext, which a direct resolver call cannot reproduce.
    let (status, headers, body) = request_json_with_headers(
        &app.router,
        "POST",
        "/mcp",
        Some(json!({
            "jsonrpc": "2.0", "id": 1, "method": "tools/call",
            "params": {
                "_meta": {
                    "io.modelcontextprotocol/protocolVersion": "2026-07-28",
                    "io.modelcontextprotocol/clientCapabilities": {},
                    "io.modelcontextprotocol/clientInfo": {
                        "name": "codex-mcp-client", "version": "1.0"
                    }
                },
                "name": "await_human_help",
                "arguments": {}
            }
        })),
        &[
            ("mcp-protocol-version", "2026-07-28"),
            ("mcp-method", "tools/call"),
            ("mcp-name", "await_human_help"),
        ],
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(!headers.contains_key("mcp-session-id"));
    let session = session_from_result(&app, &body).await?;
    assert_eq!(session.client_name(), "codex-mcp-client");
    assert_eq!(
        claw_server::api::mcp::naming::session_group_title(&session, "metadata-check"),
        "codex/metadata-check"
    );
    Ok(())
}

#[tokio::test]
async fn desktop_cached_tools_are_refused_before_a_stateless_session_starts() -> anyhow::Result<()>
{
    let app = playwright_app().await?;
    for name in [
        "name_session",
        "tabs",
        "snapshot",
        "save_skill",
        "mark_skill_run",
        "run_skill",
        "solve_captcha",
    ] {
        let body = stateless_mcp_request(
            &app.router,
            "tools/call",
            json!({"name":name,"arguments":{}}),
            Some("Codex"),
        )
        .await?;
        assert_eq!(body["result"]["isError"], true, "{body}");
        assert!(
            body["result"]["content"][0]["text"]
                .as_str()
                .is_some_and(|text| text.contains("run")),
            "{body}"
        );
        assert!(
            body["result"]["_meta"]["com.browseros.neo/session"].is_null(),
            "{body}"
        );
    }
    assert_eq!(app.state.sessions.count().await, 0);
    assert!(
        app.state
            .audit_log
            .list_tasks(Default::default())
            .await?
            .tasks
            .is_empty()
    );
    Ok(())
}

/// Playwright agents name their tasks through the SDK. The shared MCP handler still
/// validates taskName, and hosted catalog calls continue to use it to name a session.
#[tokio::test]
async fn task_name_names_only_hosted_sessions() -> anyhow::Result<()> {
    for app in [hosted_test_app().await?, playwright_app().await?] {
        let mode = app.state.config.mode;
        let tool = if mode.is_hosted() {
            "tabs"
        } else {
            "await_human_help"
        };
        let first = stateless_mcp_request(
            &app.router,
            "tools/call",
            json!({ "name": tool, "arguments": {
                "action": "list", "taskName": "Invoice Processing"
            } }),
            Some("Claude Code"),
        )
        .await?;
        let handle = first["result"]["_meta"]["com.browseros.neo/session"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("{mode:?}: missing session handle: {first}"))?;
        let session = app
            .state
            .sessions
            .lookup(&SessionId::new(handle))
            .await
            .ok_or_else(|| anyhow::anyhow!("{mode:?}: missing session"))?;
        let label = session.label().await;
        assert_eq!(label == "invoice-processing", mode.is_hosted(), "{mode:?}");

        let rejected = stateless_mcp_request(
            &app.router,
            "tools/call",
            json!({ "name": tool, "arguments": {
                "action": "list", "session": handle, "taskName": "!!!"
            } }),
            Some("Claude Code"),
        )
        .await?;
        assert_eq!(rejected["result"]["isError"], true, "{rejected}");
        let text = rejected["result"]["content"][0]["text"]
            .as_str()
            .unwrap_or_default();
        assert!(text.contains("taskName"), "{mode:?}: {text}");
        assert_eq!(session.label().await, label, "{mode:?}");
    }
    Ok(())
}
