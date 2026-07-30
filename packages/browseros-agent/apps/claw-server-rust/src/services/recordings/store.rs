//! Transactional document-keyed rrweb persistence, independent of MCP attribution.

use crate::{
    clock::now_epoch_ms,
    db::{AppendDocumentBatch, RecordingIndex},
    error::{AppError, AppResult, IoPath},
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{HashMap, HashSet},
    io::SeekFrom,
    path::PathBuf,
    sync::Arc,
    time::Duration,
};
use tokio::{
    fs::{self, OpenOptions},
    io::{AsyncSeekExt, AsyncWriteExt},
    sync::Mutex,
    task::JoinHandle,
    time::{MissedTickBehavior, interval},
};
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

const DAY_MS: i64 = 24 * 60 * 60 * 1000;
const RETENTION_INTERVAL: Duration = Duration::from_secs(60 * 60);
pub const RECORDING_ORPHAN_TTL_MS: i64 = 60 * 60 * 1000;
/// Grace window before a replay file with no stream row is treated as orphaned,
/// so a fresh append whose stream commit is still in flight is never swept.
const ORPHAN_FILE_GRACE_MS: i64 = 5 * 60 * 1000;
/// Per-run scan cap so a pathological `replays/` directory cannot hang the sweep.
const MAX_ORPHAN_FILE_SCAN: usize = 100_000;

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
    pub orphan_files_deleted: u64,
    pub claims_deleted: u64,
}

/// One step of migrating a document's rrweb payload from the SQLite blob to its
/// on-disk replay file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MigrationStep {
    /// A document's blob was extracted to its file (with its byte length).
    Migrated(i64),
    /// The disk was full; the oldest recording was dropped to recover space.
    Recovered,
    /// No un-migrated documents remain.
    Done,
}

/// Stores each Chrome document stream (metadata in SQLite, rrweb payload on disk
/// under `replays_root`) and its durable batch acceptance ledger.
pub struct RecordingStore {
    root: PathBuf,
    replays_root: PathBuf,
    index: Arc<RecordingIndex>,
    document_locks: Mutex<HashMap<String, Arc<Mutex<()>>>>,
}

impl RecordingStore {
    #[must_use]
    pub fn new(
        root: PathBuf,
        replays_root: PathBuf,
        index: Arc<RecordingIndex>,
        _max_open_handles: usize,
        _idle_handle: Duration,
    ) -> Arc<Self> {
        Arc::new(Self {
            root,
            replays_root,
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
        if self.index.batch_accepted(document_id, batch_id).await? {
            return Ok(false);
        }
        if events.is_empty() {
            return Ok(true);
        }
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
        // Append to the on-disk replay file, then record the batch. The
        // per-document lock serializes this; a crash between the two truncates
        // back to the committed length on the next append (see append_payload_file).
        // A legacy document may still hold its whole payload in the DB blob with
        // payload_bytes == 0; extract it to the file first so this batch appends
        // after those events, otherwise the background migration would later
        // truncate the file back to the blob and lose this batch.
        let committed = match self.index.committed_payload_bytes(document_id).await? {
            0 => self.extract_blob_locked(document_id).await?,
            committed => committed,
        };
        let payload_bytes = self
            .append_payload_file(document_id, committed, &payload)
            .await?;
        self.index
            .commit_batch(AppendDocumentBatch {
                document_id,
                tab_id,
                target_id,
                first_event_at,
                last_event_at,
                size_bytes,
                event_count,
                batch_id,
                has_gap,
                payload_bytes,
            })
            .await?;
        Ok(true)
    }

    /// Appends the batch payload to the document's `replays/` file. Truncates to
    /// `committed` first (dropping bytes an interrupted prior append never
    /// committed) and clamps to the file's actual length (an externally
    /// deleted/truncated file never zero-pads). Returns the new committed length.
    async fn append_payload_file(
        &self,
        document_id: &str,
        committed: i64,
        payload: &str,
    ) -> AppResult<i64> {
        let path = self.replay_path_for(document_id);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).await.with_path(parent)?;
        }
        let mut file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(&path)
            .await
            .with_path(&path)?;
        let current = file.metadata().await.with_path(&path)?.len();
        let base = committed.clamp(0, i64::try_from(current).unwrap_or(i64::MAX));
        let base_u64 = u64::try_from(base).unwrap_or(0);
        file.set_len(base_u64).await.with_path(&path)?;
        file.seek(SeekFrom::Start(base_u64))
            .await
            .with_path(&path)?;
        file.write_all(payload.as_bytes()).await.with_path(&path)?;
        file.sync_all().await.with_path(&path)?;
        Ok(base.saturating_add(i64::try_from(payload.len()).unwrap_or(i64::MAX)))
    }

