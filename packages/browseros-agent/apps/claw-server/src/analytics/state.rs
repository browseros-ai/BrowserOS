//! Durable BrowserClaw analytics identity and consent. Every writer holds the same
//! process-shared lock so migration, concurrent first starts and consent updates
//! cannot replace the chosen UUID or lose an installation alias.

use super::installation::{read_installation_id, valid_identity};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
};
use tempfile::NamedTempFile;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TelemetryState {
    pub distinct_id: String,
    pub enabled: bool,
    pub consent: bool,
}

/// The durable source for Rust and cockpit identity, consent, and evidence of
/// old installation IDs that must stay linked even if Chromium changes its file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AnalyticsState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) distinct_id: Option<String>,
    #[serde(default = "default_enabled")]
    pub(crate) enabled: bool,
    // Keep observed aliases even if Chromium later removes/changes its file.
    // Delivery is idempotently retried on each consenting startup, so this is
    // evidence of a known pair, not a claim that PostHog accepted the merge.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) installation_aliases: Vec<String>,
}

fn default_enabled() -> bool {
    true
}

pub(crate) fn state_path(browserclaw_dir: &Path) -> PathBuf {
    browserclaw_dir.join("analytics.json")
}

pub(crate) async fn load_or_create_state(path: &Path) -> AnalyticsState {
    let path = path.to_owned();
    match tokio::task::spawn_blocking(move || load_or_create_blocking(&path)).await {
        Ok(Ok(state)) => state,
        result => {
            tracing::warn!(error = ?result, "analytics state unavailable; disabling without rotating identity");
            AnalyticsState {
                distinct_id: None,
                enabled: false,
                installation_aliases: vec![],
            }
        }
    }
}

fn load_or_create_blocking(path: &Path) -> io::Result<AnalyticsState> {
    let _lock = lock_state(path)?;
    let original = read_state(path)?;
    let mut state = original.clone().unwrap_or(AnalyticsState {
        distinct_id: None,
        enabled: true,
        installation_aliases: vec![],
    });
    let installation = match read_installation_id(parent(path)) {
        Ok(id) => id,
        // Chromium is optional once BrowserClaw already has its own identity.
        Err(error) if state.distinct_id.is_some() => {
            tracing::warn!(%error, "ignoring unavailable Chromium identity; keeping analytics identity");
            None
        }
        Err(error) => return Err(error),
    };
    let id = state.distinct_id.get_or_insert_with(|| {
        installation
            .clone()
            .unwrap_or_else(|| Uuid::new_v4().to_string())
    });
    if let Some(alias) = installation
        && alias != *id
        && !state.installation_aliases.contains(&alias)
    {
        state.installation_aliases.push(alias);
    }
    if original.as_ref() != Some(&state) {
        write_state(path, &state)?;
    }
    Ok(state)
}

fn parent(path: &Path) -> &Path {
    path.parent().unwrap_or_else(|| Path::new("."))
}

fn lock_state(path: &Path) -> io::Result<fs::File> {
    fs::create_dir_all(parent(path))?;
    // Lock a stable sibling inode: locking analytics.json itself would stop
    // protecting other writers as soon as an atomic rename replaced that file.
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path.with_extension("json.lock"))?;
    lock.lock()?;
    Ok(lock) // dropping the handle releases the OS lock, including on errors.
}

fn read_state(path: &Path) -> io::Result<Option<AnalyticsState>> {
    let raw = match fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let state: AnalyticsState = serde_json::from_str(&raw)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    if state
        .distinct_id
        .as_deref()
        .is_some_and(|id| !valid_identity(id))
        || state
            .installation_aliases
            .iter()
            .any(|id| !valid_identity(id))
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid analytics UUID",
        ));
    }
    Ok(Some(state))
}

pub(crate) async fn persist_state(path: &Path, state: &AnalyticsState) -> io::Result<()> {
    let path = path.to_owned();
    let state = state.clone();
    tokio::task::spawn_blocking(move || {
        let _lock = lock_state(&path)?;
        let mut next = read_state(&path)?.unwrap_or_else(|| state.clone());
        if state.distinct_id.is_none() || next.distinct_id != state.distinct_id {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "analytics identity unavailable or changed",
            ));
        }
        // Re-read under the lock: another process may have discovered an alias
        // since this service started. A consent change must retain that evidence.
        next.enabled = state.enabled;
        for alias in &state.installation_aliases {
            if !next.installation_aliases.contains(alias) {
                next.installation_aliases.push(alias.clone());
            }
        }
        write_state(&path, &next)
    })
    .await
    .map_err(io::Error::other)?
}

