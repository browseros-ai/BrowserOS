use std::{
    collections::BTreeMap,
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};

use harness_integrations::{
    AgentId, AgentScope, DisconnectInput, Error, InspectEntryInput, LinkInput, ListLinksFilter,
    McpManager, McpServer, McpServerSpec, MigrateServerInput, UnlinkInput, detect_installed_agents,
    is_agent_supported, is_installed, list_supported_agents, resolve_agent_surface,
    resolve_harness_definition,
};
use serde_json::Value;
use tempfile::tempdir;

fn stdio_server(name: &str) -> McpServer {
    McpServer {
        name: name.to_string(),
        spec: McpServerSpec::Stdio {
            command: format!("{name}-mcp"),
            args: Vec::new(),
            env: BTreeMap::new(),
        },
    }
}

fn link_input(server: McpServer, agent: AgentId, config_path: &Path) -> LinkInput {
    let mut input = LinkInput::new(server, agent);
    input.config_path = Some(config_path.to_path_buf());
    input
}

#[test]
fn link_writes_config_and_byte_compatible_manifest() -> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let manager = McpManager::new(root.path().join("workspace"));
    let config = root.path().join("cursor.json");
    let summary = manager.link(link_input(
        McpServer {
            name: "  gh  ".to_string(),
            spec: McpServerSpec::Stdio {
                command: "gh-mcp".to_string(),
                args: vec!["serve".to_string()],
                env: BTreeMap::new(),
            },
        },
        AgentId::Cursor,
        &config,
    ))?;
    assert_eq!(summary.server_name, "gh");
    assert!(summary.created);

    let config_json: Value = serde_json::from_str(&fs::read_to_string(&config)?)?;
    assert_eq!(config_json["mcpServers"]["gh"]["command"], "gh-mcp");
    let manifest_raw = fs::read_to_string(root.path().join("workspace/manifest.json"))?;
    assert!(manifest_raw.ends_with('\n'));
    assert!(manifest_raw.contains("\"addedAt\""));
    assert!(manifest_raw.contains("\"configPath\""));
    assert!(manifest_raw.contains("\"createdAt\""));
    assert!(!manifest_raw.contains("added_at"));
    Ok(())
}

#[test]
fn relink_is_idempotent_and_spec_is_last_write_wins() -> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let manager = McpManager::new(root.path().join("workspace"));
    let cursor = root.path().join("cursor.json");
    let zed = root.path().join("zed.json");
    manager.link(link_input(stdio_server("gh"), AgentId::Cursor, &cursor))?;
    let first_manifest: Value = serde_json::from_str(&fs::read_to_string(
        root.path().join("workspace/manifest.json"),
    )?)?;
    let summary = manager.link(link_input(stdio_server("gh"), AgentId::Cursor, &cursor))?;
    assert!(!summary.created);
    let second_manifest: Value = serde_json::from_str(&fs::read_to_string(
        root.path().join("workspace/manifest.json"),
    )?)?;
    assert_eq!(
        first_manifest["servers"]["gh"]["addedAt"],
        second_manifest["servers"]["gh"]["addedAt"]
    );
    assert_eq!(
        first_manifest["servers"]["gh"]["links"]["cursor"]["createdAt"],
        second_manifest["servers"]["gh"]["links"]["cursor"]["createdAt"]
    );

    manager.link(link_input(
        McpServer {
            name: "gh".to_string(),
            spec: McpServerSpec::Http {
                url: "https://example.com/mcp".to_string(),
                headers: BTreeMap::new(),
            },
        },
        AgentId::Zed,
        &zed,
    ))?;
    let listed = manager.list()?;
    assert!(matches!(listed[0].spec, McpServerSpec::Http { .. }));
    assert_eq!(listed[0].links.len(), 2);
    Ok(())
}

#[test]
fn foreign_entries_require_explicit_overwrite() -> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let manager = McpManager::new(root.path().join("workspace"));
    let config = root.path().join("cursor.json");
    fs::write(&config, r#"{"mcpServers":{"gh":{"command":"foreign"}}}"#)?;
    let error = manager
        .link(link_input(stdio_server("gh"), AgentId::Cursor, &config))
        .err()
        .ok_or("expected a foreign-entry error")?;
    assert!(matches!(
        error,
        Error::ForeignEntry {
            agent: AgentId::Cursor,
            ..
        }
    ));
    let mut overwrite = link_input(stdio_server("gh"), AgentId::Cursor, &config);
    overwrite.allow_overwrite = true;
    let summary = manager.link(overwrite)?;
    assert!(summary.overwrote_foreign);
    Ok(())
}

