//! `/mcp-manager/*`: installs/uninstalls the BrowserOS MCP server into third-party agents'
//! configs (curated set: claude-code, codex, cursor, opencode, antigravity, vscode, zed) through
//! `harness-integrations`, with the TS `humaniseInstallError` texts, plus the boot self-heal.
//! Implemented by P12.

use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentStatus {
    pub id: String,
    pub name: String,
    pub detected: bool,
    pub installed: bool,
}

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct McpManagerError(pub String);

#[derive(Debug, Clone)]
pub struct McpManagerService {
    manifest_dir: PathBuf,
    public_url: Option<String>,
}

const STUB: &str = "MCP manager is not implemented yet in server-rust";

impl McpManagerService {
    pub fn new(manifest_dir: PathBuf, public_url: Option<String>) -> Self {
        Self {
            manifest_dir,
            public_url,
        }
    }

    pub async fn agents(&self) -> Result<Vec<AgentStatus>, McpManagerError> {
        let _ = (&self.manifest_dir, &self.public_url);
        Err(McpManagerError(STUB.to_owned()))
    }

    pub async fn install(&self, agent_id: &str) -> Result<(), McpManagerError> {
        let _ = agent_id;
        Err(McpManagerError(STUB.to_owned()))
    }

    pub async fn uninstall(&self, agent_id: &str) -> Result<(), McpManagerError> {
        let _ = agent_id;
        Err(McpManagerError(STUB.to_owned()))
    }

    /// Re-points stale installs at the current server URL, in the background at boot.
    pub async fn self_heal(&self) {}
}
