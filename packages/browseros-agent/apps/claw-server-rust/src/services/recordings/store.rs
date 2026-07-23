//! Transactional document-keyed rrweb persistence, independent of MCP attribution.

use crate::{
    clock::now_epoch_ms,
    db::{AppendDocumentBatch, RecordingIndex},
    error::{AppError, AppResult},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::HashMap, path::PathBuf, sync::Arc, time::Duration};
use tokio::{
    fs,
    sync::Mutex,
    task::JoinHandle,
    time::{MissedTickBehavior, interval},
};
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

const DAY_MS: i64 = 24 * 60 * 60 * 1000;
const RETENTION_INTERVAL: Duration = Duration::from_secs(60 * 60);
pub const RECORDING_ORPHAN_TTL_MS: i64 = 60 * 60 * 1000;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordingEventInput {
    /// Recorder-supplied rrweb event time in Unix-epoch milliseconds.
    pub ts: i64,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub event_type: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

pub type RecordedEvent = RecordingEventInput;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LegacyRecordedEvent {
    pub tab_id: i64,
    pub ts: i64,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub event_type: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetentionSweepResult {
    pub recordings_deleted: u64,
    pub claims_deleted: u64,
}

/// Stores each Chrome document stream and its durable batch acceptance ledger.
pub struct RecordingStore {
    root: PathBuf,
    index: Arc<RecordingIndex>,
    document_locks: Mutex<HashMap<String, Arc<Mutex<()>>>>,
}

impl RecordingStore {
    #[must_use]
    pub fn new(
        root: PathBuf,
        index: Arc<RecordingIndex>,
        _max_open_handles: usize,
        _idle_handle: Duration,
    ) -> Arc<Self> {
        Arc::new(Self {
            root,
            index,
            document_locks: Mutex::new(HashMap::new()),
        })
    }

    /// Returns false only when this document already durably accepted the batch.
    pub async fn append_batch(
        &self,
        document_id: &str,
        tab_id: i64,
        target_id: Option<&str>,
        events: &[RecordingEventInput],
        batch_id: &str,
        has_gap: bool,
    ) -> AppResult<bool> {
        let document_lock = self.lock_for(document_id).await;
        let guard = document_lock.lock().await;
        let result = self
            .append_locked(document_id, tab_id, target_id, events, batch_id, has_gap)
            .await;
        drop(guard);
        self.release_lock(document_id, &document_lock).await;
        result
    }

    async fn append_locked(
        &self,
        document_id: &str,
        tab_id: i64,
        target_id: Option<&str>,
        events: &[RecordingEventInput],
        batch_id: &str,
        has_gap: bool,
    ) -> AppResult<bool> {
        let mut payload = String::new();
        for event in events {
            payload.push_str(&serde_json::to_string(event)?);
            payload.push('\n');
        }
        let first_event_at = events
            .iter()
            .map(|event| event.ts)
            .min()
            .unwrap_or_default();
        let last_event_at = events
            .iter()
            .map(|event| event.ts)
            .max()
            .unwrap_or_default();
        let size_bytes = i64::try_from(payload.len()).unwrap_or(i64::MAX);
        let event_count = i64::try_from(events.len()).unwrap_or(i64::MAX);
        self.index
            .append_document_batch(AppendDocumentBatch {
                document_id,
                tab_id,
                target_id,
                payload,
                first_event_at,
                last_event_at,
                size_bytes,
                event_count,
                batch_id,
                has_gap,
            })
            .await
    }

    pub async fn read_range(
        &self,
        document_id: &str,
        from: i64,
        to: i64,
    ) -> AppResult<Vec<RecordedEvent>> {
        let Some(payload) = self.index.payload(document_id).await? else {
            return Ok(Vec::new());
        };
        Ok(read_payload_range(&payload, from, to))
    }

    pub async fn read_legacy_range(
        &self,
        target_id: &str,
        from: i64,
        to: i64,
    ) -> AppResult<Vec<LegacyRecordedEvent>> {
        read_file_range::<LegacyRecordedEvent>(self.path_for(target_id), from, to).await
    }

    pub async fn sweep_retention(
        &self,
        retention_days: u64,
        now: i64,
    ) -> AppResult<RetentionSweepResult> {
        let retention_ms = i64::try_from(retention_days)
            .unwrap_or(i64::MAX)
            .saturating_mul(DAY_MS);
        let retention_cutoff = now.saturating_sub(retention_ms);
        let orphan_cutoff = now.saturating_sub(RECORDING_ORPHAN_TTL_MS);
        let (claims, streams) = self.index.retention_snapshot().await?;
        let mut recordings_deleted = 0;
        for stream in streams {
            let claimed = claims.iter().any(|claim| {
                claim.tab_id == stream.tab_id
                    && stream.last_event_at >= claim.claimed_at
                    && stream.first_event_at <= claim.released_at.unwrap_or(i64::MAX)
            });
            let cutoff = if claimed {
                retention_cutoff
            } else {
                orphan_cutoff
            };
            if stream.last_event_at < cutoff && self.delete_document(&stream.document_id).await? {
                recordings_deleted += 1;
            }
        }

        let legacy = self
            .index
            .legacy_recordings_before(retention_cutoff)
            .await?;
        for recording in legacy {
            if self
                .delete_legacy_target(&recording.target_id, retention_cutoff)
                .await?
            {
                recordings_deleted += 1;
            }
        }

        Ok(RetentionSweepResult {
            recordings_deleted,
            claims_deleted: self
                .index
                .delete_released_claims_before(retention_cutoff)
                .await?,
        })
    }

    /// Runs recording retention immediately and then hourly.
    pub fn spawn_retention(
        self: Arc<Self>,
        retention_days: u64,
        cancel: CancellationToken,
    ) -> JoinHandle<()> {
        tokio::spawn(async move {
            let mut ticker = interval(RETENTION_INTERVAL);
            ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
            loop {
                tokio::select! {
                    () = cancel.cancelled() => return,
                    _ = ticker.tick() => {
                        match self.sweep_retention(retention_days, now_epoch_ms()).await {
                            Ok(result) => info!(
                                recordings_deleted = result.recordings_deleted,
                                claims_deleted = result.claims_deleted,
                                "recording retention sweep finished"
                            ),
                            Err(error) => warn!(error = %error, "recording retention sweep failed"),
                        }
                    }
                }
            }
        })
    }

    pub async fn close(&self) {}

    async fn delete_document(&self, document_id: &str) -> AppResult<bool> {
        let lock = self.lock_for(document_id).await;
        let guard = lock.lock().await;
        let result = async { self.index.delete_document(document_id).await }.await;
        drop(guard);
        self.release_lock(document_id, &lock).await;
        result
    }

    async fn delete_legacy_target(&self, target_id: &str, cutoff: i64) -> AppResult<bool> {
        let Some(recording) = self.index.legacy_recording(target_id).await? else {
            return Ok(false);
        };
        if recording.last_event_at >= cutoff
            || !remove_file(&self.path_for(target_id), target_id).await
        {
            return Ok(false);
        }
        self.index.delete_legacy_recording(target_id).await?;
        Ok(true)
    }

    async fn lock_for(&self, document_id: &str) -> Arc<Mutex<()>> {
        self.document_locks
            .lock()
            .await
            .entry(document_id.to_string())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    }

    async fn release_lock(&self, document_id: &str, lock: &Arc<Mutex<()>>) {
        let mut locks = self.document_locks.lock().await;
        if locks
            .get(document_id)
            .is_some_and(|stored| Arc::ptr_eq(stored, lock) && Arc::strong_count(stored) == 2)
        {
            locks.remove(document_id);
        }
    }

    fn path_for(&self, id: &str) -> PathBuf {
        self.root.join(format!("{}.ndjson", sanitize_id(id)))
    }
}

async fn read_file_range<T>(path: PathBuf, from: i64, to: i64) -> AppResult<Vec<T>>
where
    T: for<'de> Deserialize<'de> + Timestamped,
{
    let text = match fs::read_to_string(&path).await {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(source) => {
            return Err(AppError::Io {
                path: Some(path),
                source,
            });
        }
    };
    Ok(text
        .lines()
        .filter_map(|line| serde_json::from_str::<T>(line).ok())
        .filter(|event| event.timestamp() >= from && event.timestamp() <= to)
        .collect())
}

fn read_payload_range<T>(text: &str, from: i64, to: i64) -> Vec<T>
where
    T: for<'de> Deserialize<'de> + Timestamped,
{
    text.lines()
        .filter_map(|line| serde_json::from_str::<T>(line).ok())
        .filter(|event| event.timestamp() >= from && event.timestamp() <= to)
        .collect()
}

trait Timestamped {
    fn timestamp(&self) -> i64;
}

impl Timestamped for RecordedEvent {
    fn timestamp(&self) -> i64 {
        self.ts
    }
}

impl Timestamped for LegacyRecordedEvent {
    fn timestamp(&self) -> i64 {
        self.ts
    }
}

async fn remove_file(path: &PathBuf, id: &str) -> bool {
    match fs::remove_file(path).await {
        Ok(()) => true,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => true,
        Err(error) => {
            warn!(id, error = %error, "recording retention unlink failed");
            false
        }
    }
}

#[must_use]
pub fn legacy_document_id(target_id: &str) -> String {
    format!("legacy-{target_id}")
}

fn sanitize_id(id: &str) -> String {
    id.chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '.' || ch == '_' || ch == '-' {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{DATABASE_FILENAME, Database, RecordingIndex};
    use tempfile::tempdir;

    async fn setup() -> anyhow::Result<(tempfile::TempDir, Arc<RecordingIndex>, Arc<RecordingStore>)>
    {
        let dir = tempdir()?;
        let index = Arc::new(RecordingIndex::new(
            Database::open(dir.path().join(DATABASE_FILENAME)).await?,
        ));
        let store = RecordingStore::new(
            dir.path().join("recordings"),
            index.clone(),
            10,
            Duration::from_secs(1),
        );
        Ok((dir, index, store))
    }

    fn event(ts: i64) -> RecordingEventInput {
        RecordingEventInput {
            ts,
            event_type: Some(Value::from(3)),
            data: Some(serde_json::json!({ "ts": ts })),
        }
    }

    #[tokio::test]
    async fn document_catalog_and_durable_dedupe_survive_store_recreation() -> anyhow::Result<()> {
        let (_dir, index, store) = setup().await?;
        let document_id = "018f47a7-1c2b-7def-8123-0123456789ab";
        assert!(
            store
                .append_batch(
                    document_id,
                    11,
                    None,
                    &[event(200), event(100)],
                    "batch-a",
                    false
                )
                .await?
        );
        let recreated = RecordingStore::new(
            store.root.clone(),
            index.clone(),
            10,
            Duration::from_secs(1),
        );
        assert!(
            !recreated
                .append_batch(document_id, 11, None, &[event(100)], "batch-a", false)
                .await?
        );
        assert_eq!(index.stream_count().await?, 1);
        assert!(index.batch_exists(document_id, "batch-a").await?);
        assert_eq!(recreated.read_range(document_id, 100, 150).await?.len(), 1);
        Ok(())
    }

    #[tokio::test]
    async fn gap_is_sticky_and_target_can_resolve_after_first_batch() -> anyhow::Result<()> {
        let (_dir, index, store) = setup().await?;
        let document_id = "018f47a7-1c2b-7def-8123-0123456789ab";
        store
            .append_batch(document_id, 11, None, &[event(100)], "batch-a", true)
            .await?;
        store
            .append_batch(
                document_id,
                11,
                Some("target-a"),
                &[event(200)],
                "batch-b",
                false,
            )
            .await?;
        let Some(stream) = index.stream(document_id).await? else {
            anyhow::bail!("recording stream missing");
        };
        assert!(stream.has_gap);
        assert_eq!(stream.target_id.as_deref(), Some("target-a"));
        assert_eq!(stream.first_event_at, 100);
        assert_eq!(stream.last_event_at, 200);
        Ok(())
    }

    #[tokio::test]
    async fn document_identity_cannot_move_between_tabs() -> anyhow::Result<()> {
        let (_dir, _index, store) = setup().await?;
        let document_id = "018f47a7-1c2b-7def-8123-0123456789ab";
        store
            .append_batch(document_id, 11, None, &[event(100)], "batch-a", false)
            .await?;

        let error = match store
            .append_batch(document_id, 12, None, &[event(200)], "batch-b", false)
            .await
        {
            Ok(_) => anyhow::bail!("changed tab identity was accepted"),
            Err(error) => error,
        };
        assert!(error.to_string().contains("changed tab identity"));
        assert_eq!(
            store.read_range(document_id, 0, 300).await?,
            vec![event(100)]
        );
        Ok(())
    }

    #[tokio::test]
    async fn orphan_retention_keeps_claimed_streams_and_cascades_batches() -> anyhow::Result<()> {
        let (_dir, index, store) = setup().await?;
        let now = 10 * RECORDING_ORPHAN_TTL_MS;
        let claimed = "018f47a7-1c2b-7def-8123-0123456789ab";
        let orphan = "018f47a7-1c2b-7def-8123-0123456789ac";
        store
            .append_batch(
                claimed,
                11,
                None,
                &[event(now - 2 * RECORDING_ORPHAN_TTL_MS)],
                "claimed",
                false,
            )
            .await?;
        store
            .append_batch(
                orphan,
                22,
                None,
                &[event(now - 2 * RECORDING_ORPHAN_TTL_MS)],
                "orphan",
                false,
            )
            .await?;
        index
            .insert_session_tab("session-a", "agent-a", 11, None, 0, Some(now))
            .await?;

        assert_eq!(
            store.sweep_retention(7, now).await?,
            RetentionSweepResult {
                recordings_deleted: 1,
                claims_deleted: 0,
            }
        );
        assert!(index.stream(claimed).await?.is_some());
        assert!(index.stream(orphan).await?.is_none());
        assert!(!index.batch_exists(orphan, "orphan").await?);
        assert!(!index.payload_exists(orphan).await?);
        Ok(())
    }
}