#[test]
fn unlink_uses_recorded_path_and_unknown_links_are_no_ops() -> Result<(), Box<dyn std::error::Error>>
{
    let root = tempdir()?;
    let workspace = root.path().join("workspace");
    let manager = McpManager::new(&workspace);
    let unknown = manager.unlink(UnlinkInput::new("ghost", AgentId::Cursor))?;
    assert!(!unknown.removed);
    assert!(!workspace.join("manifest.json").exists());

    let config = root.path().join("custom/cursor.json");
    fs::create_dir_all(config.parent().ok_or("missing test parent")?)?;
    manager.link(link_input(stdio_server("gh"), AgentId::Cursor, &config))?;
    let summary = manager.unlink(UnlinkInput::new("gh", AgentId::Cursor))?;
    assert!(summary.removed);
    let value: Value = serde_json::from_str(&fs::read_to_string(config)?)?;
    assert!(value["mcpServers"]["gh"].is_null());
    Ok(())
}

#[test]
fn disconnect_never_touches_other_agent_files() -> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let manager = McpManager::new(root.path().join("workspace"));
    let cursor = root.path().join("cursor.json");
    let vscode = root.path().join("vscode.json");
    let zed = root.path().join("zed.json");
    for (agent, path) in [
        (AgentId::Cursor, &cursor),
        (AgentId::VsCode, &vscode),
        (AgentId::Zed, &zed),
    ] {
        manager.link(link_input(stdio_server("gh"), agent, path))?;
    }
    let vscode_before = fs::read(&vscode)?;
    let zed_before = fs::read(&zed)?;
    let summary = manager.disconnect(DisconnectInput::new("gh", AgentId::Cursor))?;
    assert!(summary.unlinked);
    assert!(!summary.removed_manifest);
    assert_eq!(fs::read(vscode)?, vscode_before);
    assert_eq!(fs::read(zed)?, zed_before);
    assert_eq!(manager.list_links(ListLinksFilter::default())?.len(), 2);
    Ok(())
}

#[test]
fn disconnect_can_keep_an_empty_manifest_entry() -> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let manager = McpManager::new(root.path().join("workspace"));
    let config = root.path().join("cursor.json");
    manager.link(link_input(stdio_server("solo"), AgentId::Cursor, &config))?;
    let mut input = DisconnectInput::new("solo", AgentId::Cursor);
    input.remove_if_last = false;
    let summary = manager.disconnect(input)?;
    assert!(!summary.removed_manifest);
    assert!(manager.list()?[0].links.is_empty());
    Ok(())
}

#[test]
fn list_links_filters_by_server_and_agent() -> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let manager = McpManager::new(root.path().join("workspace"));
    let cursor = root.path().join("cursor.json");
    let zed = root.path().join("zed.json");
    manager.link(link_input(stdio_server("one"), AgentId::Cursor, &cursor))?;
    manager.link(link_input(stdio_server("two"), AgentId::Zed, &zed))?;
    let links = manager.list_links(ListLinksFilter {
        server_names: None,
        agents: Some(vec![AgentId::Zed]),
    })?;
    assert_eq!(links.len(), 1);
    assert_eq!(links[0].server_name, "two");
    Ok(())
}

#[test]
fn rescan_reads_each_manifest_recorded_config_path() -> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let manager = McpManager::new(root.path().join("workspace"));
    let first = root.path().join("custom/first.json");
    let second = root.path().join("custom/second.json");
    fs::create_dir_all(first.parent().ok_or("missing custom config parent")?)?;
    manager.link(link_input(stdio_server("one"), AgentId::Cursor, &first))?;
    manager.link(link_input(stdio_server("two"), AgentId::Cursor, &second))?;

    let report = manager.rescan()?;
    assert_eq!(report.verified.len(), 2);
    assert_eq!(
        report
            .verified
            .iter()
            .map(|link| link.config_path.as_path())
            .collect::<Vec<_>>(),
        [first.as_path(), second.as_path()]
    );
    assert!(report.drifted.is_empty());
    assert!(report.missing.is_empty());
    Ok(())
}

