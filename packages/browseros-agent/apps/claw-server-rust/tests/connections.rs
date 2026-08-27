use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode, header},
};
use claw_server_rust::{
    AppState,
    analytics::{AnalyticsSink, events},
    build_router,
    config::Config,
    services::harness::{
        BROWSERCLAW_LEGACY_MCP_SERVER_NAME, BROWSEROS_MCP_SERVER_NAME,
        BROWSEROS_NEO_LEGACY_MCP_SERVER_NAME, Harness, HarnessService,
    },
};
use harness_integrations::{
    AgentId, AgentScope, InspectEntryInput, LinkInput, McpManager, McpServer, McpServerSpec,
    SkillSpec, resolve_agent_mcp_config_path,
};
use serde_json::{Value, json};
use std::{
    env, fs,
    path::Path,
    process::Command,
    sync::{Arc, Mutex},
    time::Duration,
};
use tower::ServiceExt;

const CHILD_CASE: &str = "CLAW_CONNECTIONS_TEST_CHILD";
const TEST_HOME: &str = "CLAW_CONNECTIONS_TEST_HOME";
const MCP_URL: &str = "http://127.0.0.1:9200/mcp";

#[derive(Default)]
struct RecordingAnalytics {
    events: Mutex<Vec<(events::EventDefinition, Value)>>,
}

impl AnalyticsSink for RecordingAnalytics {
    fn capture(&self, event: events::EventDefinition, properties: Value) {
        self.events
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push((event, properties));
    }
}

