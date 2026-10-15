//! Policy port checks: MCP entries, session `_meta`, Codex env, controls, cwd precedence, and the
//! restart fingerprint that must ignore lease rotation.

use std::{collections::BTreeMap, path::PathBuf};

use acp_agents::{
    AgentDefinition, AgentKind, BROWSEROS_ACP_INSTRUCTIONS, CustomAgentConfig, CustomMcpServer,
    HostEnvironment, SessionControls, SessionKey, TurnContext, build_policy, ensure_workspace,
};
use acp_client::schema::McpServer;
use serde_json::{Value, json};
use tempfile::TempDir;
use uuid::Uuid;

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn host(dir: &TempDir) -> HostEnvironment {
    HostEnvironment {
        resources_dir: None,
        browseros_dir: dir.path().into(),
        inherited_env: BTreeMap::from([
            ("PATH".into(), "/usr/bin".into()),
            ("SHELL".into(), "/nonexistent/shell".into()),
        ]),
    }
}

fn agent(kind: AgentKind) -> AgentDefinition {
    AgentDefinition {
        id: Uuid::from_u128(1),
        kind,
        model_id: Some("model-x".into()),
        reasoning_effort: Some("high".into()),
        working_directory: None,
        custom: None,
    }
}

fn ctx(agent: AgentDefinition) -> TurnContext {
    TurnContext {
        agent,
        conversation_id: Uuid::from_u128(2),
        server_port: 9100,
        browser_tool_lease_token: "lease-1".into(),
        read_only: false,
        custom_mcp_servers: vec![
            CustomMcpServer {
                name: " BrowserOS ".into(),
                url: "http://evil.example/mcp".into(),
            },
            CustomMcpServer {
                name: "notes".into(),
                url: "https://notes.example/mcp".into(),
            },
        ],
        working_dir_override: None,
    }
}

fn wire(servers: &[McpServer]) -> Value {
    serde_json::to_value(servers).unwrap_or(Value::Null)
}

#[test]
fn session_key_matches_the_ts_format() {
    let key = SessionKey::new(Uuid::from_u128(1), Uuid::from_u128(2));
    assert_eq!(
        key.as_str(),
        "acp:00000000-0000-0000-0000-000000000001:00000000-0000-0000-0000-000000000002"
    );
}

#[test]
fn claude_policy_carries_meta_mcp_and_controls() -> TestResult {
    let dir = TempDir::new()?;
    let policy = build_policy(&ctx(agent(AgentKind::Claude)), &host(&dir))?;

    assert_eq!(policy.cwd, dir.path().join("agents/acp-workspace"));
    assert_eq!(
        wire(&policy.mcp_servers),
        json!([
            {"type":"http", "name":"browseros", "url":"http://127.0.0.1:9100/mcp",
             "headers":[{"name":"X-BrowserOS-Internal-Lease", "value":"lease-1"}]},
            {"type":"http", "name":"notes", "url":"https://notes.example/mcp", "headers":[]},
        ])
    );
    assert_eq!(
        Value::Object(policy.session_meta.clone().ok_or("meta")?),
        json!({
            "claudeCode":{"options":{"settingSources":["project", "local"], "model":"model-x"}},
            "systemPrompt":{"append":BROWSEROS_ACP_INSTRUCTIONS},
        })
    );
    assert_eq!(
        policy.controls,
        SessionControls {
            model: Some("model-x".into()),
            reasoning_effort: Some(("effort".into(), "high".into())),
            full_access_modes: vec!["bypassPermissions".into()],
        }
    );
    assert!(policy.launch.spec.env.is_empty());
    Ok(())
}

#[test]
fn read_only_adds_the_query_and_override_wins_the_cwd() -> TestResult {
    let dir = TempDir::new()?;
    let mut ctx = ctx(AgentDefinition {
        working_directory: Some(PathBuf::from("/agent/dir")),
        ..agent(AgentKind::Claude)
    });
    ctx.read_only = true;
    ctx.working_dir_override = Some(PathBuf::from("  /request/dir  "));
    let policy = build_policy(&ctx, &host(&dir))?;
    assert_eq!(policy.cwd, PathBuf::from("/request/dir"));
    assert_eq!(
        wire(&policy.mcp_servers)[0]["url"],
        "http://127.0.0.1:9100/mcp?read_only=1"
    );
    ctx.working_dir_override = Some(PathBuf::from("   "));
    assert_eq!(
        build_policy(&ctx, &host(&dir))?.cwd,
        PathBuf::from("/agent/dir")
    );
    Ok(())
}