#[test]
fn inspect_and_migrate_generated_entries_across_every_harness_shape()
-> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let manager = McpManager::new(root.path().join("workspace"));
    let legacy_spec = McpServerSpec::Http {
        url: "http://127.0.0.1:9100/mcp".to_string(),
        headers: BTreeMap::new(),
    };
    let canonical_spec = McpServerSpec::Http {
        url: "http://127.0.0.1:9200/mcp".to_string(),
        headers: BTreeMap::new(),
    };

    for agent in AgentId::ALL {
        let config = root.path().join(format!("configs/{agent}"));
        fs::create_dir_all(config.parent().ok_or("missing config parent")?)?;
        manager.link(link_input(
            McpServer {
                name: "BrowserClaw".to_string(),
                spec: legacy_spec.clone(),
            },
            agent,
            &config,
        ))?;
        let inspected = manager
            .inspect_entry(InspectEntryInput::new("BrowserClaw", agent).at_path(&config))?
            .ok_or("missing generated entry")?;
        assert_eq!(inspected.spec, legacy_spec);

        let mut input = MigrateServerInput::new(
            "BrowserClaw",
            McpServer {
                name: "BrowserOS neo".to_string(),
                spec: canonical_spec.clone(),
            },
            agent,
        );
        input.config_path = Some(config.clone());
        let summary = manager.migrate_server(input)?;
        assert!(summary.migrated, "{agent}");
        assert!(
            manager
                .inspect_entry(InspectEntryInput::new("BrowserClaw", agent).at_path(&config))?
                .is_none(),
            "{agent}"
        );
        assert_eq!(
            manager
                .inspect_entry(InspectEntryInput::new("BrowserOS neo", agent).at_path(&config))?
                .map(|entry| entry.spec),
            Some(canonical_spec.clone()),
            "{agent}"
        );
    }

    Ok(())
}

#[test]
fn migration_scopes_foreign_authority_and_preserves_timestamps_and_formatting()
-> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let workspace = root.path().join("workspace");
    let manager = McpManager::new(&workspace);
    let config = root.path().join("custom/cursor.json");
    fs::create_dir_all(config.parent().ok_or("missing config parent")?)?;
    fs::write(
        &config,
        "{\n  // keep this\n  \"theme\": \"dark\",\n  \"mcpServers\": {}\n}\n",
    )?;
    let legacy_spec = McpServerSpec::Http {
        url: "http://127.0.0.1:9100/mcp".to_string(),
        headers: BTreeMap::new(),
    };
    manager.link(link_input(
        McpServer {
            name: "BrowserClaw".to_string(),
            spec: legacy_spec.clone(),
        },
        AgentId::Cursor,
        &config,
    ))?;
    let legacy = manager.list()?.remove(0);
    let legacy_added_at = legacy.added_at;
    let legacy_created_at = legacy.links[&AgentId::Cursor].created_at.clone();
    let with_collision = fs::read_to_string(&config)?.replace(
        "\"BrowserClaw\":",
        "\"BrowserOS neo\": { \"command\": \"foreign\" },\n    \"BrowserClaw\":",
    );
    fs::write(&config, with_collision)?;

    let mut migration = MigrateServerInput::new(
        "BrowserClaw",
        McpServer {
            name: "BrowserOS neo".to_string(),
            spec: McpServerSpec::Http {
                url: "http://127.0.0.1:9200/mcp".to_string(),
                headers: BTreeMap::new(),
            },
        },
        AgentId::Cursor,
    );
    migration.config_path = Some(config.clone());
    let summary = manager.migrate_server(migration.clone())?;
    assert!(summary.migrated);
    assert!(summary.overwrote_foreign);
    let raw = fs::read_to_string(&config)?;
    assert!(raw.contains("// keep this"));
    assert!(raw.contains("\"theme\": \"dark\""));
    assert!(!raw.contains("BrowserClaw"));
    let canonical = manager.list()?.remove(0);
    assert_eq!(canonical.name, "BrowserOS neo");
    assert_eq!(canonical.added_at, legacy_added_at);
    assert_eq!(
        canonical.links[&AgentId::Cursor].created_at,
        legacy_created_at
    );
    assert!(!manager.migrate_server(migration)?.migrated);

    let foreign_workspace = root.path().join("foreign-workspace");
    let foreign_manager = McpManager::new(&foreign_workspace);
    let foreign_config = root.path().join("foreign/cursor.json");
    fs::create_dir_all(foreign_config.parent().ok_or("missing foreign parent")?)?;
    let foreign_raw = r#"{"mcpServers":{"BrowserClaw":{"command":"foreign"},"BrowserOS neo":{"command":"also-foreign"}},"keep":true}"#;
    fs::write(&foreign_config, foreign_raw)?;
    let mut unmatched = MigrateServerInput::new(
        "BrowserClaw",
        McpServer {
            name: "BrowserOS neo".to_string(),
            spec: legacy_spec.clone(),
        },
        AgentId::Cursor,
    );
    unmatched.config_path = Some(foreign_config.clone());
    assert!(!foreign_manager.migrate_server(unmatched)?.migrated);
    assert_eq!(fs::read_to_string(&foreign_config)?, foreign_raw);
    assert!(!foreign_workspace.join("manifest.json").exists());

    let exact_workspace = root.path().join("exact-workspace");
    let exact_manager = McpManager::new(&exact_workspace);
    let exact_config = root.path().join("exact/cursor.json");
    fs::create_dir_all(exact_config.parent().ok_or("missing exact parent")?)?;
    fs::write(
        &exact_config,
        r#"{"mcpServers":{"BrowserClaw":{"url":"http://127.0.0.1:9100/mcp","type":"http"}}}"#,
    )?;
    let observed = exact_manager
        .inspect_entry(
            InspectEntryInput::new("BrowserClaw", AgentId::Cursor).at_path(&exact_config),
        )?
        .ok_or("missing exact source")?;
    let mut exact = MigrateServerInput::new(
        "BrowserClaw",
        McpServer {
            name: "BrowserOS neo".to_string(),
            spec: legacy_spec,
        },
        AgentId::Cursor,
    );
    exact.config_path = Some(exact_config.clone());
    exact.unmanaged_source_spec = Some(observed.spec);
    assert!(exact_manager.migrate_server(exact)?.migrated);
    assert!(fs::read_to_string(exact_config)?.contains("BrowserOS neo"));
    Ok(())
}

