//! The user's decision-model credential, and whether the goal-driven mode is
//! switched on.
//!
//! Persisted beside the other settings stores, with two differences that the
//! others do not need. The file is created readable only by its owner, and the
//! credential is redacted everywhere it could be printed, so the one copy of
//! it lives in this file and in memory and nowhere else.
//!
//! The active flag is mirrored into an atomic because the MCP tool list is
//! built synchronously, and the mode is only advertised when it can actually
//! run.

use crate::error::{AppError, AppResult};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use std::{
    io::{self, Write},
    path::{Path, PathBuf},
};
use tempfile::NamedTempFile;
use tokio::sync::Mutex;

const JEV_SETTINGS_FILE: &str = "jev-mode.json";

/// A credential that never prints itself.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Credential(String);

impl Credential {
    #[must_use]
    pub fn new(raw: impl Into<String>) -> Self {
        Self(raw.into().trim().to_string())
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The value itself. Named so that every use site reads as a deliberate
    /// disclosure.
    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }

    /// The last four characters, for telling the user which credential is
    /// stored without showing it back to them.
    #[must_use]
    pub fn fingerprint(&self) -> String {
        let visible: String = self
            .0
            .chars()
            .rev()
            .take(4)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        if visible.is_empty() {
            String::new()
        } else {
            format!("...{visible}")
        }
    }
}

impl std::fmt::Debug for Credential {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.0.is_empty() {
            formatter.write_str("Credential(unset)")
        } else {
            formatter.write_str("Credential(redacted)")
        }
    }
}

/// What is persisted.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct JevSettings {
    pub credential: Credential,
    /// The model id the provider resolved our alias to when this credential was
    /// last checked. The alias moves, and the model's documented limitations are
    /// version specific, so the version that actually answered is worth keeping.
    #[serde(default)]
    pub model: String,
    /// Paused keeps the credential but stops advertising the mode, which is a
    /// different intent from removing it.
    pub paused: bool,
    pub budgets: Budgets,
}

/// Limits on a single run. Every one of them ends the run by handing back
/// rather than by failing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Budgets {
    pub max_steps: u32,
    pub max_seconds: u32,
}

impl Default for Budgets {
    fn default() -> Self {
        Self {
            max_steps: 24,
            max_seconds: 90,
        }
    }
}

impl JevSettings {
    /// Whether the mode can run: a credential is stored and it is not paused.
    #[must_use]
    pub fn is_active(&self) -> bool {
        !self.credential.is_empty() && !self.paused
    }
}

pub struct JevSettingsStore {
    path: PathBuf,
    state: Mutex<JevSettings>,
    /// Mirrors `is_active` for the synchronous tool-list path.
    active: AtomicBool,
    /// Announces a change in whether the mode is offered.
    ///
    /// The tool list changes when a credential is added, paused or forgotten,
    /// and the server already advertises that it notifies clients of tool list
    /// changes. Without this it never did, which made the advertisement false:
    /// a client that listed tools at connect would never discover the tool.
    visibility: tokio::sync::watch::Sender<bool>,
}

impl JevSettingsStore {
    pub async fn new(browserclaw_dir: impl AsRef<Path>) -> Self {
        let path = browserclaw_dir.as_ref().join(JEV_SETTINGS_FILE);
        let state = load_or_default(&path).await;
        let active = AtomicBool::new(state.is_active());
        let (visibility, _) = tokio::sync::watch::channel(state.is_active());
        Self {
            path,
            state: Mutex::new(state),
            visibility,
            active,
        }
    }

    /// Whether to advertise the mode. Synchronous on purpose: the tool list is
    /// built without awaiting.
    #[must_use]
    pub fn is_active(&self) -> bool {
        self.active.load(Ordering::Relaxed)
    }

    pub async fn get(&self) -> JevSettings {
        self.state.lock().await.clone()
    }

    /// The credential to decide with, or `None` when the mode is off.
    pub async fn credential(&self) -> Option<Credential> {
        let state = self.state.lock().await;
        state.is_active().then(|| state.credential.clone())
    }

    /// Stores a credential and the model id the provider resolved for it.
    ///
    /// The model id arrives from the check that validated the credential, so it
    /// describes the version that actually answered rather than the alias asked
    /// for.
    pub async fn set_credential(
        &self,
        credential: Credential,
        model: String,
    ) -> AppResult<JevSettings> {
        self.update(|state| {
            state.credential = credential;
            state.model = model;
            // Storing a credential is an opt in, so it also unpauses.
            state.paused = false;
        })
        .await
    }

    pub async fn set_paused(&self, paused: bool) -> AppResult<JevSettings> {
        self.update(|state| state.paused = paused).await
    }

    pub async fn set_budgets(&self, budgets: Budgets) -> AppResult<JevSettings> {
        self.update(|state| state.budgets = budgets).await
    }

    /// Forgets the credential entirely, which is a different intent from
    /// pausing.
    pub async fn clear(&self) -> AppResult<JevSettings> {
        self.update(|state| {
            state.credential = Credential::default();
            // The recorded version describes the key that was checked, so it
            // goes with the key rather than outliving it.
            state.model = String::new();
            state.paused = false;
        })
        .await
    }