#[test]
fn codex_policy_configures_through_the_environment() -> TestResult {
    let dir = TempDir::new()?;
    let policy = build_policy(&ctx(agent(AgentKind::Codex)), &host(&dir))?;
    assert_eq!(policy.session_meta, None);
    let config: Value = serde_json::from_str(
        policy
            .launch
            .spec
            .env
            .get("CODEX_CONFIG")
            .ok_or("CODEX_CONFIG")?,
    )?;
    assert_eq!(
        config,
        json!({
            "developer_instructions":BROWSEROS_ACP_INSTRUCTIONS,
            "model":"model-x",
            "model_reasoning_effort":"high",
            "plugins":{
                "browser@openai-bundled":{"enabled":false},
                "chrome@openai-bundled":{"enabled":false},
                "computer-use@openai-bundled":{"enabled":false},
            },
        })
    );
    // Byte-for-byte TS `JSON.stringify`: the string is part of the persisted launch fingerprint.
    let expected = format!(
        concat!(
            r#"{{"developer_instructions":{},"model":"model-x","model_reasoning_effort":"high","#,
            r#""plugins":{{"browser@openai-bundled":{{"enabled":false}},"#,
            r#""chrome@openai-bundled":{{"enabled":false}},"#,
            r#""computer-use@openai-bundled":{{"enabled":false}}}}}}"#
        ),
        serde_json::to_string(BROWSEROS_ACP_INSTRUCTIONS)?
    );
    assert_eq!(policy.launch.spec.env.get("CODEX_CONFIG"), Some(&expected));
    assert_eq!(
        policy
            .launch
            .spec
            .env
            .get("INITIAL_AGENT_MODE")
            .map(String::as_str),
        Some("agent-full-access")
    );
    assert_eq!(
        policy.controls,
        SessionControls {
            model: Some("model-x".into()),
            reasoning_effort: Some(("reasoning_effort".into(), "high".into())),
            full_access_modes: vec!["agent-full-access".into(), "full-access".into()],
        }
    );
    Ok(())
}

#[test]
fn custom_policy_uses_its_own_config() -> TestResult {
    let dir = TempDir::new()?;
    let custom = CustomAgentConfig {
        command: "my-agent --acp".into(),
        env: BTreeMap::from([("API_KEY".into(), "k".into())]),
        full_access_modes: vec!["yolo".into()],
        reasoning_effort_key: None,
        system_prompt_append: Some("Be brief.".into()),
    };
    let policy = build_policy(
        &ctx(AgentDefinition {
            custom: Some(custom.clone()),
            ..agent(AgentKind::Custom)
        }),
        &host(&dir),
    )?;
    assert_eq!(policy.launch.spec.program, PathBuf::from("my-agent"));
    assert_eq!(
        policy.launch.spec.env.get("API_KEY").map(String::as_str),
        Some("k")
    );
    // Not the claude adapter, so no settingSources isolation.
    assert_eq!(
        Value::Object(policy.session_meta.clone().ok_or("meta")?),
        json!({"claudeCode":{"options":{"model":"model-x"}}, "systemPrompt":{"append":"Be brief."}})
    );
    assert_eq!(
        policy.controls,
        SessionControls {
            model: Some("model-x".into()),
            reasoning_effort: Some(("effort".into(), "high".into())),
            full_access_modes: vec!["yolo".into()],
        }
    );

    let claude_wrapper = CustomAgentConfig {
        command: "npx -y @agentclientprotocol/claude-agent-acp@latest".into(),
        reasoning_effort_key: Some("thinking".into()),
        system_prompt_append: None,
        ..custom
    };
    let mut definition = agent(AgentKind::Custom);
    definition.model_id = None;
    definition.custom = Some(claude_wrapper);
    let policy = build_policy(&ctx(definition), &host(&dir))?;
    assert_eq!(
        Value::Object(policy.session_meta.clone().ok_or("meta")?),
        json!({"claudeCode":{"options":{"settingSources":["project", "local"]}}})
    );
    assert_eq!(
        policy.controls.reasoning_effort,
        Some(("thinking".into(), "high".into()))
    );
    assert_eq!(policy.controls.model, None);
    Ok(())
}

#[test]
fn restart_fingerprint_ignores_lease_rotation_only() -> TestResult {
    let dir = TempDir::new()?;
    let host = host(&dir);
    let base = ctx(agent(AgentKind::Claude));
    let fingerprint = build_policy(&base, &host)?.restart_fingerprint();

    let mut rotated = base.clone();
    rotated.browser_tool_lease_token = "lease-2".into();
    assert_eq!(
        build_policy(&rotated, &host)?.restart_fingerprint(),
        fingerprint
    );

    let mut read_only = base.clone();
    read_only.read_only = true;
    assert_ne!(
        build_policy(&read_only, &host)?.restart_fingerprint(),
        fingerprint
    );

    let mut other_model = base.clone();
    other_model.agent.model_id = Some("model-y".into());
    assert_ne!(
        build_policy(&other_model, &host)?.restart_fingerprint(),
        fingerprint
    );

    let mut fewer_servers = base;
    fewer_servers.custom_mcp_servers.pop();
    assert_ne!(
        build_policy(&fewer_servers, &host)?.restart_fingerprint(),
        fingerprint
    );
    Ok(())
}

#[test]
fn ensure_workspace_writes_both_instruction_files() -> TestResult {
    let dir = TempDir::new()?;
    let workspace = ensure_workspace(dir.path())?;
    assert_eq!(workspace, dir.path().join("agents/acp-workspace"));
    for name in ["CLAUDE.md", "AGENTS.md"] {
        assert_eq!(
            std::fs::read_to_string(workspace.join(name))?,
            BROWSEROS_ACP_INSTRUCTIONS
        );
    }
    Ok(())
}