#[test]
fn migration_converges_from_interrupted_install_and_cleanup_states()
-> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let legacy_spec = McpServerSpec::Http {
        url: "http://127.0.0.1:9100/mcp".to_string(),
        headers: BTreeMap::new(),
    };
    let canonical_spec = McpServerSpec::Http {
        url: "http://127.0.0.1:9200/mcp".to_string(),
        headers: BTreeMap::new(),
    };

    let installed_workspace = root.path().join("installed-workspace");
    let installed_manager = McpManager::new(&installed_workspace);
    let legacy_config = root.path().join("installed/legacy-cursor.json");
    let canonical_config = root.path().join("installed/canonical-cursor.json");
    fs::create_dir_all(legacy_config.parent().ok_or("missing installed parent")?)?;
    installed_manager.link(link_input(
        McpServer {
            name: "BrowserClaw".to_string(),
            spec: legacy_spec.clone(),
        },
        AgentId::Cursor,
        &legacy_config,
    ))?;
    installed_manager.link(link_input(
        McpServer {
            name: "BrowserOS neo".to_string(),
            spec: canonical_spec.clone(),
        },
        AgentId::Cursor,
        &canonical_config,
    ))?;
    let canonical_before = installed_manager
        .list()?
        .into_iter()
        .find(|server| server.name == "BrowserOS neo")
        .ok_or("missing canonical manifest entry")?;
    let mut installed_migration = MigrateServerInput::new(
        "BrowserClaw",
        McpServer {
            name: "BrowserOS neo".to_string(),
            spec: canonical_spec.clone(),
        },
        AgentId::Cursor,
    );
    installed_migration.config_path = Some(legacy_config.clone());
    assert!(
        installed_manager
            .migrate_server(installed_migration)?
            .migrated
    );
    let installed_servers = installed_manager.list()?;
    assert_eq!(installed_servers.len(), 1);
    assert_eq!(installed_servers[0].name, "BrowserOS neo");
    assert_eq!(installed_servers[0].added_at, canonical_before.added_at);
    assert_eq!(
        installed_servers[0].links[&AgentId::Cursor].created_at,
        canonical_before.links[&AgentId::Cursor].created_at
    );
    assert_eq!(
        installed_servers[0].links[&AgentId::Cursor].config_path,
        canonical_config
    );
    let legacy_raw = fs::read_to_string(legacy_config)?;
    assert!(!legacy_raw.contains("BrowserClaw"));
    assert!(!legacy_raw.contains("BrowserOS neo"));
    let canonical_raw = fs::read_to_string(canonical_config)?;
    assert!(canonical_raw.contains("BrowserOS neo"));
    assert!(!canonical_raw.contains("BrowserClaw"));

    let cleaned_workspace = root.path().join("cleaned-workspace");
    let cleaned_manager = McpManager::new(&cleaned_workspace);
    let cleaned_config = root.path().join("cleaned/cursor.json");
    fs::create_dir_all(cleaned_config.parent().ok_or("missing cleaned parent")?)?;
    cleaned_manager.link(link_input(
        McpServer {
            name: "BrowserClaw".to_string(),
            spec: legacy_spec,
        },
        AgentId::Cursor,
        &cleaned_config,
    ))?;
    fs::write(&cleaned_config, r#"{"mcpServers":{}}"#)?;
    let mut cleaned_migration = MigrateServerInput::new(
        "BrowserClaw",
        McpServer {
            name: "BrowserOS neo".to_string(),
            spec: canonical_spec,
        },
        AgentId::Cursor,
    );
    cleaned_migration.config_path = Some(cleaned_config.clone());
    assert!(cleaned_manager.migrate_server(cleaned_migration)?.migrated);
    let cleaned_servers = cleaned_manager.list()?;
    assert_eq!(cleaned_servers.len(), 1);
    assert_eq!(cleaned_servers[0].name, "BrowserOS neo");
    let cleaned_raw = fs::read_to_string(cleaned_config)?;
    assert!(cleaned_raw.contains("BrowserOS neo"));
    assert!(!cleaned_raw.contains("BrowserClaw"));
    Ok(())
}