    /// Loads a document's committed NDJSON payload, file-first, falling back to
    /// the DB blob for a document that predates the file migration.
    async fn load_payload(&self, document_id: &str) -> AppResult<Option<String>> {
        let committed = self.index.committed_payload_bytes(document_id).await?;
        if committed > 0 {
            let path = self.replay_path_for(document_id);
            let bytes = match fs::read(&path).await {
                Ok(bytes) => bytes,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
                Err(source) => {
                    return Err(AppError::Io {
                        path: Some(path),
                        source,
                    });
                }
            };
            let limit = usize::try_from(committed)
                .unwrap_or(usize::MAX)
                .min(bytes.len());
            return Ok(Some(String::from_utf8_lossy(&bytes[..limit]).into_owned()));
        }
        self.index.payload(document_id).await
    }

    fn replay_path_for(&self, document_id: &str) -> PathBuf {
        let key = replay_key(document_id);
        self.replays_root
            .join(replay_shard(&key))
            .join(format!("{key}.ndjson"))
    }

    pub async fn read_range(
        &self,
        document_id: &str,
        from: i64,
        to: i64,
    ) -> AppResult<Vec<RecordedEvent>> {
        let Some(payload) = self.load_payload(document_id).await? else {
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

    /// Total bytes of rrweb recording payloads stored in the database.
    pub async fn recording_bytes_total(&self) -> AppResult<i64> {
        self.index.recording_bytes_total().await
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

        let known: HashSet<String> = self.index.all_document_ids().await?.into_iter().collect();
        let orphan_files_deleted = self.sweep_orphan_replays(&known, now).await;

        Ok(RetentionSweepResult {
            recordings_deleted,
            orphan_files_deleted,
            claims_deleted: self
                .index
                .delete_released_claims_before(retention_cutoff)
                .await?,
        })
    }

    /// Deletes replay files whose document has no live stream row (drift from a
    /// crashed writer or a best-effort partial unlink that removed the row but
    /// not the file). Skips files younger than the grace window so an in-flight
    /// append is never mistaken for an orphan, and caps the scan. Best-effort:
    /// unreadable directories and failed unlinks are ignored, never surfaced.
    async fn sweep_orphan_replays(&self, known: &HashSet<String>, now: i64) -> u64 {
        let mut deleted = 0u64;
        let mut scanned = 0usize;
        let Ok(mut shards) = fs::read_dir(&self.replays_root).await else {
            return 0;
        };
        while scanned <= MAX_ORPHAN_FILE_SCAN
            && let Ok(Some(shard)) = shards.next_entry().await
        {
            if !shard
                .file_type()
                .await
                .map(|file_type| file_type.is_dir())
                .unwrap_or(false)
            {
                continue;
            }
            let Ok(mut files) = fs::read_dir(shard.path()).await else {
                continue;
            };
            while let Ok(Some(entry)) = files.next_entry().await {
                scanned += 1;
                if scanned > MAX_ORPHAN_FILE_SCAN {
                    return deleted;
                }
                let name = entry.file_name();
                let Some(document_id) = name
                    .to_str()
                    .and_then(|name| name.strip_suffix(".ndjson"))
                    .and_then(decode_replay_key)
                else {
                    continue; // foreign / non-key file: leave alone
                };
                if known.contains(&document_id) {
                    continue;
                }
                let Ok(meta) = entry.metadata().await else {
                    continue;
                };
                if !meta.is_file() || replay_file_too_young(&meta, now) {
                    continue;
                }
                if fs::remove_file(entry.path()).await.is_ok() {
                    deleted += 1;
                }
            }
        }
        deleted
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
                                orphan_files_deleted = result.orphan_files_deleted,
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

    /// Migrates every remaining DB-blob payload to its file, smallest first, then
    /// stops. Yields between documents so it never starves live recording traffic.
    pub fn spawn_payload_migration(self: Arc<Self>, cancel: CancellationToken) -> JoinHandle<()> {
        tokio::spawn(async move {
            let mut migrated = 0u64;
            let mut since_reclaim = 0u64;
            loop {
                if cancel.is_cancelled() {
                    return;
                }
                match self.migrate_next_document().await {
                    Ok(MigrationStep::Done) => {
                        if migrated > 0 {
                            let _ = self.index.incremental_reclaim().await;
                            info!(migrated, "recording payload migration finished");
                        }
                        return;
                    }
                    Ok(MigrationStep::Migrated(_)) => {
                        migrated += 1;
                        since_reclaim += 1;
                        if since_reclaim >= 64 {
                            let _ = self.index.incremental_reclaim().await;
                            since_reclaim = 0;
                        }
                    }
                    Ok(MigrationStep::Recovered) => {
                        tokio::select! {
                            () = cancel.cancelled() => return,
                            () = tokio::time::sleep(Duration::from_secs(5)) => {}
                        }
                    }
                    Err(error) => {
                        warn!(error = %error, "recording payload migration step failed");
                        tokio::select! {
                            () = cancel.cancelled() => return,
                            () = tokio::time::sleep(Duration::from_secs(30)) => {}
                        }
                    }
                }
                tokio::task::yield_now().await;
            }
        })
    }

    /// Extracts one un-migrated document's blob to its replay file (smallest
    /// first). Falls into disk-full recovery (drop the oldest recording) when a
    /// write runs out of space, restoring functionality rather than failing.
    pub async fn migrate_next_document(&self) -> AppResult<MigrationStep> {
        let Some((document_id, _size)) = self.index.smallest_unmigrated_document().await? else {
            return Ok(MigrationStep::Done);
        };
        match self.migrate_document(&document_id).await {
            Ok(bytes) => Ok(MigrationStep::Migrated(bytes)),
            Err(AppError::Io { source, .. }) if is_disk_full(&source) => {
                self.recover_disk_space().await?;
                Ok(MigrationStep::Recovered)
            }
            Err(error) => Err(error),
        }
    }

    async fn migrate_document(&self, document_id: &str) -> AppResult<i64> {
        let lock = self.lock_for(document_id).await;
        let guard = lock.lock().await;
        let result = self.extract_blob_locked(document_id).await;
        drop(guard);
        self.release_lock(document_id, &lock).await;
        result
    }

    /// Extracts a document's entire DB-blob payload to the start of its replay
    /// file and drops the blob, assuming the caller holds the document lock.
    /// Returns the committed file length, or 0 when no blob remains (a new
    /// document, or one a concurrent step already migrated). Idempotent.
    async fn extract_blob_locked(&self, document_id: &str) -> AppResult<i64> {
        let Some(payload) = self.index.payload(document_id).await? else {
            return Ok(0);
        };
        // A blob only ever exists while payload_bytes is 0, so it always occupies
        // the file from committed length 0. Setting the committed length and
        // dropping the blob commit atomically, so no crash can leave the blob
        // behind for a later append and migration to truncate over.
        let bytes = self.append_payload_file(document_id, 0, &payload).await?;
        self.index.finalize_extraction(document_id, bytes).await?;
        Ok(bytes)
    }

    /// Drops the oldest recording (blob, file, and row) to reclaim space when the
    /// disk is full, then returns freed pages to the OS where possible.
    async fn recover_disk_space(&self) -> AppResult<()> {
        if let Some(document_id) = self.index.oldest_document_id().await? {
            warn!(
                document_id,
                "audit disk full: dropping oldest recording to recover functionality"
            );
            self.delete_document(&document_id).await?;
        }
        let _ = self.index.incremental_reclaim().await;
        Ok(())
    }

    pub async fn close(&self) {}

    async fn delete_document(&self, document_id: &str) -> AppResult<bool> {
        let lock = self.lock_for(document_id).await;
        let guard = lock.lock().await;
        let result = async {
            let deleted = self.index.delete_document(document_id).await?;
            if deleted {
                remove_file(&self.replay_path_for(document_id), document_id).await;
            }
            Ok(deleted)
        }
        .await;
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

/// Whether an IO error is an out-of-disk condition.
fn is_disk_full(error: &std::io::Error) -> bool {
    // ENOSPC (28) on unix; ERROR_HANDLE_DISK_FULL (39) / ERROR_DISK_FULL (112) on windows.
    matches!(error.raw_os_error(), Some(28 | 39 | 112))
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

/// Collision-free, filesystem-safe single-segment key for a document id (the
/// replay file stem). base64url yields only `[A-Za-z0-9_-]`.
fn replay_key(document_id: &str) -> String {
    URL_SAFE_NO_PAD.encode(document_id.as_bytes())
}

/// Decodes a replay file stem (base64url of the document id) back to the
/// document id. Returns None for a foreign file whose name is not a replay key.
fn decode_replay_key(stem: &str) -> Option<String> {
    String::from_utf8(URL_SAFE_NO_PAD.decode(stem).ok()?).ok()
}

/// Whether a replay file is within the orphan grace window, i.e. recent enough
/// to still be an in-flight append whose stream row has not committed yet.
fn replay_file_too_young(meta: &std::fs::Metadata, now: i64) -> bool {
    let Some(mtime_ms) = meta
        .modified()
        .ok()
        .and_then(|modified| modified.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|since| i64::try_from(since.as_millis()).unwrap_or(i64::MAX))
    else {
        return false;
    };
    now.saturating_sub(mtime_ms) < ORPHAN_FILE_GRACE_MS
}

/// Two-character shard directory for a replay key so `replays/` never becomes a
/// single directory with one file per recording.
fn replay_shard(key: &str) -> String {
    let prefix: String = key.chars().take(2).collect();
    if prefix.is_empty() {
        "_".to_string()
    } else {
        prefix
    }
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
            dir.path().join("replays"),
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
            store.replays_root.clone(),
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
                orphan_files_deleted: 0,
                claims_deleted: 0,
            }
        );
        assert!(index.stream(claimed).await?.is_some());
        assert!(index.stream(orphan).await?.is_none());
        assert!(!index.batch_exists(orphan, "orphan").await?);
        assert!(!index.payload_exists(orphan).await?);
        Ok(())
    }

    #[tokio::test]
    async fn migrates_a_db_blob_to_its_replay_file() -> anyhow::Result<()> {
        let (_dir, index, store) = setup().await?;
        let ndjson = "{\"ts\":10}\n{\"ts\":20}\n";
        index
            .seed_legacy_payload("doc-legacy", 7, ndjson, 10, 20, 2)
            .await?;
        // Before migration the read falls back to the DB blob.
        assert_eq!(store.read_range("doc-legacy", 0, 100).await?.len(), 2);

        assert!(matches!(
            store.migrate_next_document().await?,
            MigrationStep::Migrated(_)
        ));
        assert_eq!(index.unmigrated_document_count().await?, 0);
        assert!(matches!(
            store.migrate_next_document().await?,
            MigrationStep::Done
        ));

        // After migration the same events are served from the file.
        assert!(store.replay_path_for("doc-legacy").exists());
        assert_eq!(store.read_range("doc-legacy", 0, 100).await?.len(), 2);
        Ok(())
    }

    #[tokio::test]
    async fn appending_to_an_unmigrated_legacy_document_preserves_both_payloads()
    -> anyhow::Result<()> {
        let (_dir, index, store) = setup().await?;
        let doc = "018f47a7-1c2b-7def-8123-0123456789ab";
        // Pre-upgrade state: a stream row with its whole payload still in the DB
        // blob, never file-migrated (payload_bytes == 0).
        index
            .seed_legacy_payload(doc, 11, "{\"ts\":10}\n{\"ts\":20}\n", 10, 20, 2)
            .await?;

        // A new batch for the SAME document arrives before background migration
        // reaches it. The blob must be extracted first, not overwritten.
        assert!(
            store
                .append_batch(doc, 11, None, &[event(30)], "new-batch", false)
                .await?
        );

        // The inline extraction consumed the blob, so migration has nothing left
        // to move and cannot truncate the file back to the blob.
        assert_eq!(index.unmigrated_document_count().await?, 0);
        assert!(matches!(
            store.migrate_next_document().await?,
            MigrationStep::Done
        ));

        // Every event survives, legacy first then the new batch.
        let timestamps: Vec<i64> = store
            .read_range(doc, 0, 1000)
            .await?
            .into_iter()
            .map(|event| event.ts)
            .collect();
        assert_eq!(timestamps, vec![10, 20, 30]);
        Ok(())
    }

    #[tokio::test]
    async fn finalizing_extraction_commits_length_and_blob_removal_atomically() -> anyhow::Result<()>
    {
        let (_dir, index, _store) = setup().await?;
        index
            .seed_legacy_payload("doc", 11, "{\"ts\":1}\n{\"ts\":2}\n", 1, 2, 2)
            .await?;
        assert_eq!(index.unmigrated_document_count().await?, 1);
        assert_eq!(index.committed_payload_bytes("doc").await?, 0);

        index.finalize_extraction("doc", 18).await?;

        // Both effects land together. Were they not one transaction, a crash
        // between them could leave a nonzero committed length with the blob still
        // present, which a later append and migration would truncate over.
        assert_eq!(index.committed_payload_bytes("doc").await?, 18);
        assert_eq!(index.unmigrated_document_count().await?, 0);
        Ok(())
    }

    #[tokio::test]
    async fn orphan_replay_files_are_swept_while_tracked_files_are_kept() -> anyhow::Result<()> {
        let (_dir, index, store) = setup().await?;
        // A tracked recording: its replay file must survive the sweep.
        store
            .append_batch("keep-doc", 5, None, &[event(10)], "batch-a", false)
            .await?;
        let kept = store.replay_path_for("keep-doc");
        assert!(kept.exists());

        // An orphan replay file: a valid replay key with no stream row behind it.
        let orphan = store.replay_path_for("ghost-doc");
        if let Some(parent) = orphan.parent() {
            fs::create_dir_all(parent).await?;
        }
        fs::write(&orphan, b"{\"ts\":1}\n").await?;
        // A foreign file whose stem is not a replay key (the `.` is outside the
        // base64url alphabet, so decoding fails) must be left alone.
        let foreign = store.replays_root.join("zz").join("my.notes.ndjson");
        fs::create_dir_all(store.replays_root.join("zz")).await?;
        fs::write(&foreign, b"keep me\n").await?;

        let known: HashSet<String> = index.all_document_ids().await?.into_iter().collect();
        // now = i64::MAX so neither file is inside the young-file grace window.
        assert_eq!(store.sweep_orphan_replays(&known, i64::MAX).await, 1);
        assert!(kept.exists());
        assert!(!orphan.exists());
        assert!(foreign.exists());
        Ok(())
    }

    #[tokio::test]
    async fn uncommitted_file_residue_is_truncated_on_next_append() -> anyhow::Result<()> {
        use tokio::io::AsyncWriteExt as _;
        let (_dir, _index, store) = setup().await?;
        store
            .append_batch("doc", 3, None, &[event(10)], "batch-a", false)
            .await?;
        // Simulate a crash after a file append that never committed its batch:
        // extra bytes past the committed length.
        let path = store.replay_path_for("doc");
        let mut residue = fs::OpenOptions::new().append(true).open(&path).await?;
        residue.write_all(b"{\"ts\":999}\n").await?;
        residue.sync_all().await?;
        drop(residue);

        // The next real append truncates the residue back to the committed length.
        store
            .append_batch("doc", 3, None, &[event(20)], "batch-b", false)
            .await?;
        let timestamps: Vec<i64> = store
            .read_range("doc", 0, 1000)
            .await?
            .into_iter()
            .map(|event| event.ts)
            .collect();
        assert_eq!(timestamps, vec![10, 20]);
        Ok(())
    }
}
