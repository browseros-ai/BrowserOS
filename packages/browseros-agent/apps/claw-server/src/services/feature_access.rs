//! Persisted installation grants, shared by HTTP, discovery and browser sessions.
//! The local operator API is the temporary writer. Paid access can replace that writer with
//! verified user entitlements while execution continues consuming immutable FeatureAccess.
use crate::{
    error::{AppError, AppResult},
    storage::JsonStore,
};
use browseros_core::FeatureFlags;
use tokio::sync::{Mutex, watch};

const FILE: &str = "feature-access.json";

/// Owns grant publication. A save and its watch notification are one serialized transition;
/// readers can never observe access that failed to persist.
pub struct FeatureAccessStore {
    files: JsonStore,
    live: watch::Sender<FeatureFlags>,
    writes: Mutex<()>,
}

impl FeatureAccessStore {
    pub async fn load(files: JsonStore) -> Self {
        let flags = match files.read_json::<FeatureFlags>(FILE).await {
            Ok(flags) => flags,
            Err(AppError::StorageNotFound(_)) => FeatureFlags::default(),
            Err(error) => {
                tracing::warn!(%error, "feature access unreadable; disabling Pro");
                FeatureFlags::default()
            }
        };
        Self {
            files,
            live: watch::channel(flags).0,
            writes: Mutex::new(()),
        }
    }

    #[must_use]
    pub fn snapshot(&self) -> FeatureFlags {
        *self.live.borrow()
    }

    #[must_use]
    pub fn subscribe(&self) -> watch::Receiver<FeatureFlags> {
        self.live.subscribe()
    }

    pub async fn update(&self, flags: FeatureFlags) -> AppResult<()> {
        let _write = self.writes.lock().await;
        self.files.write_json(FILE, &flags).await?;
        // A redundant PUT persists too, but does not invalidate every client's tool cache.
        self.live.send_if_modified(|current| {
            if *current == flags {
                return false;
            }
            *current = flags;
            true
        });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn failed_save_and_corrupt_file_fail_closed() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let files = JsonStore::new(dir.path().to_path_buf());
        tokio::fs::write(dir.path().join(FILE), "{broken").await?;
        let store = FeatureAccessStore::load(files).await;
        assert!(!store.snapshot().pro);
        let changes = store.subscribe();
        store.update(FeatureFlags { pro: true }).await?;
        assert!(changes.has_changed()?);
        assert!(store.snapshot().pro);
        tokio::fs::remove_file(dir.path().join(FILE)).await?;
        tokio::fs::create_dir(dir.path().join(FILE)).await?;
        let changes = store.subscribe();
        assert!(store.update(FeatureFlags { pro: false }).await.is_err());
        assert!(store.snapshot().pro);
        assert!(!changes.has_changed()?);
        Ok(())
    }

    #[tokio::test]
    async fn identical_writes_do_not_emit_changes() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let store = FeatureAccessStore::load(JsonStore::new(dir.path().to_path_buf())).await;
        let changes = store.subscribe();
        store.update(FeatureFlags::default()).await?;
        assert!(!changes.has_changed()?);
        Ok(())
    }
}
