use agent_mcp_manager::{
    AgentId, AgentScope, LinkInput, Manager, McpServer, McpServerSpec,
    resolve_agent_mcp_config_path,
};
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode, header},
};
use claw_server_rust::services::harness::{Harness, HarnessService};
use claw_server_rust::{AppState, build_router, config::Config};
use serde_json::{Value, json};
use std::{env, fs, path::Path, process::Command, sync::Arc, time::Duration};
use tower::ServiceExt;

const CHILD_CASE: &str = "CLAW_CONNECTIONS_TEST_CHILD";
const TEST_HOME: &str = "CLAW_CONNECTIONS_TEST_HOME";
const MCP_URL: &str = "http://127.0.0.1:9200/mcp";

#[test]
fn connections_adapter_writes_lists_disconnects_and_heals() -> anyhow::Result<()> {
    if env::var_os(CHILD_CASE).is_some() {
        return tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()?
            .block_on(run_connections_case());
    }

    let root = tempfile::tempdir()?;
    let output = Command::new(env::current_exe()?)
        .arg("--exact")
        .arg("connections_adapter_writes_lists_disconnects_and_heals")
        .arg("--nocapture")
        .env(CHILD_CASE, "1")
        .env(TEST_HOME, root.path())
        .env("HOME", root.path())
        .env("CLAUDE_CONFIG_DIR", root.path())
        .env("XDG_CONFIG_HOME", root.path().join(".config"))
        .output()?;
    if !output.status.success() {
        anyhow::bail!(
            "connections child failed\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(())
}

async fn run_connections_case() -> anyhow::Result<()> {
    let home = env::var_os(TEST_HOME)
        .map(std::path::PathBuf::from)
        .ok_or_else(|| anyhow::anyhow!("missing test home"))?;
    let claw_dir = home.join("claw");
    let service = HarnessService::new(claw_dir.join("mcp-manager"), home.clone());
    let paths = config_paths()?;

    for (agent, path) in &paths {
        if *agent != AgentId::Antigravity {
            fs::create_dir_all(parent(path)?)?;
        }
    }

    assert_legacy_manifest_migration(&home, &paths).await?;

    let not_installed = service
        .connect_browseros(Harness::Antigravity, MCP_URL)
        .await?;
    assert!(!not_installed.installed);
    assert_eq!(
        not_installed.message,
        "Antigravity is not installed on this machine. Launch it once so the MCP config directory exists, then try again."
    );
    fs::create_dir_all(parent(path_for(&paths, AgentId::Antigravity)?)?)?;

    let initial = service.list_browseros_connections().await?;
    assert_eq!(initial.len(), 7);
    assert_eq!(
        initial
            .iter()
            .map(|state| state.harness.as_str())
            .collect::<Vec<_>>(),
        [
            "Claude Code",
            "Codex",
            "Cursor",
            "OpenCode",
            "Antigravity",
            "VS Code",
            "Zed"
        ]
    );
    assert!(initial.iter().all(|state| !state.installed));
    assert_eq!(initial[0].message, "Claude Code is not configured.");

    let claude_path = path_for(&paths, AgentId::ClaudeCode)?;
    fs::write(
        claude_path,
        r#"{"mcpServers":{"BrowserClaw":{"command":"foreign"}}}"#,
    )?;
    let claude = service
        .connect_browseros(Harness::ClaudeCode, MCP_URL)
        .await?;
    assert!(claude.installed);
    assert_eq!(claude.agent_id, AgentId::ClaudeCode);
    assert_eq!(claude.config_path.as_deref(), Some("~/.claude.json"));
    assert_eq!(
        claude.message,
        "BrowserOS registered as an MCP server in Claude Code."
    );

    let codex = service.connect_browseros(Harness::Codex, MCP_URL).await?;
    let zed = service.connect_browseros(Harness::Zed, MCP_URL).await?;
    assert!(codex.installed && zed.installed);

    let claude_json: Value = serde_json::from_str(&fs::read_to_string(claude_path)?)?;
    assert_eq!(
        claude_json["mcpServers"]["BrowserClaw"],
        json!({ "type": "http", "url": MCP_URL })
    );

    let codex_toml: toml::Value =
        toml::from_str(&fs::read_to_string(path_for(&paths, AgentId::Codex)?)?)?;
    assert_eq!(
        codex_toml["mcp_servers"]["BrowserClaw"]["url"].as_str(),
        Some(MCP_URL)
    );

    let zed_json: Value =
        serde_json::from_str(&fs::read_to_string(path_for(&paths, AgentId::Zed)?)?)?;
    assert_eq!(
        zed_json["context_servers"]["BrowserClaw"],
        json!({ "url": MCP_URL, "source": "custom", "enabled": true })
    );

    let manifest: Value = serde_json::from_str(&fs::read_to_string(
        claw_dir.join("mcp-manager/manifest.json"),
    )?)?;
    assert_eq!(manifest["version"], 1);
    assert_eq!(
        manifest["servers"]["BrowserClaw"]["links"]
            .as_object()
            .map(serde_json::Map::len),
        Some(3)
    );
    assert!(manifest["servers"]["BrowserClaw"]["addedAt"].is_string());
    assert!(manifest["servers"]["BrowserClaw"]["links"]["claude-code"]["createdAt"].is_string());

    let configured = service.list_browseros_connections().await?;
    assert_eq!(configured.len(), 7);
    assert_eq!(
        configured
            .iter()
            .filter(|state| state.installed)
            .map(|state| state.harness)
            .collect::<Vec<_>>(),
        [Harness::ClaudeCode, Harness::Codex, Harness::Zed]
    );
    assert_eq!(configured[0].message, "Configured in Claude Code.");

    fs::write(claude_path, "{\"mcpServers\":{}}")?;
    let scan = service.run_integrity_scan().await?;
    assert_eq!(scan.verified, 2);
    assert_eq!(scan.drifted, 1);
    assert_eq!(scan.missing, 0);
    assert_eq!(scan.healed, 1);
    assert_eq!(scan.failed, 0);
    let healed: Value = serde_json::from_str(&fs::read_to_string(claude_path)?)?;
    assert_eq!(healed["mcpServers"]["BrowserClaw"]["type"], "http");

    let disconnected = service.disconnect_browseros(Harness::Codex).await?;
    assert!(!disconnected.installed);
    assert_eq!(disconnected.message, "BrowserOS unregistered from Codex.");
    assert!(!fs::read_to_string(path_for(&paths, AgentId::Codex)?)?.contains("BrowserClaw"));
    let after_disconnect = service.list_browseros_connections().await?;
    let codex = after_disconnect
        .iter()
        .find(|state| state.harness == Harness::Codex)
        .ok_or_else(|| anyhow::anyhow!("missing Codex row"))?;
    assert!(!codex.installed);
    assert_eq!(codex.message, "Codex is not configured.");

    let router = test_router(&claw_dir, &home).await?;
    let (status, listed) = request_json(&router, "GET", "/connections").await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(listed["connections"].as_array().map(Vec::len), Some(7));
    assert_eq!(listed["connections"][0]["harness"], "Claude Code");
    assert_eq!(listed["connections"][0]["agentId"], "claude-code");

    let (status, connected) =
        request_json(&router, "POST", "/connections/VS%20Code/connect").await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(connected["harness"], "VS Code");
    assert_eq!(connected["agentId"], "vscode");
    assert_eq!(connected["installed"], true);
    let (status, disconnected) =
        request_json(&router, "POST", "/connections/VS%20Code/disconnect").await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(disconnected["installed"], false);

    let custom_workspace = claw_dir.join("custom-mcp-manager");
    let custom_config = home.join("custom/cursor.json");
    fs::create_dir_all(parent(&custom_config)?)?;
    let custom_manager = Manager::new(&custom_workspace);
    let mut custom_link = LinkInput::new(
        McpServer {
            name: "CustomPath".to_string(),
            spec: McpServerSpec::Http {
                url: MCP_URL.to_string(),
                headers: Default::default(),
            },
        },
        AgentId::Cursor,
    );
    custom_link.config_path = Some(custom_config.clone());
    custom_manager.link(custom_link)?;
    let custom_service = HarnessService::new(custom_workspace, home.clone());
    let scan = custom_service.run_integrity_scan().await?;
    assert_eq!(scan.verified, 1);
    assert_eq!(scan.healed, 0);
    assert!(!path_for(&paths, AgentId::Cursor)?.exists());
    Ok(())
}

async fn assert_legacy_manifest_migration(
    home: &Path,
    paths: &[(AgentId, std::path::PathBuf)],
) -> anyhow::Result<()> {
    let workspace = home.join("claw/legacy-mcp-manager");
    fs::create_dir_all(&workspace)?;
    let claude_path = path_for(paths, AgentId::ClaudeCode)?;
    fs::write(
        claude_path,
        format!(
            "{{\"mcpServers\":{{\"BrowserClaw\":{{\"type\":\"http\",\"url\":\"{MCP_URL}\"}}}}}}"
        ),
    )?;
    let added_at = "2026-01-02T03:04:05Z";
    fs::write(
        workspace.join("manifest.json"),
        serde_json::to_string_pretty(&json!({
            "version": 1,
            "servers": {
                "BrowserClaw": {
                    "spec": { "transport": "http", "url": MCP_URL },
                    "addedAt": added_at
                }
            },
            "links": [{
                "serverName": "BrowserClaw",
                "agent": "claude-code",
                "configPath": claude_path
            }]
        }))?,
    )?;

    let service = HarnessService::new(workspace.clone(), home.to_path_buf());
    let listed = service.list_browseros_connections().await?;
    let claude = listed
        .iter()
        .find(|state| state.harness == Harness::ClaudeCode)
        .ok_or_else(|| anyhow::anyhow!("missing migrated Claude Code row"))?;
    assert!(claude.installed);
    let connected = service
        .connect_browseros(Harness::ClaudeCode, MCP_URL)
        .await?;
    assert!(connected.installed, "{}", connected.message);

    let migrated = Manager::new(&workspace).list()?;
    assert_eq!(migrated.len(), 1);
    assert_eq!(migrated[0].name, "BrowserClaw");
    assert_eq!(migrated[0].added_at, added_at);
    assert_eq!(migrated[0].links[&AgentId::ClaudeCode].created_at, added_at);

    let corrupt_workspace = home.join("claw/corrupt-mcp-manager");
    fs::create_dir_all(&corrupt_workspace)?;
    let corrupt = "{ definitely not json";
    fs::write(corrupt_workspace.join("manifest.json"), corrupt)?;
    let corrupt_service = HarnessService::new(corrupt_workspace.clone(), home.to_path_buf());
    let error = corrupt_service
        .run_integrity_scan()
        .await
        .err()
        .ok_or_else(|| anyhow::anyhow!("corrupt manifest unexpectedly migrated"))?;
    assert!(error.to_string().contains("is not valid JSON"));
    assert_eq!(
        fs::read_to_string(corrupt_workspace.join("manifest.json"))?,
        corrupt
    );
    Ok(())
}

async fn test_router(claw_dir: &Path, home: &Path) -> anyhow::Result<Router> {
    let config = Arc::new(Config {
        server_port: 9200,
        cdp_port: 49337,
        proxy_port: None,
        resources_dir: claw_dir.join("resources"),
        browserclaw_dir: claw_dir.to_path_buf(),
        claw_dir: claw_dir.to_path_buf(),
        session_idle: Duration::from_secs(300),
        session_sweep_interval: Duration::from_secs(60),
        screencast_screenshot_fallback: true,
        dev_mode: false,
        auth_token: None,
    });
    let state = AppState::new_with_home(config, None, home.to_path_buf()).await?;
    Ok(build_router(state))
}

async fn request_json(
    router: &Router,
    method: &str,
    uri: &str,
) -> anyhow::Result<(StatusCode, Value)> {
    let request = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::HOST, "localhost")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::empty())?;
    let response = router.clone().oneshot(request).await?;
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await?;
    Ok((status, serde_json::from_slice(&bytes)?))
}

fn config_paths() -> anyhow::Result<Vec<(AgentId, std::path::PathBuf)>> {
    AgentId::ALL
        .into_iter()
        .map(|agent| {
            resolve_agent_mcp_config_path(agent, AgentScope::System)
                .map(|path| (agent, path))
                .map_err(anyhow::Error::from)
        })
        .collect()
}

fn path_for(paths: &[(AgentId, std::path::PathBuf)], agent: AgentId) -> anyhow::Result<&Path> {
    paths
        .iter()
        .find(|(candidate, _)| *candidate == agent)
        .map(|(_, path)| path.as_path())
        .ok_or_else(|| anyhow::anyhow!("missing config path for {agent}"))
}

fn parent(path: &Path) -> anyhow::Result<&Path> {
    path.parent()
        .ok_or_else(|| anyhow::anyhow!("config path has no parent: {}", path.display()))
}
