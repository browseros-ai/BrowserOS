//! Exercises background reconciliation through the same interface as the server,
//! using real config files to detect lost comments, options or unrelated entries.
use harness_integrations::{AgentId, McpManager, McpServer, McpServerSpec, ReconcileInput};
use std::{collections::BTreeMap, fs, path::Path};

fn input(agent: AgentId, path: &Path, port: u16) -> ReconcileInput {
    let mut input = ReconcileInput::new(
        McpServer {
            name: "browseros-neo".into(),
            spec: McpServerSpec::Http {
                url: format!("http://127.0.0.1:{port}/mcp"),
                headers: BTreeMap::new(),
            },
        },
        agent,
    );
    input.config_path = Some(path.into());
    input.aliases = vec!["BrowserClaw".into(), "BrowserOS neo".into()];
    input
}

#[test]
fn existing_config_changes_only_its_endpoint_port() -> Result<(), Box<dyn std::error::Error>> {
    let root = tempfile::tempdir()?;
    let manager = McpManager::new(root.path().join("state"));
    let config = root.path().join("cursor.json");
    manager.reconcile(input(AgentId::Cursor, &config, 9001))?;
    let customized = "{\n  // keep me\n  \"mcpServers\": {\n    \"browseros-neo\": {\"url\": \"http://127.0.0.1:9001/mcp\", \"disabled\": true, \"timeout\": 123, \"headers\": {\"X-Custom\": \"keep\"}},\n    \"other\": {\"url\": \"http://127.0.0.1:9001/other\"}\n  }\n}\n";
    fs::write(&config, customized)?;
    let modified = fs::metadata(&config)?.modified()?;
    manager.reconcile(input(AgentId::Cursor, &config, 9001))?;
    assert_eq!(fs::read_to_string(&config)?, customized);
    assert_eq!(fs::metadata(&config)?.modified()?, modified);
    let updated = manager.reconcile(input(AgentId::Cursor, &config, 9002))?;
    assert!(updated.updated);
    assert_eq!(
        fs::read_to_string(&config)?,
        customized.replace("9001/mcp", "9002/mcp")
    );
    let manifest_modified = fs::metadata(root.path().join("state/manifest.json"))?.modified()?;
    manager.reconcile(input(AgentId::Cursor, &config, 9002))?;
    assert_eq!(
        fs::metadata(root.path().join("state/manifest.json"))?.modified()?,
        manifest_modified
    );
    Ok(())
}

#[test]
fn preserves_toml_options_and_adopts_alias_without_renaming()
-> Result<(), Box<dyn std::error::Error>> {
    let root = tempfile::tempdir()?;
    let manager = McpManager::new(root.path().join("state"));
    let config = root.path().join("config.toml");
    let raw = "# settings\nmodel = \"custom\"\n[mcp_servers.\"BrowserClaw\"]\nurl = \"http://127.0.0.1:9001/mcp?tag=keep#anchor\" # endpoint\nenabled = false\nstartup_timeout_sec = 42\nhttp_headers = { X = \"keep\" }\n";
    fs::write(&config, raw)?;
    assert!(
        manager
            .reconcile(input(AgentId::Codex, &config, 9001))?
            .connected
    );
    assert_eq!(fs::read_to_string(&config)?, raw);
    assert!(
        manager
            .reconcile(input(AgentId::Codex, &config, 9002))?
            .updated
    );
    assert_eq!(
        fs::read_to_string(&config)?,
        raw.replace(":9001/", ":9002/")
    );
    Ok(())
}

#[test]
fn preserves_bridge_arguments_and_foreign_endpoints() -> Result<(), Box<dyn std::error::Error>> {
    let root = tempfile::tempdir()?;
    let manager = McpManager::new(root.path().join("state"));
    let config = root.path().join("cursor.json");
    let raw = r#"{"mcpServers":{"browseros-neo":{"command":"npx","args":["mcp-remote","http://127.0.0.1:9001/mcp","--header","X: keep"],"env":{"KEEP":"yes"}}}}"#;
    fs::write(&config, raw)?;
    assert!(
        manager
            .reconcile(input(AgentId::Cursor, &config, 9002))?
            .updated
    );
    assert_eq!(
        fs::read_to_string(&config)?,
        raw.replace(":9001/", ":9002/")
    );
    for url in [
        "https://example.com/mcp",
        "http://127.0.0.1:9001/custom",
        "http://user@127.0.0.1:9001/mcp",
    ] {
        let custom = raw.replace("http://127.0.0.1:9001/mcp", url);
        fs::write(&config, &custom)?;
        assert!(
            !manager
                .reconcile(input(AgentId::Cursor, &config, 9002))?
                .connected
        );
        assert_eq!(fs::read_to_string(&config)?, custom);
    }
    Ok(())
}

#[test]
fn recorded_paths_and_multiple_agents_recover_independently()
-> Result<(), Box<dyn std::error::Error>> {
    let root = tempfile::tempdir()?;
    let manager = McpManager::new(root.path().join("state"));
    let configs = [
        (AgentId::Codex, "custom.toml"),
        (AgentId::Cursor, "custom.json"),
    ];
    for (agent, path) in configs {
        assert!(
            manager
                .reconcile(input(agent, &root.path().join(path), 9001))?
                .created
        );
    }
    // One migrated manifest entry must not hide another app's old port. A later
    // pass resolves the recorded path even without an explicit path override.
    for (agent, path) in configs {
        let mut request = input(agent, &root.path().join(path), 9002);
        request.config_path = None;
        assert!(manager.reconcile(request)?.updated);
        assert!(fs::read_to_string(root.path().join(path))?.contains(":9002/mcp"));
    }
    Ok(())
}

#[test]
fn disconnects_unrecorded_local_entry_without_touching_foreign_entries()
-> Result<(), Box<dyn std::error::Error>> {
    let root = tempfile::tempdir()?;
    let manager = McpManager::new(root.path().join("state"));
    let config = root.path().join("custom.json");
    let raw = r#"{"mcpServers":{"browseros-neo":{"url":"http://127.0.0.1:9011/mcp","headers":{"X":"keep"}},"BrowserClaw":{"url":"https://example.com/mcp"}},"keep":true}"#;
    fs::write(&config, raw)?;
    let mut request = harness_integrations::DisconnectInput::new("browseros-neo", AgentId::Cursor);
    request.config_path = Some(config.clone());
    request.unmanaged_endpoint = Some("http://127.0.0.1/mcp".into());
    assert!(manager.disconnect(request.clone())?.unlinked);
    let remaining = fs::read_to_string(&config)?;
    assert!(!remaining.contains("browseros-neo"));
    assert!(remaining.contains("https://example.com/mcp"));
    request.server_name = "BrowserClaw".into();
    assert!(!manager.disconnect(request)?.unlinked);
    assert_eq!(fs::read_to_string(&config)?, remaining);
    assert!(!root.path().join("state/manifest.json").exists());
    Ok(())
}