#[test]
fn install_gate_and_project_scope_return_typed_errors() -> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let manager = McpManager::new(root.path().join("workspace"));
    let missing = root.path().join("missing/cursor.json");
    let error = manager
        .link(link_input(stdio_server("gh"), AgentId::Cursor, &missing))
        .err()
        .ok_or("expected install error")?;
    assert!(matches!(
        error,
        Error::AgentNotInstalled {
            agent: AgentId::Cursor,
            ..
        }
    ));

    let config = root.path().join("cursor.json");
    let mut project = link_input(stdio_server("gh"), AgentId::Cursor, &config);
    project.scope = AgentScope::Project;
    assert!(matches!(
        manager.link(project),
        Err(Error::UnresolvedConfigPath { .. })
    ));
    Ok(())
}

#[test]
fn malformed_manifest_is_never_silently_reset() -> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let workspace = root.path().join("workspace");
    fs::create_dir_all(&workspace)?;
    fs::write(workspace.join("manifest.json"), "{ broken")?;
    let manager = McpManager::new(workspace);
    assert!(matches!(manager.list(), Err(Error::Manifest { .. })));
    Ok(())
}

#[test]
fn recent_catalog_path_fixes_match_typescript() -> Result<(), Box<dyn std::error::Error>> {
    let opencode = resolve_agent_surface(AgentId::OpenCode, AgentScope::System)?;
    assert_eq!(
        opencode.harness.install_check_paths.darwin,
        &[
            "$XDG_CONFIG_HOME/opencode",
            "$HOME/.config/opencode",
            "$HOME/.opencode",
            "$HOME/.local/share/opencode",
        ]
    );
    assert_eq!(
        opencode.harness.install_check_paths.linux,
        &[
            "$XDG_CONFIG_HOME/opencode",
            "$HOME/.config/opencode",
            "$HOME/.opencode",
            "$HOME/.local/share/opencode",
        ]
    );
    assert_eq!(
        opencode.harness.install_check_paths.windows,
        &[
            "$USERPROFILE\\.config\\opencode",
            "$USERPROFILE\\.opencode",
            "$USERPROFILE\\.local\\share\\opencode",
        ]
    );
    assert_eq!(
        opencode.mcp.system_paths.darwin,
        &[
            "$XDG_CONFIG_HOME/opencode/opencode.json",
            "$HOME/.config/opencode/opencode.json",
            "$XDG_CONFIG_HOME/opencode/opencode.jsonc",
            "$HOME/.config/opencode/opencode.jsonc",
            "$HOME/.opencode/opencode.jsonc",
        ]
    );

    let antigravity = resolve_agent_surface(AgentId::Antigravity, AgentScope::System)?;
    assert_eq!(
        antigravity.harness.install_check_paths.darwin,
        &["$HOME/.gemini/antigravity"]
    );
    assert_eq!(
        antigravity.mcp.system_paths.darwin,
        &["$HOME/.gemini/config/mcp_config.json"]
    );
    assert_eq!(
        antigravity.mcp.system_paths.linux,
        &["$HOME/.gemini/config/mcp_config.json"]
    );
    assert_eq!(
        antigravity.mcp.system_paths.windows,
        &["$USERPROFILE\\.gemini\\config\\mcp_config.json"]
    );
    Ok(())
}

