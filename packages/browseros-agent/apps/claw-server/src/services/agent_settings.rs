//! The user's agent-tool settings, persisted as `agent-settings.json` in the BrowserClaw dir.
//! Loaded once at startup: a missing, corrupt, or unreadable file keeps human help on and never
//! fails the start. The value is mirrored in an atomic because MCP `initialize`,
//! `server/discover`, and `tools/list` read it synchronously.
//!
//! The decision model credential lives here too, in a file of its own. Two reasons it is not a
//! key in the settings file: the settings file is the kind of thing that ends up in a
//! diagnostics bundle, and a credential needs a mode that the ordinary writer does not set.
//! A saved token takes precedence over `JEV_TOKEN`, and that precedence is decided here so
//! that no other layer has to know there are two places to look.

use crate::{
    config::TokenSource,
    error::{AppError, AppResult},
    storage::JsonStore,
};
use serde::{Deserialize, Serialize};
use std::sync::{
    RwLock,
    atomic::{AtomicBool, Ordering},
};
use tokio::sync::Mutex;

const AGENT_SETTINGS_FILE: &str = "agent-settings.json";
const JEV_TOKEN_FILE: &str = "jev-token.json";
/// A paste target needs a bound, because a paste can be a whole file.
pub const MAX_JEV_TOKEN: usize = 4096;

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

/// The credential file's shape. One key, so the file explains itself to whoever finds it.
#[derive(Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct TokenFile {
    jev_token: String,
}

/// The file's shape. A missing key keeps human help on, so `{}` changes nothing.
#[derive(Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct SettingsFile {
    human_help_enabled: bool,
    task_cleanup_tip_enabled: bool,
}

impl Default for SettingsFile {
    fn default() -> Self {
        Self {
            human_help_enabled: true,
            task_cleanup_tip_enabled: false,
        }
    }
}

pub struct AgentSettingsStore {
    files: JsonStore,
    human_help_enabled: AtomicBool,
    task_cleanup_tip_enabled: AtomicBool,
    /// Whether a saved token exists, for the synchronous readers that only need
    /// to know whether to offer the tool.
    jev_token_saved: AtomicBool,
    /// The saved token itself, for the one path that builds a client with it.
    /// Never held across an await, never logged.
    jev_token: RwLock<Option<String>>,
    /// `JEV_TOKEN` as it was at startup, read once so precedence is decided in
    /// one place rather than wherever somebody reaches for the environment.
    jev_token_from_environment: Option<String>,
    /// Serializes saves, so the file and the live value never disagree.
    writes: Mutex<()>,
}

impl AgentSettingsStore {
    /// Loads with `JEV_TOKEN` as this process sees it.
    pub async fn load(files: JsonStore) -> Self {
        let from_environment = std::env::var("JEV_TOKEN").ok().and_then(|raw| usable(&raw));
        Self::load_with_environment(files, from_environment).await
    }

    /// The same, with the environment's token supplied.
    ///
    /// A test that asserted precedence by reading the real environment would pass or fail by
    /// whose shell ran it, which this codebase has been bitten by before.
    pub async fn load_with_environment(files: JsonStore, from_environment: Option<String>) -> Self {
        let settings = match files.read_json::<SettingsFile>(AGENT_SETTINGS_FILE).await {
            Ok(file) => file,
            Err(AppError::StorageNotFound(_)) => SettingsFile::default(),
            Err(error) => {
                tracing::warn!(%error, "agent settings unreadable; keeping defaults");
                SettingsFile::default()
            }
        };
        // A credential that cannot be read is the same as no credential: a server that will
        // not start is worse than a server without the loop.
        let saved = match files.read_json::<TokenFile>(JEV_TOKEN_FILE).await {
            Ok(file) => usable(&file.jev_token),
            Err(AppError::StorageNotFound(_)) => None,
            Err(error) => {
                tracing::warn!(%error, "saved decision model token unreadable; ignoring it");
                None
            }
        };
        Self {
            files,
            human_help_enabled: AtomicBool::new(settings.human_help_enabled),
            task_cleanup_tip_enabled: AtomicBool::new(settings.task_cleanup_tip_enabled),
            jev_token_saved: AtomicBool::new(saved.is_some()),
            jev_token: RwLock::new(saved),
            jev_token_from_environment: from_environment.as_deref().and_then(usable),
            writes: Mutex::new(()),
        }
    }

    /// Where the credential comes from. Cheap, and safe to call from a synchronous path.
    #[must_use]
    pub fn jev_token_source(&self) -> TokenSource {
        if self.jev_token_saved.load(Ordering::Acquire) {
            TokenSource::Settings
        } else if self.jev_token_from_environment.is_some() {
            TokenSource::Environment
        } else {
            TokenSource::None
        }
    }

