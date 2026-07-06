pub mod manifest;
pub mod plan;
pub mod surfaces;

use crate::{
    error::{AppError, AppResult},
    services::agents::Harness,
};
use manifest::{HarnessLink, HarnessManifest};
use plan::{PlanAction, PlanOutcome};
use std::{collections::BTreeMap, path::PathBuf, sync::Arc};
use surfaces::{McpServerSpec, SurfacePaths, config_path_for};
use tokio::sync::Mutex;

pub const BROWSEROS_MCP_SERVER_NAME: &str = "BrowserClaw";

#[derive(Clone)]
pub struct HarnessService {
    workspace_dir: PathBuf,
    home_dir: PathBuf,
    paths: SurfacePaths,
    mutex: Arc<Mutex<()>>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionState {
    pub harness: Harness,
    pub installed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config_path: Option<String>,
    pub agent_id: Option<String>,
    pub message: String,
}

impl HarnessService {
    #[must_use]
    pub fn new(workspace_dir: PathBuf, home_dir: PathBuf) -> Self {
        Self {
            workspace_dir,
            home_dir,
            paths: SurfacePaths::default(),
            mutex: Arc::new(Mutex::new(())),
        }
    }

    #[must_use]
    pub fn with_paths(workspace_dir: PathBuf, home_dir: PathBuf, paths: SurfacePaths) -> Self {
        Self {
            workspace_dir,
            home_dir,
            paths,
            mutex: Arc::new(Mutex::new(())),
        }
    }

    pub async fn connect_browseros(
        &self,
        harness: Harness,
        mcp_url: &str,
    ) -> AppResult<ConnectionState> {
        self.connect(harness, BROWSEROS_MCP_SERVER_NAME, mcp_url, true)
            .await
    }

    pub async fn disconnect_browseros(&self, harness: Harness) -> AppResult<ConnectionState> {
        self.disconnect(harness, BROWSEROS_MCP_SERVER_NAME).await
    }

    pub async fn list_browseros_connections(&self) -> AppResult<Vec<ConnectionState>> {
        let manifest = self.load_manifest().await?;
        let mut by_agent: BTreeMap<String, HarnessLink> = BTreeMap::new();
        for link in manifest.links {
            if link.server_name == BROWSEROS_MCP_SERVER_NAME {
                by_agent.insert(link.agent.clone(), link);
            }
        }
        Ok(Harness::ALL
            .iter()
            .copied()
            .map(|harness| {
                let Some(agent_id) = harness.agent_id() else {
                    return ConnectionState {
                        harness,
                        installed: true,
                        config_path: None,
                        agent_id: None,
                        message: format!("{harness} runs inside BrowserOS."),
                    };
                };
                if let Some(link) = by_agent.get(agent_id) {
                    return ConnectionState {
                        harness,
                        installed: true,
                        config_path: Some(link.config_path.clone()),
                        agent_id: Some(agent_id.to_string()),
                        message: format!("Configured in {harness}."),
                    };
                }
                ConnectionState {
                    harness,
                    installed: false,
                    config_path: None,
                    agent_id: Some(agent_id.to_string()),
                    message: format!("{harness} is not configured."),
                }
            })
            .collect())
    }

    pub async fn dry_run(
        &self,
        harness: Harness,
        server_name: &str,
        mcp_url: &str,
    ) -> AppResult<PlanOutcome> {
        let spec = spec_for(harness, mcp_url);
        let config_path = config_path_for(harness, &self.home_dir, &self.paths)?;
        plan::build_plan(PlanAction::Connect, harness, server_name, spec, config_path).await
    }

    pub async fn heal_claude_code_http_tags(&self) -> AppResult<usize> {
        let path = config_path_for(Harness::ClaudeCode, &self.home_dir, &self.paths)?;
        surfaces::heal_claude_code_http_tags(&path).await
    }

    async fn connect(
        &self,
        harness: Harness,
        server_name: &str,
        mcp_url: &str,
        allow_overwrite: bool,
    ) -> AppResult<ConnectionState> {
        let Some(agent_id) = harness.agent_id() else {
            return Ok(ConnectionState {
                harness,
                installed: true,
                config_path: None,
                agent_id: None,
                message: format!("{harness} runs inside BrowserOS; no harness config to write."),
            });
        };
        let _guard = self.mutex.lock().await;
        let spec = spec_for(harness, mcp_url);
        let config_path = config_path_for(harness, &self.home_dir, &self.paths)?;
        let outcome = plan::build_plan(
            PlanAction::Connect,
            harness,
            server_name,
            spec.clone(),
            config_path.clone(),
        )
        .await?;
        outcome.apply(allow_overwrite).await?;
        let mut manifest = self.load_manifest().await?;
        manifest.upsert_server(server_name, spec);
        manifest.upsert_link(HarnessLink {
            server_name: server_name.to_string(),
            agent: agent_id.to_string(),
            config_path: config_path.to_string_lossy().to_string(),
        });
        self.save_manifest(&manifest).await?;
        Ok(ConnectionState {
            harness,
            installed: true,
            config_path: Some(config_path.to_string_lossy().to_string()),
            agent_id: Some(agent_id.to_string()),
            message: format!("BrowserOS registered as an MCP server in {harness}."),
        })
    }

    async fn disconnect(&self, harness: Harness, server_name: &str) -> AppResult<ConnectionState> {
        let Some(agent_id) = harness.agent_id() else {
            return Ok(ConnectionState {
                harness,
                installed: false,
                config_path: None,
                agent_id: None,
                message: format!("{harness} runs inside BrowserOS; nothing to disconnect."),
            });
        };
        let _guard = self.mutex.lock().await;
        let config_path = config_path_for(harness, &self.home_dir, &self.paths)?;
        let outcome = plan::build_plan(
            PlanAction::Disconnect,
            harness,
            server_name,
            McpServerSpec::Http {
                url: String::new(),
                headers: BTreeMap::new(),
            },
            config_path.clone(),
        )
        .await?;
        outcome.apply(false).await?;
        let mut manifest = self.load_manifest().await?;
        manifest.remove_link(server_name, agent_id);
        if !manifest.has_links(server_name) {
            manifest.remove_server(server_name);
        }
        self.save_manifest(&manifest).await?;
        Ok(ConnectionState {
            harness,
            installed: false,
            config_path: Some(config_path.to_string_lossy().to_string()),
            agent_id: Some(agent_id.to_string()),
            message: format!("BrowserOS unregistered from {harness}."),
        })
    }

    async fn load_manifest(&self) -> AppResult<HarnessManifest> {
        HarnessManifest::load(&self.workspace_dir).await
    }

    async fn save_manifest(&self, manifest: &HarnessManifest) -> AppResult<()> {
        manifest.save(&self.workspace_dir).await
    }
}

#[must_use]
pub fn spec_for(harness: Harness, mcp_url: &str) -> McpServerSpec {
    if supports_http(harness) {
        return McpServerSpec::Http {
            url: mcp_url.to_string(),
            headers: BTreeMap::new(),
        };
    }
    McpServerSpec::Stdio {
        command: "npx".to_string(),
        args: vec!["mcp-remote".to_string(), mcp_url.to_string()],
        env: BTreeMap::new(),
    }
}

fn supports_http(harness: Harness) -> bool {
    matches!(
        harness,
        Harness::ClaudeCode
            | Harness::Cursor
            | Harness::VsCode
            | Harness::Zed
            | Harness::Codex
            | Harness::GeminiCli
    )
}

impl From<std::io::Error> for AppError {
    fn from(source: std::io::Error) -> Self {
        AppError::Io { path: None, source }
    }
}
