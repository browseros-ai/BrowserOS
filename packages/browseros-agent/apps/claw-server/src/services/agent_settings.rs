//! User-controlled agent tool settings, persisted as a small JSON file in the
//! BrowserClaw dir. Mirrors the audit-retention store: atomic write,
//! corrupt/unreadable falls back to the default rather than failing. The flag
//! is also mirrored in an atomic because MCP `tools/list` and `initialize`
//! read it synchronously.

use crate::error::{AppError, AppResult};
use serde::{Deserialize, Serialize};
use std::{
    io::{self, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};
use tempfile::NamedTempFile;
use tokio::sync::Mutex;

const AGENT_SETTINGS_FILE: &str = "agent-settings.json";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentSettings {
    #[serde(default = "default_true")]
    pub human_help_enabled: bool,
}

impl Default for AgentSettings {
    fn default() -> Self {
        Self {
            human_help_enabled: true,
        }
    }
}

fn default_true() -> bool {
    true
}

pub struct AgentSettingsStore {
    path: PathBuf,
    human_help_enabled: AtomicBool,
    /// Serializes writes so the file and the atomic never disagree.
    write_lock: Mutex<()>,
}

impl AgentSettingsStore {
    pub async fn new(browserclaw_dir: impl AsRef<Path>) -> Self {
        let path = browserclaw_dir.as_ref().join(AGENT_SETTINGS_FILE);
        let settings = load_or_default(&path).await;
        Self {
            path,
            human_help_enabled: AtomicBool::new(settings.human_help_enabled),
            write_lock: Mutex::new(()),
        }
    }

    #[must_use]
    pub fn get(&self) -> AgentSettings {
        AgentSettings {
            human_help_enabled: self.human_help_enabled(),
        }
    }

    #[must_use]
    pub fn human_help_enabled(&self) -> bool {
        self.human_help_enabled.load(Ordering::Acquire)
    }

    pub async fn set(&self, settings: AgentSettings) -> AppResult<AgentSettings> {
        let _guard = self.write_lock.lock().await;
        persist(&self.path, settings)
            .await
            .map_err(|source| AppError::Io {
                path: Some(self.path.clone()),
                source,
            })?;
        self.human_help_enabled
            .store(settings.human_help_enabled, Ordering::Release);
        Ok(settings)
    }
}

async fn load_or_default(path: &Path) -> AgentSettings {
    match tokio::fs::read_to_string(path).await {
        Ok(raw) => serde_json::from_str(&raw).unwrap_or_else(|error| {
            tracing::warn!(path = %path.display(), %error, "agent settings corrupt; using default");
            AgentSettings::default()
        }),
        Err(error) if error.kind() == io::ErrorKind::NotFound => AgentSettings::default(),
        Err(error) => {
            tracing::warn!(%error, "agent settings unreadable; using default");
            AgentSettings::default()
        }
    }
}

async fn persist(path: &Path, settings: AgentSettings) -> io::Result<()> {
    let path = path.to_path_buf();
    tokio::task::spawn_blocking(move || persist_blocking(&path, settings))
        .await
        .map_err(io::Error::other)?
}

fn persist_blocking(path: &Path, settings: AgentSettings) -> io::Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)?;
    let mut tmp = NamedTempFile::new_in(parent)?;
    let mut raw = serde_json::to_string_pretty(&settings).map_err(io::Error::other)?;
    raw.push('\n');
    tmp.write_all(raw.as_bytes())?;
    tmp.flush()?;
    tmp.as_file().sync_all()?;
    tmp.persist(path).map(|_| ()).map_err(|error| error.error)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn missing_config_keeps_human_help_on() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let store = AgentSettingsStore::new(dir.path()).await;
        assert!(store.human_help_enabled());
        Ok(())
    }

    #[tokio::test]
    async fn set_persists_and_reloads() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let store = AgentSettingsStore::new(dir.path()).await;
        store
            .set(AgentSettings {
                human_help_enabled: false,
            })
            .await?;
        assert!(!store.human_help_enabled());
        assert!(
            !AgentSettingsStore::new(dir.path())
                .await
                .human_help_enabled()
        );
        Ok(())
    }

    #[tokio::test]
    async fn corrupt_config_falls_back_to_default_without_erroring() -> anyhow::Result<()> {
        let dir = tempdir()?;
        tokio::fs::write(dir.path().join(AGENT_SETTINGS_FILE), "{not json").await?;
        let store = AgentSettingsStore::new(dir.path()).await;
        assert_eq!(store.get(), AgentSettings::default());
        Ok(())
    }
}
