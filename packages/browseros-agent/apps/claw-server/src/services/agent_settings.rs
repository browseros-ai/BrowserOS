//! The user's agent-tool settings, persisted as `agent-settings.json` in the BrowserClaw dir.
//! Loaded once at startup: a missing, corrupt, or unreadable file keeps human help on and never
//! fails the start. The value is mirrored in an atomic because MCP `initialize`,
//! `server/discover`, and `tools/list` read it synchronously.

use crate::{
    error::{AppError, AppResult},
    storage::JsonStore,
};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::Mutex;

const AGENT_SETTINGS_FILE: &str = "agent-settings.json";

/// Whether agents are offered `request_human_help` and `await_human_help`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HumanHelp {
    On,
    Off,
}

impl HumanHelp {
    /// Maps the wire boolean (`humanHelpEnabled`) to the setting.
    #[must_use]
    pub fn from_enabled(enabled: bool) -> Self {
        if enabled { Self::On } else { Self::Off }
    }

    #[must_use]
    pub fn is_enabled(self) -> bool {
        self == Self::On
    }
}

/// The file's shape. A missing key keeps human help on, so `{}` changes nothing.
#[derive(Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct SettingsFile {
    human_help_enabled: bool,
}

impl Default for SettingsFile {
    fn default() -> Self {
        Self {
            human_help_enabled: true,
        }
    }
}

pub struct AgentSettingsStore {
    files: JsonStore,
    human_help_enabled: AtomicBool,
    /// Serializes saves, so the file and the live value never disagree.
    writes: Mutex<()>,
}

impl AgentSettingsStore {
    pub async fn load(files: JsonStore) -> Self {
        let human_help = match files.read_json::<SettingsFile>(AGENT_SETTINGS_FILE).await {
            Ok(file) => HumanHelp::from_enabled(file.human_help_enabled),
            Err(AppError::StorageNotFound(_)) => HumanHelp::On,
            Err(error) => {
                tracing::warn!(%error, "agent settings unreadable; keeping human help on");
                HumanHelp::On
            }
        };
        Self {
            files,
            human_help_enabled: AtomicBool::new(human_help.is_enabled()),
            writes: Mutex::new(()),
        }
    }

    #[must_use]
    pub fn human_help(&self) -> HumanHelp {
        HumanHelp::from_enabled(self.human_help_enabled.load(Ordering::Acquire))
    }

    /// Saves first and only then updates the live value, so a failed save changes nothing.
    pub async fn set_human_help(&self, human_help: HumanHelp) -> AppResult<()> {
        let _write = self.writes.lock().await;
        let file = SettingsFile {
            human_help_enabled: human_help.is_enabled(),
        };
        self.files.write_json(AGENT_SETTINGS_FILE, &file).await?;
        self.human_help_enabled
            .store(human_help.is_enabled(), Ordering::Release);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use tempfile::tempdir;

    async fn store_in(dir: &Path) -> AgentSettingsStore {
        AgentSettingsStore::load(JsonStore::new(dir.to_path_buf())).await
    }

    #[tokio::test]
    async fn tests_that_human_help_is_on_without_a_settings_file() -> anyhow::Result<()> {
        let dir = tempdir()?;
        assert_eq!(store_in(dir.path()).await.human_help(), HumanHelp::On);
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_the_human_help_setting_survives_a_restart() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let store = store_in(dir.path()).await;
        for human_help in [HumanHelp::Off, HumanHelp::On] {
            store.set_human_help(human_help).await?;
            assert_eq!(store.human_help(), human_help);
            assert_eq!(store_in(dir.path()).await.human_help(), human_help);
        }
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_a_corrupt_settings_file_keeps_human_help_on() -> anyhow::Result<()> {
        let dir = tempdir()?;
        tokio::fs::write(dir.path().join(AGENT_SETTINGS_FILE), "{not json").await?;
        assert_eq!(store_in(dir.path()).await.human_help(), HumanHelp::On);
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_an_empty_settings_object_keeps_human_help_on() -> anyhow::Result<()> {
        let dir = tempdir()?;
        tokio::fs::write(dir.path().join(AGENT_SETTINGS_FILE), "{}").await?;
        assert_eq!(store_in(dir.path()).await.human_help(), HumanHelp::On);
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_an_unreadable_settings_file_keeps_human_help_on() -> anyhow::Result<()> {
        let dir = tempdir()?;
        // A directory where the file belongs reads as an I/O error, not as JSON.
        tokio::fs::create_dir(dir.path().join(AGENT_SETTINGS_FILE)).await?;
        assert_eq!(store_in(dir.path()).await.human_help(), HumanHelp::On);
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_a_failed_save_leaves_human_help_unchanged() -> anyhow::Result<()> {
        let dir = tempdir()?;
        // A directory where the file belongs makes the final rename fail.
        tokio::fs::create_dir(dir.path().join(AGENT_SETTINGS_FILE)).await?;
        let store = store_in(dir.path()).await;
        assert!(store.set_human_help(HumanHelp::Off).await.is_err());
        assert_eq!(store.human_help(), HumanHelp::On);
        Ok(())
    }
}