    async fn update(&self, change: impl FnOnce(&mut JevSettings)) -> AppResult<JevSettings> {
        let mut state = self.state.lock().await;
        let mut next = state.clone();
        change(&mut next);
        persist(&self.path, &next)
            .await
            .map_err(|source| AppError::Io {
                path: Some(self.path.clone()),
                source,
            })?;
        *state = next.clone();
        let active = next.is_active();
        self.active.store(active, Ordering::Relaxed);
        // Only when it actually moved: a budget change does not alter the tool
        // list, and telling a client to re-list its tools for nothing is a
        // cost. `send_if_modified` notifies only when the closure reports a
        // change, which is the comparison done once and correctly rather than
        // by hand at the call site.
        self.visibility.send_if_modified(|current| {
            let moved = *current != active;
            *current = active;
            moved
        });
        Ok(next)
    }

    /// Watches whether the mode is offered, so a connection can tell its client
    /// when the tool list changes.
    #[must_use]
    pub fn visibility(&self) -> tokio::sync::watch::Receiver<bool> {
        self.visibility.subscribe()
    }
}

async fn load_or_default(path: &Path) -> JevSettings {
    match tokio::fs::read_to_string(path).await {
        Ok(raw) => serde_json::from_str(&raw).unwrap_or_else(|error| {
            // Deliberately does not log the path's contents.
            tracing::warn!(%error, "decision mode settings corrupt; treating the mode as off");
            JevSettings::default()
        }),
        Err(error) if error.kind() == io::ErrorKind::NotFound => JevSettings::default(),
        Err(error) => {
            tracing::warn!(%error, "decision mode settings unreadable; treating the mode as off");
            JevSettings::default()
        }
    }
}

async fn persist(path: &Path, settings: &JevSettings) -> io::Result<()> {
    let path = path.to_path_buf();
    let settings = settings.clone();
    tokio::task::spawn_blocking(move || persist_blocking(&path, &settings))
        .await
        .map_err(io::Error::other)?
}

fn persist_blocking(path: &Path, settings: &JevSettings) -> io::Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)?;
    let mut tmp = NamedTempFile::new_in(parent)?;
    // Narrow the permissions before the credential is written, not after, so
    // it is never briefly world readable.
    owner_only(tmp.as_file())?;
    let mut raw = serde_json::to_string_pretty(settings).map_err(io::Error::other)?;
    raw.push('\n');
    tmp.write_all(raw.as_bytes())?;
    tmp.flush()?;
    tmp.as_file().sync_all()?;
    tmp.persist(path).map(|_| ()).map_err(|error| error.error)
}

#[cfg(unix)]
fn owner_only(file: &std::fs::File) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    file.set_permissions(std::fs::Permissions::from_mode(0o600))
}