fn write_state(path: &Path, state: &AnalyticsState) -> io::Result<()> {
    let mut temporary = NamedTempFile::new_in(parent(path))?;
    serde_json::to_writer_pretty(&mut temporary, state).map_err(io::Error::other)?;
    temporary.write_all(b"\n")?;
    temporary.flush()?;
    temporary.as_file().sync_all()?;
    temporary
        .persist(path)
        .map(|_| ())
        .map_err(|error| error.error)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::tempdir;

    const A: &str = "2e087632-1f4e-4ee7-b8bb-cf8ad53e91a8";
    const B: &str = "31eca9ca-566d-4373-8a1c-2f29b32dbed1";

    #[tokio::test]
    async fn installation_seeds_missing_or_consent_only_analytics() -> anyhow::Result<()> {
        for existing in [None, Some(r#"{"enabled":false}"#)] {
            let directory = tempdir()?;
            let path = state_path(directory.path());
            if let Some(raw) = existing {
                fs::write(&path, raw)?;
            }
            fs::write(
                directory.path().join("installation.json"),
                json!({"install_id": B}).to_string(),
            )?;
            let state = load_or_create_state(&path).await;
            assert_eq!(state.distinct_id.as_deref(), Some(B));
            assert_eq!(state.enabled, existing.is_none());
            assert!(state.installation_aliases.is_empty());
            assert_eq!(read_state(&path)?, Some(state));
        }
        Ok(())
    }

    #[tokio::test]
    async fn migration_preserves_old_identity_and_consent_or_adopts_existing_installation()
    -> anyhow::Result<()> {
        for old in [None, Some(A)] {
            for installation in [None, Some(A), Some(B)] {
                for consent in [true, false] {
                    let directory = tempdir()?;
                    let path = state_path(directory.path());
                    fs::write(
                        &path,
                        json!({"distinctId": old, "enabled": consent}).to_string(),
                    )?;
                    let installation_path = directory.path().join("installation.json");
                    let original_installation =
                        installation.map(|id| json!({"install_id": id}).to_string());
                    if let Some(raw) = &original_installation {
                        fs::write(&installation_path, raw)?;
                    }
                    let state = load_or_create_state(&path).await;
                    let id = state
                        .distinct_id
                        .as_deref()
                        .ok_or_else(|| anyhow::anyhow!("missing persisted UUID"))?;
                    assert!(valid_identity(id));
                    if let Some(expected) = old.or(installation) {
                        assert_eq!(id, expected);
                    }
                    assert_eq!(state.enabled, consent);
                    assert_eq!(
                        state.installation_aliases,
                        installation
                            .filter(|alias| *alias != id)
                            .into_iter()
                            .collect::<Vec<_>>()
                    );
                    assert_eq!(load_or_create_state(&path).await, state);
                    assert_eq!(read_state(&path)?, Some(state));
                    assert_eq!(
                        fs::read_to_string(installation_path).ok(),
                        original_installation
                    );
                }
            }
        }
        Ok(())
    }

    #[tokio::test]
    async fn concurrent_first_starts_share_one_durable_uuid_without_creating_installation()
    -> anyhow::Result<()> {
        let directory = tempdir()?;
        let path = state_path(directory.path());
        let tasks: Vec<_> = (0..32)
            .map(|_| {
                let path = path.clone();
                tokio::spawn(async move { load_or_create_state(&path).await })
            })
            .collect();
        let mut states = Vec::new();
        for task in tasks {
            states.push(task.await?);
        }
        let first = &states[0];
        assert!(first.distinct_id.as_deref().is_some_and(valid_identity));
        assert!(first.enabled);
        assert!(states.iter().all(|state| state == first));
        assert_eq!(read_state(&path)?.as_ref(), Some(first));
        assert!(!directory.path().join("installation.json").exists());
        Ok(())
    }

    #[tokio::test]
    async fn consent_writes_preserve_aliases_discovered_by_another_process() -> anyhow::Result<()> {
        let directory = tempdir()?;
        let path = state_path(directory.path());
        fs::write(&path, json!({"distinctId": A, "enabled": true}).to_string())?;
        let mut stale = load_or_create_state(&path).await;
        let installation_path = directory.path().join("installation.json");
        fs::write(&installation_path, json!({"install_id": B}).to_string())?;
        let migrated = load_or_create_state(&path).await;
        assert_eq!(migrated.installation_aliases, [B]);
        fs::remove_file(installation_path)?;
        stale.enabled = false;
        persist_state(&path, &stale).await?;
        let restarted = load_or_create_state(&path).await;
        assert_eq!(restarted.distinct_id.as_deref(), Some(A));
        assert!(!restarted.enabled);
        assert_eq!(restarted.installation_aliases, [B]);
        Ok(())
    }

    #[tokio::test]
    async fn invalid_analytics_is_preserved_instead_of_replaced_by_installation()
    -> anyhow::Result<()> {
        for raw in [
            "{broken",
            r#"{"distinctId":""}"#,
            r#"{"distinctId":"not-a-uuid"}"#,
            r#"{"distinctId":"00000000-0000-0000-0000-000000000000"}"#,
        ] {
            let directory = tempdir()?;
            let path = state_path(directory.path());
            fs::write(&path, raw)?;
            fs::write(
                directory.path().join("installation.json"),
                json!({"install_id": B}).to_string(),
            )?;
            let state = load_or_create_state(&path).await;
            assert!(state.distinct_id.is_none());
            assert!(!state.enabled);
            assert_eq!(fs::read_to_string(path)?, raw);
        }
        Ok(())
    }

    #[tokio::test]
    async fn existing_analytics_identity_does_not_depend_on_readable_chromium_file()
    -> anyhow::Result<()> {
        let directory = tempdir()?;
        let path = state_path(directory.path());
        fs::write(&path, json!({"distinctId": A, "enabled": true}).to_string())?;
        fs::write(directory.path().join("installation.json"), "{broken")?;
        let state = load_or_create_state(&path).await;
        assert_eq!(state.distinct_id.as_deref(), Some(A));
        assert!(state.enabled);
        assert!(state.installation_aliases.is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn unwritable_state_never_exposes_an_ephemeral_uuid() -> anyhow::Result<()> {
        let directory = tempdir()?;
        let blocker = directory.path().join("not-a-directory");
        fs::write(&blocker, "block writes")?;
        let state = load_or_create_state(&blocker.join("analytics.json")).await;
        assert!(state.distinct_id.is_none());
        assert!(!state.enabled);
        Ok(())
    }
}
