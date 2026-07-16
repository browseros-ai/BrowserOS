use std::{collections::BTreeMap, fs, path::Path};

use agent_mcp_manager::{
    AgentId, AgentScope, DisconnectInput, Error, LinkInput, ListLinksFilter, Manager, McpServer,
    McpServerSpec, UnlinkInput, detect_installed_agents, is_agent_supported, is_installed,
    list_supported_agents, resolve_agent_surface,
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
    let manager = Manager::new(root.path().join("workspace"));
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
    let manager = Manager::new(root.path().join("workspace"));
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
    let manager = Manager::new(root.path().join("workspace"));
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
    let manager = Manager::new(&workspace);
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
    let manager = Manager::new(root.path().join("workspace"));
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
    let manager = Manager::new(root.path().join("workspace"));
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
    let manager = Manager::new(root.path().join("workspace"));
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
    let manager = Manager::new(root.path().join("workspace"));
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
fn install_gate_and_project_scope_return_typed_errors() -> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let manager = Manager::new(root.path().join("workspace"));
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
    let manager = Manager::new(workspace);
    assert!(matches!(manager.list(), Err(Error::Manifest { .. })));
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
            agent_mcp_manager::McpTransport::Stdio,
            agent_mcp_manager::McpTransport::Http,
        ]
    );
    let claude = resolve_agent_surface(AgentId::ClaudeCode, AgentScope::System)?.client;
    assert_eq!(
        claude.system_paths.darwin,
        &["$CLAUDE_CONFIG_DIR/.claude.json", "$HOME/.claude.json"]
    );
    assert_eq!(
        claude.http.and_then(|shape| shape.sse_tag_value),
        Some("sse")
    );
    let codex = resolve_agent_surface(AgentId::Codex, AgentScope::System)?.client;
    assert_eq!(
        codex.http.and_then(|shape| shape.header_field),
        Some("http_headers")
    );
    let opencode = resolve_agent_surface(AgentId::OpenCode, AgentScope::System)?.client;
    assert!(opencode.stdio.command_as_array);
    assert_eq!(opencode.stdio.env_field, Some("environment"));
    assert_eq!(detect_installed_agents()?.len(), 7);
    assert!(is_installed(&[])?.is_empty());
    Ok(())
}
