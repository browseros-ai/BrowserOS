//! User-controlled audit retention policy, persisted as a small JSON file in
//! the BrowserClaw dir and read live by the retention sweep. Mirrors the
//! analytics-consent store: atomic write, corrupt/unreadable falls back to the
//! default rather than failing.

use crate::error::{AppError, AppResult};
use serde::{Deserialize, Serialize};
use std::{
    io::{self, Write},
    path::{Path, PathBuf},
};
use tempfile::NamedTempFile;
use tokio::sync::Mutex;

const AUDIT_RETENTION_FILE: &str = "audit-retention.json";
const DEFAULT_RETENTION_DAYS: u32 = 7;

/// The persisted policy. `KeepForever` never deletes on age; `DeleteAfterDays`
/// removes audit data older than `days`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "mode")]
pub enum AuditRetention {
    KeepForever,
    DeleteAfterDays { days: u32 },
}

impl Default for AuditRetention {
    fn default() -> Self {
        Self::DeleteAfterDays {
            days: DEFAULT_RETENTION_DAYS,
        }
    }
}

impl AuditRetention {
    /// The retention window in days, or `None` for keep-forever.
    #[must_use]
    pub fn days(self) -> Option<u32> {
        match self {
            Self::KeepForever => None,
            Self::DeleteAfterDays { days } => Some(days),
        }
    }
}

pub struct AuditSettingsStore {
    path: PathBuf,
    state: Mutex<AuditRetention>,
}

impl AuditSettingsStore {
    pub async fn new(browserclaw_dir: impl AsRef<Path>) -> Self {
        let path = browserclaw_dir.as_ref().join(AUDIT_RETENTION_FILE);
        let state = load_or_default(&path).await;
        Self {
            path,
            state: Mutex::new(state),
        }
    }

    pub async fn get(&self) -> AuditRetention {
        *self.state.lock().await
    }

    pub async fn set(&self, policy: AuditRetention) -> AppResult<AuditRetention> {
        persist(&self.path, policy)
            .await
            .map_err(|source| AppError::Io {
                path: Some(self.path.clone()),
                source,
            })?;
        *self.state.lock().await = policy;
        Ok(policy)
    }
}

async fn load_or_default(path: &Path) -> AuditRetention {
    match tokio::fs::read_to_string(path).await {
        Ok(raw) => serde_json::from_str(&raw).unwrap_or_else(|error| {
            tracing::warn!(path = %path.display(), %error, "audit retention config corrupt; using default");
            AuditRetention::default()
        }),
        Err(error) if error.kind() == io::ErrorKind::NotFound => AuditRetention::default(),
        Err(error) => {
            tracing::warn!(%error, "audit retention config unreadable; using default");
            AuditRetention::default()
        }
    }
}

async fn persist(path: &Path, policy: AuditRetention) -> io::Result<()> {
    let path = path.to_path_buf();
    tokio::task::spawn_blocking(move || persist_blocking(&path, policy))
        .await
        .map_err(io::Error::other)?
}

fn persist_blocking(path: &Path, policy: AuditRetention) -> io::Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)?;
    let mut tmp = NamedTempFile::new_in(parent)?;
    let mut raw = serde_json::to_string_pretty(&policy).map_err(io::Error::other)?;
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
    async fn missing_config_defaults_to_seven_days() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let store = AuditSettingsStore::new(dir.path()).await;
        assert_eq!(
            store.get().await,
            AuditRetention::DeleteAfterDays { days: 7 }
        );
        Ok(())
    }

    #[tokio::test]
    async fn set_persists_and_reloads() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let store = AuditSettingsStore::new(dir.path()).await;
        store.set(AuditRetention::KeepForever).await?;
        assert_eq!(store.get().await, AuditRetention::KeepForever);

        let reopened = AuditSettingsStore::new(dir.path()).await;
        assert_eq!(reopened.get().await, AuditRetention::KeepForever);

        store
            .set(AuditRetention::DeleteAfterDays { days: 30 })
            .await?;
        assert_eq!(
            AuditSettingsStore::new(dir.path()).await.get().await,
            AuditRetention::DeleteAfterDays { days: 30 }
        );
        Ok(())
    }

    #[tokio::test]
    async fn corrupt_config_falls_back_to_default_without_erroring() -> anyhow::Result<()> {
        let dir = tempdir()?;
        tokio::fs::write(dir.path().join(AUDIT_RETENTION_FILE), "{not json").await?;
        let store = AuditSettingsStore::new(dir.path()).await;
        assert_eq!(store.get().await, AuditRetention::default());
        Ok(())
    }

    #[test]
    fn days_accessor_maps_variants() {
        assert_eq!(AuditRetention::KeepForever.days(), None);
        assert_eq!(
            AuditRetention::DeleteAfterDays { days: 15 }.days(),
            Some(15)
        );
    }
}