impl RecordingAnalytics {
    fn take(&self) -> Vec<(events::EventDefinition, Value)> {
        std::mem::take(
            &mut *self
                .events
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
        )
    }
}

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
    let browserclaw_dir = home.join("claw");
    let analytics = Arc::new(RecordingAnalytics::default());
    let service = HarnessService::new_with_managed_skill(
        browserclaw_dir.join("mcp-manager"),
        browserclaw_dir.join("harness-integrations"),
        home.clone(),
        SkillSpec::new("browseros-neo", "managed skill v1\n")?,
        analytics.clone(),
    );
    let paths = config_paths()?;

    for (agent, path) in &paths {
        if *agent != AgentId::Antigravity {
            fs::create_dir_all(parent(path)?)?;
        }
    }

    assert_identity_migration(&home, &paths, analytics.clone()).await?;
    assert_legacy_manifest_migration(&home, &paths).await?;

    let not_installed = service
        .connect_browseros(Harness::Antigravity, MCP_URL)
        .await?;
    assert!(!not_installed.installed);
    assert!(analytics.take().is_empty());
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
        r#"{"mcpServers":{"BrowserOS neo":{"command":"foreign"},"BrowserClaw":{"command":"unrelated"}}}"#,
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
    let claude_skill = home.join("skills/browseros-neo");
    assert_eq!(
        fs::read_to_string(claude_skill.join("SKILL.md"))?,
        "managed skill v1\n"
    );

    let codex = service.connect_browseros(Harness::Codex, MCP_URL).await?;
    let zed = service.connect_browseros(Harness::Zed, MCP_URL).await?;
    assert!(codex.installed && zed.installed);
    let shared_skill = home.join(".agents/skills/browseros-neo");
    assert_eq!(
        fs::read_to_string(shared_skill.join("SKILL.md"))?,
        "managed skill v1\n"
    );
    let shared_target_path = fs::canonicalize(parent(&shared_skill)?)?.join("browseros-neo");
    let skill_manifest_path = browserclaw_dir.join("harness-integrations/skills.json");
    let skill_manifest: Value = serde_json::from_str(&fs::read_to_string(&skill_manifest_path)?)?;
    let shared_record = skill_manifest["targets"]
        .as_array()
        .and_then(|targets| {
            targets
                .iter()
                .find(|target| target["targetPath"] == shared_target_path.display().to_string())
        })
        .ok_or_else(|| anyhow::anyhow!("missing shared skill target"))?;
    assert_eq!(shared_record["consumers"], json!(["codex", "zed"]));

    let claude_json: Value = serde_json::from_str(&fs::read_to_string(claude_path)?)?;
    assert_eq!(
        claude_json["mcpServers"][BROWSEROS_MCP_SERVER_NAME],
        json!({ "type": "http", "url": MCP_URL })
    );
    assert_eq!(
        claude_json["mcpServers"][BROWSERCLAW_LEGACY_MCP_SERVER_NAME],
        json!({ "command": "unrelated" })
    );
    assert_eq!(
        claude_json["mcpServers"][BROWSEROS_NEO_LEGACY_MCP_SERVER_NAME],
        json!({ "command": "foreign" })
    );

    let codex_raw = fs::read_to_string(path_for(&paths, AgentId::Codex)?)?;
    assert!(codex_raw.contains("[mcp_servers.browseros-neo]"));
    let codex_toml: toml::Value = toml::from_str(&codex_raw)?;
    assert_eq!(
        codex_toml["mcp_servers"][BROWSEROS_MCP_SERVER_NAME]["url"].as_str(),
        Some(MCP_URL)
    );

    let zed_json: Value =
        serde_json::from_str(&fs::read_to_string(path_for(&paths, AgentId::Zed)?)?)?;
    assert_eq!(
        zed_json["context_servers"][BROWSEROS_MCP_SERVER_NAME],
        json!({ "url": MCP_URL, "source": "custom", "enabled": true })
    );

    let manifest: Value = serde_json::from_str(&fs::read_to_string(
        browserclaw_dir.join("mcp-manager/manifest.json"),
    )?)?;
    assert_eq!(manifest["version"], 1);
    assert_eq!(
        manifest["servers"][BROWSEROS_MCP_SERVER_NAME]["links"]
            .as_object()
            .map(serde_json::Map::len),
        Some(3)
    );
    assert!(manifest["servers"][BROWSEROS_MCP_SERVER_NAME]["addedAt"].is_string());
    assert!(
        manifest["servers"][BROWSEROS_MCP_SERVER_NAME]["links"]["claude-code"]["createdAt"]
            .is_string()
    );

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
    let manifest_before_list = fs::read(&skill_manifest_path)?;
    let manifest_mtime_before_list = fs::metadata(&skill_manifest_path)?.modified()?;
    let skill_mtime_before_list = fs::metadata(shared_skill.join("SKILL.md"))?.modified()?;
    service.list_browseros_connections().await?;
    assert_eq!(fs::read(&skill_manifest_path)?, manifest_before_list);
    assert_eq!(
        fs::metadata(&skill_manifest_path)?.modified()?,
        manifest_mtime_before_list
    );
    assert_eq!(
        fs::metadata(shared_skill.join("SKILL.md"))?.modified()?,
        skill_mtime_before_list
    );

    fs::write(shared_skill.join("SKILL.md"), "edited")?;
    let reconnected = service.connect_browseros(Harness::Codex, MCP_URL).await?;
    assert!(reconnected.installed);
    assert_eq!(
        fs::read_to_string(shared_skill.join("SKILL.md"))?,
        "managed skill v1\n"
    );

    fs::remove_dir_all(&shared_skill)?;
    let boot_repair = service.run_skill_reconciliation().await?;
    assert_eq!(boot_repair.installed, 1);
    assert_eq!(
        fs::read_to_string(shared_skill.join("SKILL.md"))?,
        "managed skill v1\n"
    );

    let ota_service = HarnessService::new_with_managed_skill(
        browserclaw_dir.join("mcp-manager"),
        browserclaw_dir.join("harness-integrations"),
        home.clone(),
        SkillSpec::new("browseros-neo", "managed skill v2\n")?,
        analytics.clone(),
    );
    let ota_update = ota_service.run_skill_reconciliation().await?;
    assert_eq!(ota_update.updated, 2);
    assert_eq!(
        fs::read_to_string(shared_skill.join("SKILL.md"))?,
        "managed skill v2\n"
    );
    let restored_skill = service.run_skill_reconciliation().await?;
    assert_eq!(restored_skill.updated, 2);

    const NEW_MCP_URL: &str = "http://127.0.0.1:9999/mcp";
    let migrated = service.migrate_connected_urls(NEW_MCP_URL).await?;
    assert_eq!(migrated.migrated, 3);
    assert_eq!(migrated.failed, 0);

    let claude_json: Value = serde_json::from_str(&fs::read_to_string(claude_path)?)?;
    assert_eq!(
        claude_json["mcpServers"][BROWSEROS_MCP_SERVER_NAME]["url"], NEW_MCP_URL,
        "claude config re-pointed to the new URL"
    );
    let codex_toml: toml::Value =
        toml::from_str(&fs::read_to_string(path_for(&paths, AgentId::Codex)?)?)?;
    assert_eq!(
        codex_toml["mcp_servers"][BROWSEROS_MCP_SERVER_NAME]["url"].as_str(),
        Some(NEW_MCP_URL)
    );
    let zed_json: Value =
        serde_json::from_str(&fs::read_to_string(path_for(&paths, AgentId::Zed)?)?)?;
    assert_eq!(
        zed_json["context_servers"][BROWSEROS_MCP_SERVER_NAME]["url"],
        NEW_MCP_URL
    );

    // A crash can update the manifest before every agent config reaches the new URL.
    const STALE_MCP_URL: &str = "http://127.0.0.1:8888/mcp";
    fs::write(
        claude_path,
        format!(
            r#"{{"mcpServers":{{"{BROWSEROS_MCP_SERVER_NAME}":{{"type":"http","url":"{STALE_MCP_URL}"}}}}}}"#
        ),
    )?;
    let repaired = service.migrate_connected_urls(NEW_MCP_URL).await?;
    assert_eq!(repaired.migrated, 3);
    assert_eq!(repaired.failed, 0);
    let claude_json: Value = serde_json::from_str(&fs::read_to_string(claude_path)?)?;
    assert_eq!(
        claude_json["mcpServers"][BROWSEROS_MCP_SERVER_NAME]["url"], NEW_MCP_URL,
        "a straggler left on a stale port is repaired, not skipped"
    );

    let restored = service.migrate_connected_urls(MCP_URL).await?;
    assert_eq!(restored.migrated, 3);
    assert_eq!(configured[0].message, "Configured in Claude Code.");

    fs::write(claude_path, "{\"mcpServers\":{}}")?;
    let scan = service.run_integrity_scan().await?;
    assert_eq!(scan.verified, 2);
    assert_eq!(scan.drifted, 1);
    assert_eq!(scan.missing, 0);
    assert_eq!(scan.healed, 1);
    assert_eq!(scan.failed, 0);
    let healed: Value = serde_json::from_str(&fs::read_to_string(claude_path)?)?;
    assert_eq!(
        healed["mcpServers"][BROWSEROS_MCP_SERVER_NAME]["type"],
        "http"
    );

    let disconnected = service.disconnect_browseros(Harness::Codex).await?;
    assert!(!disconnected.installed);
    assert_eq!(disconnected.message, "BrowserOS unregistered from Codex.");
    assert!(
        !fs::read_to_string(path_for(&paths, AgentId::Codex)?)?.contains(BROWSEROS_MCP_SERVER_NAME)
    );
    assert!(shared_skill.exists());
    let skill_manifest: Value = serde_json::from_str(&fs::read_to_string(&skill_manifest_path)?)?;
    let shared_record = skill_manifest["targets"]
        .as_array()
        .and_then(|targets| {
            targets
                .iter()
                .find(|target| target["targetPath"] == shared_target_path.display().to_string())
        })
        .ok_or_else(|| anyhow::anyhow!("missing shared skill target after Codex disconnect"))?;
    assert_eq!(shared_record["consumers"], json!(["zed"]));
    let after_disconnect = service.list_browseros_connections().await?;
    let codex = after_disconnect
        .iter()
        .find(|state| state.harness == Harness::Codex)
        .ok_or_else(|| anyhow::anyhow!("missing Codex row"))?;
    assert!(!codex.installed);
    assert_eq!(codex.message, "Codex is not configured.");

    let antigravity_skill = home.join(".gemini/config/skills/browseros-neo");
    fs::create_dir_all(&antigravity_skill)?;
    fs::write(antigravity_skill.join("SKILL.md"), "foreign skill")?;
    fs::write(antigravity_skill.join("keep.txt"), "keep")?;
    let antigravity = service
        .connect_browseros(Harness::Antigravity, MCP_URL)
        .await?;
    assert!(antigravity.installed);
    assert!(
        antigravity
            .message
            .contains("skill reconciliation needs a retry")
    );
    assert_eq!(
        fs::read_to_string(antigravity_skill.join("SKILL.md"))?,
        "foreign skill"
    );
    assert_eq!(
        fs::read_to_string(antigravity_skill.join("keep.txt"))?,
        "keep"
    );
    let listed = service.list_browseros_connections().await?;
    assert!(
        listed
            .iter()
            .any(|state| state.harness == Harness::Antigravity && state.installed)
    );

    fs::remove_dir_all(&antigravity_skill)?;
    let antigravity = service
        .connect_browseros(Harness::Antigravity, MCP_URL)
        .await?;
    assert!(antigravity.installed);
    assert_eq!(
        antigravity.message,
        "BrowserOS registered as an MCP server in Antigravity."
    );
    assert_eq!(
        fs::read_to_string(antigravity_skill.join("SKILL.md"))?,
        "managed skill v1\n"
    );

    let valid_skill_manifest = fs::read(&skill_manifest_path)?;
    fs::write(&skill_manifest_path, "{ broken")?;
    let antigravity = service.disconnect_browseros(Harness::Antigravity).await?;
    assert!(!antigravity.installed);
    assert!(
        antigravity
            .message
            .contains("skill reconciliation needs a retry")
    );
    assert!(antigravity_skill.exists());
    let listed = service.list_browseros_connections().await?;
    assert!(
        listed
            .iter()
            .all(|state| state.harness != Harness::Antigravity || !state.installed)
    );
    fs::write(&skill_manifest_path, valid_skill_manifest)?;
    let cleanup_retry = service.run_skill_reconciliation().await?;
    assert_eq!(cleanup_retry.removed, 1);
    assert!(!antigravity_skill.exists());

    let router = test_router(&browserclaw_dir, &home).await?;
    let (status, listed) = request_json(&router, "GET", "/api/v1/connections").await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(listed["items"].as_array().map(Vec::len), Some(7));
    assert_eq!(listed["items"][0]["harness"], "Claude Code");

    let (status, connected) = request_json(&router, "PUT", "/api/v1/connections/VS%20Code").await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(connected["harness"], "VS Code");
    assert_eq!(connected["installed"], true);
    let (status, disconnected) =
        request_json(&router, "DELETE", "/api/v1/connections/VS%20Code").await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(disconnected["installed"], false);

    let custom_workspace = browserclaw_dir.join("custom-mcp-manager");
    let custom_config = home.join("custom/cursor.json");
    fs::create_dir_all(parent(&custom_config)?)?;
    let custom_manager = McpManager::new(&custom_workspace);
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

    for harness in Harness::ALL {
        service.disconnect_browseros(harness).await?;
    }
    analytics.take();
    for harness in Harness::ALL {
        let state = service.connect_browseros(harness, MCP_URL).await?;
        assert!(state.installed, "{}", state.message);
        let repeated = service.connect_browseros(harness, MCP_URL).await?;
        assert!(repeated.installed, "{}", repeated.message);
    }
    let mut claude_with_foreign_legacy: Value =
        serde_json::from_str(&fs::read_to_string(claude_path)?)?;
    claude_with_foreign_legacy["mcpServers"]
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("missing Claude MCP object"))?
        .insert(
            BROWSERCLAW_LEGACY_MCP_SERVER_NAME.to_string(),
            json!({ "command": "unrelated" }),
        );
    fs::write(
        claude_path,
        serde_json::to_string_pretty(&claude_with_foreign_legacy)?,
    )?;
    for harness in Harness::ALL {
        let state = service.disconnect_browseros(harness).await?;
        assert!(!state.installed, "{}", state.message);
        let repeated = service.disconnect_browseros(harness).await?;
        assert!(!repeated.installed, "{}", repeated.message);
    }
    let claude_after_disconnect = fs::read_to_string(claude_path)?;
    assert!(claude_after_disconnect.contains(BROWSERCLAW_LEGACY_MCP_SERVER_NAME));
    assert!(!claude_after_disconnect.contains(BROWSEROS_MCP_SERVER_NAME));
    let captured = analytics.take();
    assert_eq!(captured.len(), Harness::ALL.len() * 2);
    for (index, harness) in Harness::ALL.into_iter().enumerate() {
        assert_eq!(
            captured[index],
            (
                events::HARNESS_CONNECTED,
                json!({ "harness": harness.as_str() }),
            )
        );
        assert_eq!(
            captured[index + Harness::ALL.len()],
            (
                events::HARNESS_DISCONNECTED,
                json!({ "harness": harness.as_str() }),
            )
        );
    }
    Ok(())
}

