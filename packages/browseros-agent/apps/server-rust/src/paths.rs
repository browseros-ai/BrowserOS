//! On-disk layout under the BrowserOS data dir (`$BROWSEROS_DIR` › `~/.browseros-dev` when
//! `NODE_ENV=development` › `~/.browseros`): the shared SQLite file, tool output, ACP session
//! records, MCP-host data. The same dir the TS server uses, so either binary can run.

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
    if let Some(dir) = env
        .browseros_dir
        .as_ref()
        .filter(|dir| !dir.to_string_lossy().trim().is_empty())
    {
        return PathBuf::from(dir.to_string_lossy().trim());
    }
    // os.homedir() also falls back to the platform account database when HOME
    // is absent; std's platform lookup preserves that launch-environment case.
    #[allow(deprecated)]
    let home = env
        .home
        .clone()
        .or_else(std::env::home_dir)
        .unwrap_or_default();
    if env.node_env.as_deref() == Some("development") {
        home.join(".browseros-dev")
    } else {
        home.join(".browseros")
    }
}

/// Initialize only owned state. Legacy session retention never traverses ACP
/// records or follows a session symlink into an unrelated owner directory.
impl BrowserosPaths {
    pub async fn prepare(&self) -> std::io::Result<()> {
        tokio::fs::create_dir_all(&self.browseros_dir).await?;
        let sessions = self.browseros_dir.join("sessions");
        tokio::fs::create_dir_all(&sessions).await?;
        let mut builder = tokio::fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        builder.mode(0o700);
        builder.create(&self.tool_output_dir).await?;
        let metadata = tokio::fs::symlink_metadata(&self.tool_output_dir).await?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err(std::io::Error::other(
                "BrowserOS tool output directory must be a real directory.",
            ));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            tokio::fs::set_permissions(
                &self.tool_output_dir,
                std::fs::Permissions::from_mode(0o700),
            )
            .await?;
        }
        // A configured product root can contain a user-owned sessions symlink.
        // Pruning below that link would claim files outside the legacy store.
        if tokio::fs::symlink_metadata(&sessions)
            .await?
            .file_type()
            .is_symlink()
        {
            return Ok(());
        }
        let cutoff = std::time::SystemTime::now()
            .checked_sub(std::time::Duration::from_secs(30 * 24 * 60 * 60));
        if let Ok(mut entries) = tokio::fs::read_dir(&sessions).await {
            while let Ok(Some(entry)) = entries.next_entry().await {
                if let Ok(metadata) = tokio::fs::symlink_metadata(entry.path()).await
                    && metadata.is_dir()
                    && !metadata.file_type().is_symlink()
                    && let (Some(cutoff), Ok(modified)) = (cutoff, metadata.modified())
                    && modified < cutoff
                {
                    let _ = tokio::fs::remove_dir_all(entry.path()).await;
                }
            }
        }
        Ok(())
    }
}