    /// The credential itself, for the one place that builds a client with it.
    ///
    /// A saved token wins over the environment. Never logged.
    #[must_use]
    pub fn jev_credential(&self) -> Option<String> {
        if let Ok(saved) = self.jev_token.read()
            && let Some(token) = saved.as_ref()
        {
            return Some(token.clone());
        }
        self.jev_token_from_environment.clone()
    }

    /// Saves first and only then updates the live value, so a failed save changes nothing.
    pub async fn set_jev_token(&self, token: &str) -> AppResult<()> {
        let _write = self.writes.lock().await;
        // Guarded here as well as at the edge, because a blank token stored would read as
        // present and offer a tool that cannot work.
        let token = usable(token).ok_or_else(|| AppError::Http {
            status: axum::http::StatusCode::BAD_REQUEST,
            message: format!("token must be between 1 and {MAX_JEV_TOKEN} characters"),
        })?;
        let file = TokenFile {
            jev_token: token.clone(),
        };
        self.files.write_json_private(JEV_TOKEN_FILE, &file).await?;
        if let Ok(mut live) = self.jev_token.write() {
            *live = Some(token);
        }
        self.jev_token_saved.store(true, Ordering::Release);
        Ok(())
    }

    /// Forgets the saved token. The environment, if any, becomes the credential again.
    pub async fn clear_jev_token(&self) -> AppResult<()> {
        let _write = self.writes.lock().await;
        self.files.remove_file(JEV_TOKEN_FILE).await?;
        if let Ok(mut live) = self.jev_token.write() {
            *live = None;
        }
        self.jev_token_saved.store(false, Ordering::Release);
        Ok(())
    }

    #[must_use]
    pub fn human_help(&self) -> HumanHelp {
        HumanHelp::from_enabled(self.human_help_enabled.load(Ordering::Acquire))
    }

    #[must_use]
    pub fn task_cleanup_tip_enabled(&self) -> bool {
        self.task_cleanup_tip_enabled.load(Ordering::Acquire)
    }

    /// Saves first and only then updates the live value, so a failed save changes nothing.
    pub async fn set_human_help(&self, human_help: HumanHelp) -> AppResult<()> {
        self.update(Some(human_help), None).await
    }

    pub async fn update(
        &self,
        human_help: Option<HumanHelp>,
        task_cleanup_tip_enabled: Option<bool>,
    ) -> AppResult<()> {
        let _write = self.writes.lock().await;
        let human_help = human_help.unwrap_or_else(|| self.human_help());
        let file = SettingsFile {
            human_help_enabled: human_help.is_enabled(),
            task_cleanup_tip_enabled: task_cleanup_tip_enabled
                .unwrap_or_else(|| self.task_cleanup_tip_enabled()),
        };
        self.files.write_json(AGENT_SETTINGS_FILE, &file).await?;
        self.human_help_enabled
            .store(human_help.is_enabled(), Ordering::Release);
        self.task_cleanup_tip_enabled
            .store(file.task_cleanup_tip_enabled, Ordering::Release);
        Ok(())
    }
}