async fn assert_identity_migration(
    home: &Path,
    paths: &[(AgentId, std::path::PathBuf)],
    analytics: Arc<RecordingAnalytics>,
) -> anyhow::Result<()> {
    const STALE_URL: &str = "http://127.0.0.1:7777/mcp";
    let workspace = home.join("claw/identity-mcp-manager");
    let manager = McpManager::new(&workspace);
    let seed_manager = McpManager::new(home.join("claw/identity-seed-manager"));
    let custom_claude = home.join("custom/claude.json");
    fs::create_dir_all(parent(&custom_claude)?)?;
    let source_spec = McpServerSpec::Http {
        url: STALE_URL.to_string(),
        headers: Default::default(),
    };
    let antigravity_parent = parent(path_for(paths, AgentId::Antigravity)?)?;
    let antigravity_parent_existed = antigravity_parent.exists();
    let mut migration_paths = Vec::new();
    for agent in AgentId::ALL {
        let config_path = if agent == AgentId::ClaudeCode {
            custom_claude.clone()
        } else {
            path_for(paths, agent)?.to_path_buf()
        };
        fs::create_dir_all(parent(&config_path)?)?;
        let source_name = if matches!(
            agent,
            AgentId::ClaudeCode | AgentId::Codex | AgentId::Cursor | AgentId::OpenCode
        ) {
            BROWSEROS_NEO_LEGACY_MCP_SERVER_NAME
        } else {
            BROWSERCLAW_LEGACY_MCP_SERVER_NAME
        };
        let mut input = LinkInput::new(
            McpServer {
                name: source_name.to_string(),
                spec: source_spec.clone(),
            },
            agent,
        );
        input.config_path = Some(config_path.clone());
        if agent == AgentId::ClaudeCode {
            manager.link(input)?;
        } else {
            seed_manager.link(input)?;
        }
        migration_paths.push((agent, config_path));
    }
    let source = manager.list()?.remove(0);
    let source_added_at = source.added_at;
    let source_created_at = source.links[&AgentId::ClaudeCode].created_at.clone();

    let vscode_path = path_for(&migration_paths, AgentId::VsCode)?;
    let mut collision = LinkInput::new(
        McpServer {
            name: BROWSEROS_MCP_SERVER_NAME.to_string(),
            spec: McpServerSpec::Http {
                url: "http://127.0.0.1:7000/mcp".to_string(),
                headers: Default::default(),
            },
        },
        AgentId::VsCode,
    );
    collision.config_path = Some(vscode_path.to_path_buf());
    seed_manager.link(collision)?;

    let service = HarnessService::new_with_analytics(
        workspace.clone(),
        home.to_path_buf(),
        analytics.clone(),
    );
    let outcome = service.migrate_browseros_identity(MCP_URL).await?;
    assert_eq!(outcome.migrated, 7);
    assert_eq!(outcome.failed, 0);
    assert_eq!(outcome.skipped, 0);
    let reconnected = service
        .connect_browseros(Harness::ClaudeCode, MCP_URL)
        .await?;
    assert_eq!(
        reconnected.config_path.as_deref(),
        Some("~/custom/claude.json")
    );
    assert!(!path_for(paths, AgentId::ClaudeCode)?.exists());
    assert!(analytics.take().is_empty());
    for (agent, config_path) in &migration_paths {
        for source_name in [
            BROWSEROS_NEO_LEGACY_MCP_SERVER_NAME,
            BROWSERCLAW_LEGACY_MCP_SERVER_NAME,
        ] {
            assert!(
                manager
                    .inspect_entry(
                        InspectEntryInput::new(source_name, *agent).at_path(config_path)
                    )?
                    .is_none(),
                "{agent}: {source_name}"
            );
        }
        assert_eq!(
            manager
                .inspect_entry(
                    InspectEntryInput::new(BROWSEROS_MCP_SERVER_NAME, *agent).at_path(config_path),
                )?
                .map(|entry| entry.spec),
            Some(McpServerSpec::Http {
                url: MCP_URL.to_string(),
                headers: Default::default(),
            }),
            "{agent}"
        );
    }
    let canonical = manager.list()?.remove(0);
    assert_eq!(canonical.name, BROWSEROS_MCP_SERVER_NAME);
    assert_eq!(canonical.links.len(), 7);
    assert_eq!(canonical.added_at, source_added_at);
    assert_eq!(
        canonical.links[&AgentId::ClaudeCode].created_at,
        source_created_at
    );
    assert_eq!(
        canonical.links[&AgentId::ClaudeCode].config_path,
        custom_claude
    );
    let repeated = service.migrate_browseros_identity(MCP_URL).await?;
    assert_eq!(repeated.migrated, 0);
    assert_eq!(repeated.failed, 0);
    assert_eq!(repeated.skipped, 7);
    assert!(analytics.take().is_empty());

    for (_, config_path) in &migration_paths {
        fs::remove_file(config_path)?;
    }
    if !antigravity_parent_existed {
        fs::remove_dir_all(home.join(".gemini"))?;
    }

    let canonical_workspace = home.join("claw/identity-canonical-manager");
    let canonical_manager = McpManager::new(&canonical_workspace);
    let canonical_service = HarnessService::new(canonical_workspace.clone(), home.to_path_buf());
    let cursor_path = path_for(paths, AgentId::Cursor)?;
    let canonical_raw = format!(
        r#"{{"mcpServers":{{"{BROWSEROS_MCP_SERVER_NAME}":{{"type":"http","url":"{STALE_URL}"}}}},"keep":true}}"#
    );
    fs::write(cursor_path, canonical_raw)?;
    let canonical = canonical_service
        .migrate_browseros_identity(MCP_URL)
        .await?;
    assert_eq!(canonical.migrated, 1);
    assert_eq!(canonical.failed, 0);
    assert_eq!(canonical.skipped, 6);
    let canonical_entry = canonical_manager
        .inspect_entry(
            InspectEntryInput::new(BROWSEROS_MCP_SERVER_NAME, AgentId::Cursor).at_path(cursor_path),
        )?
        .ok_or_else(|| anyhow::anyhow!("missing adopted canonical entry"))?;
    assert_eq!(
        canonical_entry.spec,
        McpServerSpec::Http {
            url: MCP_URL.to_string(),
            headers: Default::default(),
        }
    );
    assert!(fs::read_to_string(cursor_path)?.contains(r#""keep":true"#));
    assert_eq!(canonical_manager.list()?.remove(0).links.len(), 1);
    fs::remove_file(cursor_path)?;

    let foreign_workspace = home.join("claw/identity-foreign-manager");
    let foreign_service = HarnessService::new(foreign_workspace.clone(), home.to_path_buf());
    let foreign_raw = r#"{"mcpServers":{"BrowserClaw":{"command":"foreign"},"BrowserOS neo":{"command":"standalone"}},"keep":true}"#;
    fs::write(cursor_path, foreign_raw)?;
    let foreign = foreign_service.migrate_browseros_identity(MCP_URL).await?;
    assert_eq!(foreign.migrated, 0);
    assert_eq!(foreign.failed, 0);
    assert_eq!(fs::read_to_string(cursor_path)?, foreign_raw);
    assert!(!foreign_workspace.join("manifest.json").exists());
    fs::remove_file(cursor_path)?;

    let collision_workspace = home.join("claw/identity-collision-manager");
    let collision_service = HarnessService::new(collision_workspace.clone(), home.to_path_buf());
    let collision_raw = format!(
        r#"{{"mcpServers":{{"{BROWSEROS_NEO_LEGACY_MCP_SERVER_NAME}":{{"type":"http","url":"{MCP_URL}"}},"{BROWSEROS_MCP_SERVER_NAME}":{{"command":"foreign"}}}},"keep":true}}"#
    );
    fs::write(cursor_path, &collision_raw)?;
    let collision = collision_service
        .migrate_browseros_identity(MCP_URL)
        .await?;
    assert_eq!(collision.migrated, 0);
    assert_eq!(collision.failed, 1);
    assert_eq!(collision.skipped, 6);
    assert_eq!(fs::read_to_string(cursor_path)?, collision_raw);
    assert!(!collision_workspace.join("manifest.json").exists());

    let collision_disconnect = collision_service
        .disconnect_browseros(Harness::Cursor)
        .await?;
    assert!(!collision_disconnect.installed);
    assert!(
        collision_disconnect
            .message
            .contains(BROWSEROS_MCP_SERVER_NAME)
    );
    assert_eq!(fs::read_to_string(cursor_path)?, collision_raw);
    assert!(!collision_workspace.join("manifest.json").exists());

    let collision_connect = collision_service
        .connect_browseros(Harness::Cursor, MCP_URL)
        .await?;
    assert!(!collision_connect.installed);
    assert!(
        collision_connect
            .message
            .contains(BROWSEROS_MCP_SERVER_NAME)
    );
    assert_eq!(fs::read_to_string(cursor_path)?, collision_raw);
    assert!(!collision_workspace.join("manifest.json").exists());
    fs::remove_file(cursor_path)?;

    let failure_workspace = home.join("claw/identity-failure-manager");
    let failure_seed = McpManager::new(home.join("claw/identity-failure-seed"));
    let claude_path = path_for(paths, AgentId::ClaudeCode)?;
    let mut legacy = LinkInput::new(
        McpServer {
            name: BROWSERCLAW_LEGACY_MCP_SERVER_NAME.to_string(),
            spec: source_spec,
        },
        AgentId::ClaudeCode,
    );
    legacy.config_path = Some(claude_path.to_path_buf());
    failure_seed.link(legacy)?;
    let malformed = "{ definitely not json";
    fs::write(cursor_path, malformed)?;
    let failure_service = HarnessService::new(failure_workspace, home.to_path_buf());
    let partial = failure_service.migrate_browseros_identity(MCP_URL).await?;
    assert_eq!(partial.migrated, 1);
    assert_eq!(partial.failed, 1);
    assert_eq!(fs::read_to_string(cursor_path)?, malformed);
    fs::remove_file(claude_path)?;
    fs::remove_file(cursor_path)?;

    let compatibility_workspace = home.join("claw/identity-compatibility-manager");
    let compatibility_manager = McpManager::new(&compatibility_workspace);
    let mut legacy = LinkInput::new(
        McpServer {
            name: BROWSERCLAW_LEGACY_MCP_SERVER_NAME.to_string(),
            spec: McpServerSpec::Http {
                url: STALE_URL.to_string(),
                headers: Default::default(),
            },
        },
        AgentId::Cursor,
    );
    legacy.config_path = Some(cursor_path.to_path_buf());
    compatibility_manager.link(legacy)?;
    let compatibility_analytics = Arc::new(RecordingAnalytics::default());
    let compatibility = HarnessService::new_with_analytics(
        compatibility_workspace,
        home.to_path_buf(),
        compatibility_analytics.clone(),
    );
    let listed = compatibility.list_browseros_connections().await?;
    assert!(
        listed
            .iter()
            .any(|state| state.harness == Harness::Cursor && state.installed)
    );
    let url_outcome = compatibility.migrate_connected_urls(MCP_URL).await?;
    assert_eq!(url_outcome.migrated, 1);
    let updated = compatibility_manager
        .inspect_entry(
            InspectEntryInput::new(BROWSERCLAW_LEGACY_MCP_SERVER_NAME, AgentId::Cursor)
                .at_path(cursor_path),
        )?
        .ok_or_else(|| anyhow::anyhow!("missing compatible legacy entry"))?;
    assert_eq!(
        updated.spec,
        McpServerSpec::Http {
            url: MCP_URL.to_string(),
            headers: Default::default(),
        }
    );
    let disconnected = compatibility.disconnect_browseros(Harness::Cursor).await?;
    assert!(!disconnected.installed);
    assert!(
        compatibility_manager
            .list_links(Default::default())?
            .is_empty()
    );
    assert_eq!(compatibility_analytics.take().len(), 1);
    fs::remove_file(cursor_path)?;
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

    let migrated = McpManager::new(&workspace).list()?;
    assert_eq!(migrated.len(), 1);
    assert_eq!(migrated[0].name, BROWSEROS_MCP_SERVER_NAME);
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

async fn test_router(browserclaw_dir: &Path, home: &Path) -> anyhow::Result<Router> {
    let config = Arc::new(Config {
        server_port: 9200,
        cdp_port: 49337,
        proxy_port: None,
        resources_dir: browserclaw_dir.join("resources"),
        browserclaw_dir: browserclaw_dir.to_path_buf(),
        session_idle: Duration::from_secs(300),
        session_retention: Duration::from_secs(7_200),
        session_sweep_interval: Duration::from_secs(60),
        replay_retention_days: 7,
        dev_mode: false,
        auth_token: None,
    });
    let state = AppState::new_with_home(config, home.to_path_buf()).await?;
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
