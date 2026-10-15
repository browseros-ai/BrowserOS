//! On-disk layout under the BrowserOS data dir (`$BROWSEROS_DIR` › `~/.browseros-dev` when
//! `NODE_ENV=development` › `~/.browseros`): the shared SQLite file, tool output, ACP session
//! records, MCP-host data. The same dir the TS server uses, so either binary can run. Owned by P3.

use crate::config::EnvConfig;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct BrowserosPaths {
    pub browseros_dir: PathBuf,
    /// `<dir>/db/browseros.sqlite`, shared with the TS server (drizzle bookkeeping kept).
    pub db_file: PathBuf,
    /// `<dir>/tool-output` (0700).
    pub tool_output_dir: PathBuf,
    pub installation_file: PathBuf,
    pub mcp_manager_dir: PathBuf,
    pub acp_sessions_dir: PathBuf,
    pub acp_workspace_dir: PathBuf,
    pub mcp_host_dir: PathBuf,
    pub cache_dir: PathBuf,
}

impl BrowserosPaths {
    pub fn new(browseros_dir: PathBuf) -> Self {
        Self {
            db_file: browseros_dir.join("db").join("browseros.sqlite"),
            tool_output_dir: browseros_dir.join("tool-output"),
            installation_file: browseros_dir.join("installation.json"),
            mcp_manager_dir: browseros_dir.join("mcp-manager"),
            acp_sessions_dir: browseros_dir.join("agents").join("acp-sessions"),
            acp_workspace_dir: browseros_dir.join("agents").join("acp-workspace"),
            mcp_host_dir: browseros_dir.join("mcp-host"),
            cache_dir: browseros_dir.join("cache"),
            browseros_dir,
        }
    }
}

pub fn resolve_browseros_dir(env: &EnvConfig) -> PathBuf {
    if let Some(dir) = &env.browseros_dir {
        return dir.clone();
    }
    let home = env.home.clone().unwrap_or_default();
    if env.node_env.as_deref() == Some("development") {
        home.join(".browseros-dev")
    } else {
        home.join(".browseros")
    }
}