#[cfg(not(unix))]
fn owner_only(_file: &std::fs::File) -> io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn the_mode_is_off_until_a_credential_is_stored() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let store = JevSettingsStore::new(dir.path()).await;
        assert!(!store.is_active());
        assert_eq!(store.credential().await, None);
        Ok(())
    }

    #[tokio::test]
    async fn storing_a_credential_activates_and_survives_a_restart() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let store = JevSettingsStore::new(dir.path()).await;
        store
            .set_credential(Credential::new("secret-token"), "jev-1.13.0".to_string())
            .await?;
        assert!(store.is_active());
        assert_eq!(
            store.credential().await.map(|c| c.expose().to_string()),
            Some("secret-token".to_string())
        );

        let reopened = JevSettingsStore::new(dir.path()).await;
        assert!(reopened.is_active());
        Ok(())
    }

    /// Pausing and removing are different intents and must stay that way: a
    /// switch that silently discards a pasted credential is hostile.
    #[tokio::test]
    async fn pausing_keeps_the_credential_and_clearing_forgets_it() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let store = JevSettingsStore::new(dir.path()).await;
        store
            .set_credential(Credential::new("secret-token"), "jev-1.13.0".to_string())
            .await?;

        store.set_paused(true).await?;
        assert!(!store.is_active(), "a paused mode is not advertised");
        assert_eq!(store.credential().await, None);
        assert!(
            !store.get().await.credential.is_empty(),
            "pausing must not discard the credential"
        );

        store.set_paused(false).await?;
        assert!(store.is_active(), "unpausing uses the kept credential");

        store.clear().await?;
        assert!(store.get().await.credential.is_empty());
        assert!(!store.is_active());
        Ok(())
    }

    #[tokio::test]
    async fn storing_a_credential_also_unpauses() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let store = JevSettingsStore::new(dir.path()).await;
        store.set_paused(true).await?;
        store
            .set_credential(Credential::new("fresh"), "jev-1.13.0".to_string())
            .await?;
        assert!(store.is_active());
        Ok(())
    }

    /// The credential must not be reachable through any debug format, since
    /// settings end up inside structures that derive Debug.
    #[test]
    fn the_credential_is_redacted_everywhere_it_could_print() {
        let settings = JevSettings {
            model: "jev-1.13.0".to_string(),
            credential: Credential::new("super-secret-value"),
            paused: false,
            budgets: Budgets::default(),
        };
        let rendered = format!("{settings:?}");
        assert!(!rendered.contains("super-secret-value"), "{rendered}");
        assert!(rendered.contains("redacted"), "{rendered}");
        assert_eq!(format!("{:?}", Credential::default()), "Credential(unset)");
    }

    #[test]
    fn the_fingerprint_shows_only_the_tail() {
        assert_eq!(Credential::new("abcdefghij").fingerprint(), "...ghij");
        assert_eq!(Credential::new("").fingerprint(), "");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn the_settings_file_is_readable_only_by_its_owner() -> anyhow::Result<()> {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempdir()?;
        let store = JevSettingsStore::new(dir.path()).await;
        store
            .set_credential(Credential::new("secret-token"), "jev-1.13.0".to_string())
            .await?;
        let mode = tokio::fs::metadata(dir.path().join(JEV_SETTINGS_FILE))
            .await?
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600, "got {mode:o}");
        Ok(())
    }

    #[tokio::test]
    async fn corrupt_settings_leave_the_mode_off_rather_than_failing() -> anyhow::Result<()> {
        let dir = tempdir()?;
        tokio::fs::write(dir.path().join(JEV_SETTINGS_FILE), "{not json").await?;
        let store = JevSettingsStore::new(dir.path()).await;
        assert!(!store.is_active());
        Ok(())
    }

    /// The resolved model id is recorded when a credential is checked, and is
    /// absent before then. The published limitations of this model are version
    /// specific, so "which version answered" must not be guessed at.
    #[tokio::test]
    async fn the_resolved_model_is_recorded_with_the_credential() -> AppResult<()> {
        let dir = tempdir()?;
        let store = JevSettingsStore::new(dir.path()).await;
        assert_eq!(
            store.get().await.model,
            "",
            "nothing has been checked yet, so no version is claimed"
        );
        let settings = store
            .set_credential(Credential::new("secret-token"), "jev-1.13.0".to_string())
            .await?;
        assert_eq!(settings.model, "jev-1.13.0");

        // And it survives a restart, so the cockpit does not forget which
        // version the stored key was validated against.
        let reopened = JevSettingsStore::new(dir.path()).await;
        assert_eq!(reopened.get().await.model, "jev-1.13.0");
        Ok(())
    }

    /// Clearing the credential clears the version with it, because a recorded
    /// version with no key behind it is a claim about nothing.
    #[tokio::test]
    async fn clearing_the_credential_clears_the_recorded_model() -> AppResult<()> {
        let dir = tempdir()?;
        let store = JevSettingsStore::new(dir.path()).await;
        store
            .set_credential(Credential::new("secret-token"), "jev-1.13.0".to_string())
            .await?;
        store.clear().await?;
        assert_eq!(store.get().await.model, "");
        Ok(())
    }

    /// The tool list changes when the mode's visibility does, and this server
    /// advertises that it notifies clients of tool list changes. The signal is
    /// what makes that advertisement true.
    #[tokio::test]
    async fn the_visibility_change_is_announced() -> AppResult<()> {
        let dir = tempdir()?;
        let store = JevSettingsStore::new(dir.path()).await;
        let mut watch = store.visibility();
        assert!(!*watch.borrow_and_update(), "off until a key is stored");

        store
            .set_credential(Credential::new("secret-token"), "jev-1.13.0".to_string())
            .await?;
        assert!(watch.changed().await.is_ok(), "storing a key announces it");
        assert!(*watch.borrow_and_update());

        store.set_paused(true).await?;
        assert!(watch.changed().await.is_ok(), "pausing announces it too");
        assert!(!*watch.borrow_and_update());
        Ok(())
    }

    /// A change that leaves the tool list alone says nothing. A notification
    /// for a budget edit would be noise, and a client re-listing its tools for
    /// nothing is a cost.
    #[tokio::test]
    async fn a_change_that_does_not_move_visibility_is_silent() -> AppResult<()> {
        let dir = tempdir()?;
        let store = JevSettingsStore::new(dir.path()).await;
        store
            .set_credential(Credential::new("secret-token"), "jev-1.13.0".to_string())
            .await?;
        let mut watch = store.visibility();
        let _ = watch.borrow_and_update();

        store
            .set_budgets(Budgets {
                max_steps: 7,
                max_seconds: 11,
            })
            .await?;
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(50), watch.changed())
                .await
                .is_err(),
            "a budget edit does not change the tool list"
        );
        Ok(())
    }

    #[tokio::test]
    async fn budgets_default_and_persist() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let store = JevSettingsStore::new(dir.path()).await;
        assert_eq!(store.get().await.budgets, Budgets::default());
        store
            .set_budgets(Budgets {
                max_steps: 5,
                max_seconds: 10,
            })
            .await?;
        assert_eq!(
            JevSettingsStore::new(dir.path()).await.get().await.budgets,
            Budgets {
                max_steps: 5,
                max_seconds: 10
            }
        );
        Ok(())
    }
}
