//! Real harness-integrations config mutations in a subprocess with a temporary
//! HOME. The v1 manifest/config fixture is produced by the actual TS manager,
//! proving adoption rather than assuming compatible struct names.

use axum::{
    Router,
    body::{Body, to_bytes},
    routing::{get, post},
};
use harness_integrations::{
    AgentId, AgentScope, ConfigFormat, InspectEntryInput, McpManager, McpServerSpec,
    resolve_agent_mcp_config_path, resolve_agent_surface,
};
use http::{Request, StatusCode};
use serde_json::{Value, json};
use server_rust::{
    http::mcp_manager::{self as routes, ManagerState},
    klavis::Klavis,
    mcp_manager::McpManagerService,
};
use std::{
    path::{Path, PathBuf},
    process::Command,
};
use tower::ServiceExt;

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;
const PROXY: &str = "http://127.0.0.1:19100/mcp";

#[tokio::test]
async fn manager_temp_home() -> Result {
    if let Some(root) = std::env::var_os("SR_KLAVIS_TEST_HOME") {
        return acceptance(PathBuf::from(root)).await;
    }
    let dir = tempfile::tempdir()?;
    let output = Command::new(std::env::current_exe()?)
        .args(["--exact", "manager_temp_home", "--nocapture"])
        .env("SR_KLAVIS_TEST_HOME", dir.path())
        .env("HOME", dir.path())
        .env("USERPROFILE", dir.path())
        .env("XDG_CONFIG_HOME", dir.path().join(".config"))
        .env("APPDATA", dir.path().join("AppData/Roaming"))
        .env("LOCALAPPDATA", dir.path().join("AppData/Local"))
        .output()?;
    assert!(
        output.status.success(),
        "isolated acceptance failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(())
}
async fn acceptance(root: PathBuf) -> Result {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let manifest_dir = root.join("data/mcp-manager");
    let manager = McpManagerService::new(manifest_dir.clone(), Some(PROXY.into()));
    // Config inspection does not need the mixed legacy manifest. The service
    // owns projection/adoption; the harness inspector reads each explicit path.
    let library = McpManager::new(root.join("inspect-only"));
    let mut paths = Vec::new();
    for id in AgentId::ALL {
        let path = resolve_agent_mcp_config_path(id, AgentScope::System)?;
        assert!(
            path.starts_with(&root),
            "config escaped temporary HOME: {}",
            path.display()
        );
        std::fs::create_dir_all(path.parent().ok_or("config parent")?)?;
        let raw = match resolve_agent_surface(id, AgentScope::System)?.mcp.format {
            ConfigFormat::Toml => "# keep this comment\nmodel = \"fixture\"\n",
            ConfigFormat::Jsonc => "{\n// keep this comment\n\"fixture\": true\n}\n",
            ConfigFormat::Json => "{\"fixture\":true}\n",
        };
        std::fs::write(&path, raw)?;
        paths.push((id, path));
    }
    // Run the source TS package. Explicit config paths avoid macOS application
    // detection leaking into this fixture and also cover imported custom paths.
    let script = r#"
      import { bind } from './packages/agent-mcp-manager/src/index.ts';
      import { writeFile, mkdir } from 'node:fs/promises';
      const root = process.env.SR_KLAVIS_TEST_HOME;
      const paths = JSON.parse(process.env.SR_KLAVIS_AGENT_PATHS);
      const manager = bind(root + '/data/mcp-manager');
      for (const [agent, configPath] of paths) {
        await manager.link({server:{name:'browseros',spec:{transport:'http',url:'http://127.0.0.1:19000/mcp'}},agent,configPath,allowOverwrite:true});
      }
      await mkdir(root + '/legacy', {recursive:true});
      const legacy = root + '/legacy/gemini.json';
      await writeFile(legacy, '{}');
      await manager.link({server:{name:'browseros',spec:{transport:'http',url:'http://127.0.0.1:19000/mcp'}},agent:'gemini',configPath:legacy,allowOverwrite:true});
    "#;
    // Bun is installed outside the temporary HOME; use its resolved executable.
    let bun = std::env::var_os("SR_KLAVIS_BUN")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("bun"));
    let output = Command::new(bun)
        .current_dir(workspace)
        .args(["-e", script])
        .env(
            "SR_KLAVIS_AGENT_PATHS",
            serde_json::to_string(
                &paths
                    .iter()
                    .map(|(id, p)| (id.as_str(), p))
                    .collect::<Vec<_>>(),
            )?,
        )
        .output()?;
    assert!(
        output.status.success(),
        "TS fixture capture failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    // TS's TOML emitter strips comments; add a fresh one after capture so the
    // Rust maintenance pass proves it preserves the user's subsequent edits.
    for (id, path) in &paths {
        if matches!(
            resolve_agent_surface(*id, AgentScope::System)?.mcp.format,
            ConfigFormat::Toml
        ) {
            let raw = std::fs::read_to_string(path)?;
            std::fs::write(path, format!("# keep this comment\n{raw}"))?;
        }
    }
    let legacy_before = std::fs::read(root.join("legacy/gemini.json"))?;
    let rows = manager.agents().await?;
    assert_eq!(
        rows.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
        AgentId::ALL.map(AgentId::as_str)
    );
    assert!(rows.iter().all(|r| r.installed && r.linked));
    assert!(
        rows.iter()
            .all(|r| r.config_path.as_ref().is_some_and(|p| p.starts_with(&root)))
    );
    manager.self_heal().await;
    for (id, path) in &paths {
        let entry =
            library.inspect_entry(InspectEntryInput::new("browseros", *id).at_path(path))?;
        let entry = entry.ok_or("self-heal did not retain an emitted entry")?;
        assert_eq!(
            entry.spec,
            McpServerSpec::Http {
                url: PROXY.into(),
                headers: Default::default()
            }
        );
        let raw = std::fs::read_to_string(path)?;
        assert!(raw.contains("fixture"));
        if matches!(
            resolve_agent_surface(*id, AgentScope::System)?.mcp.format,
            ConfigFormat::Jsonc | ConfigFormat::Toml
        ) {
            assert!(raw.contains("keep this comment"));
        }
    }
    // An older non-curated manifest link is opaque to the Rust catalog. It
    // survives every operation and its config is never rewritten or deleted.
    assert_eq!(
        std::fs::read(root.join("legacy/gemini.json"))?,
        legacy_before
    );
    let manifest: Value =
        serde_json::from_slice(&std::fs::read(manifest_dir.join("manifest.json"))?)?;
    assert!(manifest["servers"]["browseros"]["links"]["gemini"].is_object());
    let claude = paths
        .iter()
        .find(|(id, _)| *id == AgentId::ClaudeCode)
        .ok_or("claude")?
        .1
        .clone();
    let mut config: Value = serde_json::from_slice(&std::fs::read(&claude)?)?;
    config["mcpServers"]["browseros"]["headers"] = json!({"Authorization":"user-customization"});
    std::fs::write(&claude, config.to_string())?;
    let edited = std::fs::read(&claude)?;
    manager.self_heal().await;
    assert_eq!(std::fs::read(&claude)?, edited);
    let error = manager
        .uninstall("claude-code")
        .await
        .err()
        .ok_or("edited config was deleted")?;
    assert_eq!(error.status(), StatusCode::CONFLICT);
    assert!(error.0.starts_with("Cannot remove a user-edited entry."));
    assert_eq!(std::fs::read(&claude)?, edited);
    // An explicit user install may replace the prior BrowserOS entry (TS parity).
    manager.install_at("claude-code", PROXY).await?;
    let once = std::fs::read(&claude)?;
    manager.install_at("claude-code", PROXY).await?;
    assert_eq!(std::fs::read(&claude)?, once);

    let klavis = Klavis::start_with_upstream(
        String::new(),
        reqwest::Client::new(),
        "http://127.0.0.1:9".into(),
        vec![],
    );
    let state = ManagerState {
        manager: manager.clone(),
        klavis: klavis.clone(),
        server_port: 19200,
    };
    let router = Router::new()
        .route("/tools", get(routes::tools))
        .route("/agents", get(routes::agents))
        .route("/agents/{id}/install", post(routes::install))
        .route("/agents/{id}/uninstall", post(routes::uninstall))
        .with_state(state);
    let (_, body) = request(router.clone(), "/tools", "", None).await?;
    assert!(
        body["tools"]
            .as_array()
            .ok_or("tools")?
            .iter()
            .any(|t| t["name"] == "run")
    );
    let (_, body) = request(router.clone(), "/agents", "", None).await?;
    assert_eq!(body["agents"][0]["displayName"], "Claude Code");
    assert!(body["agents"][0].get("detected").is_none());
    for invalid in ["null", "[]", r#"{"mcpUrl":null}"#, r#"{"mcpUrl":"bad"}"#] {
        let (status, body) = request(
            router.clone(),
            "/agents/claude-code/install",
            invalid,
            Some("POST"),
        )
        .await?;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(
            body,
            json!({"success":false,"message":"Invalid request body: mcpUrl must be a URL."})
        );
    }
    let (status, body) =
        request(router.clone(), "/agents/unknown/install", "{", Some("POST")).await?;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["message"], "Agent \"unknown\" is not supported.");
    // Malformed JSON is {}, and therefore uses the actual server port fallback.
    let (status, _) = request(
        router.clone(),
        "/agents/claude-code/install",
        "{",
        Some("POST"),
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    assert!(std::fs::read_to_string(&claude)?.contains("19200"));
    let (status, _) = request(
        router.clone(),
        "/agents/claude-code/install",
        &json!({"mcpUrl":PROXY}).to_string(),
        Some("POST"),
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    assert!(std::fs::read_to_string(&claude)?.contains(PROXY));
    for (id, _) in &paths {
        manager.uninstall(id.as_str()).await?;
        manager.uninstall(id.as_str()).await?;
    }
    assert!(manager.agents().await?.iter().all(|r| !r.linked));
    // Missing configs are not resurrected by the boot pass.
    manager.install_at("claude-code", PROXY).await?;
    std::fs::remove_file(&claude)?;
    manager.self_heal().await;
    assert!(!claude.exists());
    assert_eq!(
        std::fs::read(root.join("legacy/gemini.json"))?,
        legacy_before
    );
    klavis.stop().await;
    Ok(())
}
async fn request(
    router: Router,
    path: &str,
    body: &str,
    method: Option<&str>,
) -> Result<(StatusCode, Value)> {
    let response = router
        .oneshot(
            Request::builder()
                .method(method.unwrap_or("GET"))
                .uri(path)
                .body(Body::from(body.to_owned()))?,
        )
        .await?;
    Ok((
        response.status(),
        serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await?)?,
    ))
}