/// A token worth storing: not blank, and not a pasted file.
fn usable(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.len() > MAX_JEV_TOKEN {
        return None;
    }
    Some(trimmed.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use tempfile::tempdir;

    async fn store_in(dir: &Path) -> AgentSettingsStore {
        with_environment(dir, None).await
    }

    async fn with_environment(dir: &Path, token: Option<&str>) -> AgentSettingsStore {
        AgentSettingsStore::load_with_environment(
            JsonStore::new(dir.to_path_buf()),
            token.map(str::to_string),
        )
        .await
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

    #[tokio::test]
    async fn tests_that_there_is_no_credential_without_a_token_anywhere() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let store = store_in(dir.path()).await;
        assert_eq!(store.jev_token_source(), TokenSource::None);
        assert!(store.jev_credential().is_none());
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_the_environment_is_used_when_nothing_is_saved() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let store = with_environment(dir.path(), Some("from-the-environment")).await;
        assert_eq!(store.jev_token_source(), TokenSource::Environment);
        assert_eq!(
            store.jev_credential().as_deref(),
            Some("from-the-environment")
        );
        Ok(())
    }

    /// The precedence this feature exists for.
    #[tokio::test]
    async fn tests_that_a_saved_token_beats_the_environment() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let store = with_environment(dir.path(), Some("from-the-environment")).await;
        store.set_jev_token("from-the-settings").await?;
        assert_eq!(store.jev_token_source(), TokenSource::Settings);
        assert_eq!(store.jev_credential().as_deref(), Some("from-the-settings"));
        // And across a restart, because the file is what was written.
        let again = with_environment(dir.path(), Some("from-the-environment")).await;
        assert_eq!(again.jev_token_source(), TokenSource::Settings);
        assert_eq!(again.jev_credential().as_deref(), Some("from-the-settings"));
        Ok(())
    }

    /// Clearing falls back rather than going dark, which is the whole reason the
    /// source has three states instead of two.
    #[tokio::test]
    async fn tests_that_clearing_a_saved_token_falls_back_to_the_environment() -> anyhow::Result<()>
    {
        let dir = tempdir()?;
        let store = with_environment(dir.path(), Some("from-the-environment")).await;
        store.set_jev_token("from-the-settings").await?;
        store.clear_jev_token().await?;
        assert_eq!(store.jev_token_source(), TokenSource::Environment);
        assert_eq!(
            store.jev_credential().as_deref(),
            Some("from-the-environment")
        );
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_clearing_with_no_environment_leaves_no_credential() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let store = store_in(dir.path()).await;
        store.set_jev_token("from-the-settings").await?;
        store.clear_jev_token().await?;
        assert_eq!(store.jev_token_source(), TokenSource::None);
        assert!(store.jev_credential().is_none());
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_a_blank_token_is_refused_rather_than_stored() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let store = store_in(dir.path()).await;
        for blank in ["", "   ", "\n\t"] {
            assert!(store.set_jev_token(blank).await.is_err());
        }
        assert_eq!(store.jev_token_source(), TokenSource::None);
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_a_pasted_file_is_refused_rather_than_stored() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let store = store_in(dir.path()).await;
        assert!(
            store
                .set_jev_token(&"x".repeat(MAX_JEV_TOKEN + 1))
                .await
                .is_err()
        );
        assert_eq!(store.jev_token_source(), TokenSource::None);
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_a_token_is_stored_trimmed() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let store = store_in(dir.path()).await;
        store.set_jev_token("  padded  ").await?;
        assert_eq!(store.jev_credential().as_deref(), Some("padded"));
        Ok(())
    }

    /// A credential that cannot be read is the same as no credential: a server
    /// that will not start is worse than a server without the loop.
    #[tokio::test]
    async fn tests_that_a_corrupt_token_file_leaves_no_saved_token() -> anyhow::Result<()> {
        let dir = tempdir()?;
        tokio::fs::write(dir.path().join(JEV_TOKEN_FILE), "{not json").await?;
        assert_eq!(
            store_in(dir.path()).await.jev_token_source(),
            TokenSource::None
        );
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_an_unreadable_token_file_leaves_no_saved_token() -> anyhow::Result<()> {
        let dir = tempdir()?;
        tokio::fs::create_dir(dir.path().join(JEV_TOKEN_FILE)).await?;
        assert_eq!(
            store_in(dir.path()).await.jev_token_source(),
            TokenSource::None
        );
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_an_empty_token_object_leaves_no_saved_token() -> anyhow::Result<()> {
        let dir = tempdir()?;
        tokio::fs::write(dir.path().join(JEV_TOKEN_FILE), "{}").await?;
        assert_eq!(
            store_in(dir.path()).await.jev_token_source(),
            TokenSource::None
        );
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_a_failed_token_save_leaves_the_previous_one_live() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let store = store_in(dir.path()).await;
        store.set_jev_token("the-first-one").await?;
        // A directory where the file belongs makes the final rename fail, which
        // is how the existing settings tests reach this path too.
        tokio::fs::remove_file(dir.path().join(JEV_TOKEN_FILE)).await?;
        tokio::fs::create_dir(dir.path().join(JEV_TOKEN_FILE)).await?;
        assert!(store.set_jev_token("the-second-one").await.is_err());
        assert_eq!(store.jev_credential().as_deref(), Some("the-first-one"));
        Ok(())
    }

    /// The credential sits beside readable configuration, so its mode is the
    /// only thing keeping it apart from it.
    #[cfg(unix)]
    #[tokio::test]
    async fn tests_that_the_token_file_is_readable_only_by_its_owner() -> anyhow::Result<()> {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempdir()?;
        let store = store_in(dir.path()).await;
        store.set_jev_token("a-token").await?;
        let mode = tokio::fs::metadata(dir.path().join(JEV_TOKEN_FILE))
            .await?
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600, "got {mode:o}");
        Ok(())
    }

    /// The settings file travels in diagnostics bundles; the credential must not
    /// ride along in it.
    #[tokio::test]
    async fn tests_that_the_token_is_not_written_into_the_settings_file() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let store = store_in(dir.path()).await;
        store.set_jev_token("a-very-distinctive-token").await?;
        store.set_human_help(HumanHelp::Off).await?;
        let settings = tokio::fs::read_to_string(dir.path().join(AGENT_SETTINGS_FILE)).await?;
        assert!(!settings.contains("a-very-distinctive-token"));
        Ok(())
    }
}