#[test]
fn opencode_install_fingerprint_drives_probe_and_default_link_gate()
-> Result<(), Box<dyn std::error::Error>> {
    const CHILD_HOME: &str = "AGENT_MCP_MANAGER_OPENCODE_TEST_HOME";
    if let Some(home) = env::var_os(CHILD_HOME) {
        return exercise_opencode_install_fingerprint(&PathBuf::from(home));
    }

    let root = tempdir()?;
    let output = Command::new(env::current_exe()?)
        .args([
            "--exact",
            "opencode_install_fingerprint_drives_probe_and_default_link_gate",
            "--nocapture",
        ])
        .env(CHILD_HOME, root.path())
        .env("HOME", root.path())
        .env("USERPROFILE", root.path())
        .env("XDG_CONFIG_HOME", root.path().join(".config"))
        .output()?;
    if !output.status.success() {
        return Err(std::io::Error::other(format!(
            "isolated OpenCode regression failed\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ))
        .into());
    }
    Ok(())
}

fn exercise_opencode_install_fingerprint(home: &Path) -> Result<(), Box<dyn std::error::Error>> {
    fs::create_dir_all(home.join(".local/share/opencode"))?;
    assert_eq!(
        is_installed(&[AgentId::OpenCode])?.get(&AgentId::OpenCode),
        Some(&true)
    );

    let manager = McpManager::new(home.join("manager-workspace"));
    manager.link(LinkInput::new(stdio_server("browseros"), AgentId::OpenCode))?;
    let config_path = home.join(".config/opencode/opencode.json");
    let config: Value = serde_json::from_str(&fs::read_to_string(&config_path)?)?;
    assert_eq!(config["mcp"]["browseros"]["command"][0], "browseros-mcp");

    let override_path = home.join("custom/missing/opencode.json");
    let mut explicit = LinkInput::new(stdio_server("strict"), AgentId::OpenCode);
    explicit.config_path = Some(override_path.clone());
    assert!(matches!(
        manager.link(explicit),
        Err(Error::AgentNotInstalled { config_path, .. }) if config_path == override_path
    ));
    Ok(())
}

#[test]
fn public_agent_helpers_expose_exactly_the_seven_harness_targets()
-> Result<(), Box<dyn std::error::Error>> {
    assert_eq!(
        list_supported_agents(),
        vec![
            AgentId::ClaudeCode,
            AgentId::Codex,
            AgentId::Cursor,
            AgentId::OpenCode,
            AgentId::Antigravity,
            AgentId::VsCode,
            AgentId::Zed,
        ]
    );
    assert!(is_agent_supported("claude-code"));
    assert!(!is_agent_supported("toString"));
    assert_eq!(serde_json::to_string(&AgentId::OpenCode)?, "\"opencode\"");
    assert_eq!(serde_json::to_string(&AgentId::VsCode)?, "\"vscode\"");
    assert_eq!(
        resolve_agent_surface(AgentId::Codex, AgentScope::System)?.supported_transports,
        &[
            harness_integrations::McpTransport::Stdio,
            harness_integrations::McpTransport::Http,
        ]
    );
    let claude = resolve_agent_surface(AgentId::ClaudeCode, AgentScope::System)?.mcp;
    assert_eq!(
        claude.system_paths.darwin,
        &["$CLAUDE_CONFIG_DIR/.claude.json", "$HOME/.claude.json"]
    );
    assert_eq!(
        claude.http.and_then(|shape| shape.sse_tag_value),
        Some("sse")
    );
    let codex = resolve_agent_surface(AgentId::Codex, AgentScope::System)?.mcp;
    assert_eq!(
        codex.http.and_then(|shape| shape.header_field),
        Some("http_headers")
    );
    let opencode = resolve_agent_surface(AgentId::OpenCode, AgentScope::System)?.mcp;
    assert!(opencode.stdio.command_as_array);
    assert_eq!(opencode.stdio.env_field, Some("environment"));
    let codex = resolve_harness_definition(AgentId::Codex);
    let zed = resolve_harness_definition(AgentId::Zed);
    assert_ne!(codex.mcp.system_paths.darwin, zed.mcp.system_paths.darwin);
    assert_eq!(
        codex.skill.map(|surface| surface.global_roots.darwin),
        zed.skill.map(|surface| surface.global_roots.darwin)
    );
    assert_eq!(detect_installed_agents()?.len(), 7);
    assert!(is_installed(&[])?.is_empty());
    Ok(())
}
