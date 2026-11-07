//! The host side's persistence port. The trace store owns rows and retention; the host only
//! attaches after-call artifacts and fences completion. No database type crosses this seam.

use std::collections::{HashMap, HashSet};

use bytes::Bytes;
use futures_util::future::BoxFuture;
use tokio::sync::Mutex;

use crate::host::{SessionKey, TabId};
use crate::run::{PageId, RunId};

/// Facts read after a row ended, and optionally its bounded JPEG. A write without a screenshot
/// updates facts while preserving an earlier image; withholding is the only deletion operation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CallArtifacts {
    pub tab: TabId,
    pub url: String,
    pub title: String,
    pub screenshot: Option<Bytes>,
}

/// A persistence failure, represented without exposing the store's implementation.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct StoreError(pub String);

/// The only persistence operations the host requires. Implementations serialize accepted writes
/// with `withhold`: once withheld, later artifact writes must discard images too. `flushed` is a
/// barrier requested after the ledger has consumed all passes and settled all derived work.
pub type WithheldSessions = Vec<(SessionKey, Vec<PageId>)>;

/// Object-safe asynchronous port implemented by the trace store.
pub trait HostStore: Send + Sync {
    fn knows<'a>(&'a self, session: &'a SessionKey) -> BoxFuture<'a, Result<bool, StoreError>>;
    fn flushed(&self) -> BoxFuture<'_, Result<(), StoreError>>;
    fn put_artifacts<'a>(
        &'a self,
        run: &'a RunId,
        call_id: Option<u64>,
        artifacts: CallArtifacts,
    ) -> BoxFuture<'a, Result<(), StoreError>>;
    fn withhold<'a>(&'a self, session: &'a SessionKey) -> BoxFuture<'a, Result<(), StoreError>>;
    fn withheld(&self) -> BoxFuture<'_, Result<WithheldSessions, StoreError>>;
}

/// An in-memory test helper for host consumers. `record_run` stands in for the sibling trace
/// sink: artifact writes alone do not mean the store ever recorded a run of that session.
#[derive(Default)]
pub struct MemoryStore(Mutex<Memory>);

#[derive(Default)]
struct Memory {
    runs: HashMap<RunId, SessionKey>,
    targets: HashMap<SessionKey, HashSet<PageId>>,
    withheld: HashSet<SessionKey>,
    artifacts: HashMap<(RunId, Option<u64>), CallArtifacts>,
}

impl MemoryStore {
    pub async fn record_run(&self, session: &SessionKey, run: &RunId, pages: &[PageId]) {
        let mut memory = self.0.lock().await;
        memory.runs.insert(run.clone(), session.clone());
        memory
            .targets
            .entry(session.clone())
            .or_default()
            .extend(pages.iter().cloned());
    }

    pub async fn artifacts(&self, run: &RunId) -> HashMap<Option<u64>, CallArtifacts> {
        self.0
            .lock()
            .await
            .artifacts
            .iter()
            .filter(|((id, _), _)| id == run)
            .map(|((_, call), artifacts)| (*call, artifacts.clone()))
            .collect()
    }
}

impl HostStore for MemoryStore {
    fn knows<'a>(&'a self, session: &'a SessionKey) -> BoxFuture<'a, Result<bool, StoreError>> {
        Box::pin(async move { Ok(self.0.lock().await.runs.values().any(|key| key == session)) })
    }

    fn flushed(&self) -> BoxFuture<'_, Result<(), StoreError>> {
        Box::pin(async { Ok(()) })
    }

    fn put_artifacts<'a>(
        &'a self,
        run: &'a RunId,
        call_id: Option<u64>,
        mut artifacts: CallArtifacts,
    ) -> BoxFuture<'a, Result<(), StoreError>> {
        Box::pin(async move {
            let mut memory = self.0.lock().await;
            let withheld = memory
                .runs
                .get(run)
                .is_some_and(|key| memory.withheld.contains(key));
            let key = (run.clone(), call_id);
            if withheld {
                artifacts.screenshot = None;
            } else if artifacts.screenshot.is_none() {
                artifacts.screenshot = memory
                    .artifacts
                    .get(&key)
                    .and_then(|old| old.screenshot.clone());
            }
            memory.artifacts.insert(key, artifacts);
            Ok(())
        })
    }

    fn withhold<'a>(&'a self, session: &'a SessionKey) -> BoxFuture<'a, Result<(), StoreError>> {
        Box::pin(async move {
            let mut memory = self.0.lock().await;
            memory.withheld.insert(session.clone());
            let runs: HashSet<_> = memory
                .runs
                .iter()
                .filter(|(_, key)| *key == session)
                .map(|(run, _)| run.clone())
                .collect();
            for ((run, _), artifacts) in &mut memory.artifacts {
                if runs.contains(run) {
                    artifacts.screenshot = None;
                }
            }
            Ok(())
        })
    }

    fn withheld(&self) -> BoxFuture<'_, Result<WithheldSessions, StoreError>> {
        Box::pin(async move {
            let memory = self.0.lock().await;
            Ok(memory
                .withheld
                .iter()
                .map(|key| {
                    (
                        key.clone(),
                        memory
                            .targets
                            .get(key)
                            .map(|targets| targets.iter().cloned().collect())
                            .unwrap_or_default(),
                    )
                })
                .collect())
        })
    }
}

#[cfg(test)]
mod tests {
    use std::slice;

    use super::*;

    #[tokio::test]
    async fn withholding_deletes_images_and_rejects_later_images_but_keeps_facts() {
        let store = MemoryStore::default();
        let session = SessionKey::new("private");
        let run = RunId::mint();
        let target = PageId::new("target".to_string());
        assert!(!store.knows(&session).await.unwrap());
        store
            .record_run(&session, &run, slice::from_ref(&target))
            .await;
        let artifacts = CallArtifacts {
            tab: TabId::parse(1).unwrap(),
            url: "url".to_string(),
            title: "title".to_string(),
            screenshot: Some(Bytes::from_static(b"jpeg")),
        };
        store
            .put_artifacts(&run, Some(1), artifacts.clone())
            .await
            .unwrap();
        store.withhold(&session).await.unwrap();
        store.withhold(&session).await.unwrap();
        store.put_artifacts(&run, None, artifacts).await.unwrap();
        store.flushed().await.unwrap();
        assert!(store.knows(&session).await.unwrap());
        let rows = store.artifacts(&run).await;
        assert_eq!(rows.len(), 2);
        assert!(
            rows.values()
                .all(|row| row.screenshot.is_none() && row.url == "url")
        );
        assert_eq!(store.withheld().await.unwrap(), [(session, vec![target])]);
    }
}
