//! Read-time attribution from logical-tab ownership windows to document streams.

use crate::{
    db::{RecordingIndex, StreamMatchRow},
    error::AppResult,
    services::recordings::{RecordedEvent, RecordingStore, legacy_document_id},
};
use serde::Serialize;
use std::{collections::HashMap, sync::Arc};

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplayEvent {
    pub session_id: String,
    pub document_id: String,
    pub tab_id: i64,
    pub target_id: Option<String>,
    #[serde(flatten)]
    pub event: RecordedEvent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplaySegmentMeta {
    pub document_id: String,
    pub target_id: Option<String>,
    /// Lower bound in Unix-epoch milliseconds, clipped to the envelope of this session's matching
    /// tab-ownership windows.
    pub first_event_at: i64,
    /// Upper bound in Unix-epoch milliseconds, clipped to the envelope of this session's matching
    /// tab-ownership windows.
    pub last_event_at: i64,
    pub size_bytes: i64,
    pub event_count: i64,
    pub has_gap: bool,
    pub legacy: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplayTabMeta {
    pub tab_id: i64,
    pub complete: bool,
    pub first_event_at: i64,
    pub last_event_at: i64,
    pub segments: Vec<ReplaySegmentMeta>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplayMeta {
    pub exists: bool,
    pub complete: bool,
    pub first_event_at: Option<i64>,
    pub last_event_at: Option<i64>,
    pub size_bytes: i64,
    pub tabs: Vec<ReplayTabMeta>,
}

/// Slices document streams through durable tab ownership windows.
pub struct ReplayService {
    recordings: Arc<RecordingStore>,
    index: Arc<RecordingIndex>,
}

/// The recording document a session's live preview should currently follow.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveDocument {
    pub document_id: String,
    pub tab_id: i64,
    pub target_id: Option<String>,
}

impl ReplayService {
    #[must_use]
    pub fn new(recordings: Arc<RecordingStore>, index: Arc<RecordingIndex>) -> Arc<Self> {
        Arc::new(Self { recordings, index })
    }

    /// The document a session's live preview should follow: the most recently
    /// active document the session owns, optionally pinned to one browser tab.
    /// Returns None when the session has no recorded document yet. Resolution is
    /// scoped to the session's own tab-ownership windows, so it cannot surface
    /// another session's recording.
    pub async fn live_document(
        &self,
        session_id: &str,
        browser_tab_id: Option<i64>,
    ) -> AppResult<Option<LiveDocument>> {
        // stream_matches carries every window the session ever owned, including
        // released ones, because replay reconstruction needs them. Live follow
        // wants only the tab the session is currently driving, so an open claim
        // (released_at IS NULL) is required; a fully released session yields None
        // and the caller transitions the stream to idle.
        let best = self
            .index
            .stream_matches(session_id)
            .await?
            .into_iter()
            .filter(|row| row.released_at.is_none())
            .filter(|row| browser_tab_id.is_none_or(|tab| row.tab_id == tab))
            .max_by_key(|row| row.last_event_at);
        Ok(best.map(|row| LiveDocument {
            document_id: row.document_id,
            tab_id: row.tab_id,
            target_id: row.target_id,
        }))
    }

    /// All committed rrweb events for one document, oldest first, for
    /// bootstrapping a live preview.
    /// A document's committed events with the committed byte length they were
    /// read at, so a live subscriber forwards only batches past that cutoff.
    pub async fn document_events_committed(
        &self,
        document_id: &str,
    ) -> AppResult<(Vec<RecordedEvent>, i64)> {
        self.recordings.read_committed(document_id).await
    }

    /// Batch ids already durably accepted for a document, so a live subscriber
    /// can de-duplicate a forwarded batch against what its bootstrap captured.
    pub async fn accepted_batch_ids(&self, document_id: &str) -> AppResult<Vec<String>> {
        self.index.accepted_batch_ids(document_id).await
    }

    pub async fn read_session(&self, session_id: &str) -> AppResult<Vec<ReplayEvent>> {
        let matches = self.matches(session_id).await?;
        let mut events = Vec::new();
        for stream in group_matches(matches) {
            let from = stream
                .windows
                .iter()
                .map(|window| window.claimed_at)
                .min()
                .unwrap_or(i64::MAX);
            let to = stream
                .windows
                .iter()
                .map(|window| window.released_at.unwrap_or(i64::MAX))
                .max()
                .unwrap_or(i64::MIN);
            events.extend(
                self.recordings
                    .read_range(&stream.document_id, from, to)
                    .await?
                    .into_iter()
                    .filter(|event| event_in_windows(event.ts, &stream.windows))
                    .map(|event| ReplayEvent {
                        session_id: session_id.to_string(),
                        document_id: stream.document_id.clone(),
                        tab_id: stream.tab_id,
                        target_id: stream.target_id.clone(),
                        event,
                    }),
            );
        }
        events.extend(self.read_legacy_session(session_id).await?);
        events.sort_by_key(|event| event.event.ts);
        Ok(events)
    }

    pub async fn meta(&self, session_id: &str) -> AppResult<ReplayMeta> {
        let mut entries = group_matches(self.matches(session_id).await?)
            .into_iter()
            .map(|stream| {
                let first_event_at = stream.first_event_at.max(
                    stream
                        .windows
                        .iter()
                        .map(|window| window.claimed_at)
                        .min()
                        .unwrap_or(stream.first_event_at),
                );
                let last_event_at = stream.last_event_at.min(
                    stream
                        .windows
                        .iter()
                        .map(|window| window.released_at.unwrap_or(i64::MAX))
                        .max()
                        .unwrap_or(stream.last_event_at),
                );
                (
                    stream.tab_id,
                    ReplaySegmentMeta {
                        legacy: stream.document_id.starts_with("legacy-"),
                        document_id: stream.document_id,
                        target_id: stream.target_id,
                        first_event_at,
                        last_event_at,
                        size_bytes: stream.size_bytes,
                        event_count: stream.event_count,
                        has_gap: stream.has_gap,
                    },
                )
            })
            .collect::<Vec<_>>();
        entries.extend(self.legacy_meta(session_id).await?);
        Ok(build_meta(entries))
    }

    async fn matches(&self, session_id: &str) -> AppResult<Vec<StreamMatchRow>> {
        self.index.stream_matches(session_id).await
    }

    async fn read_legacy_session(&self, session_id: &str) -> AppResult<Vec<ReplayEvent>> {
        let claims = self.index.legacy_claims(session_id).await?;
        let mut events = Vec::new();
        for claim in claims {
            events.extend(
                self.recordings
                    .read_legacy_range(
                        &claim.target_id,
                        claim.claimed_at,
                        claim.released_at.unwrap_or(i64::MAX),
                    )
                    .await?
                    .into_iter()
                    .map(|legacy| ReplayEvent {
                        session_id: session_id.to_string(),
                        document_id: legacy_document_id(&claim.target_id),
                        tab_id: legacy.tab_id,
                        target_id: Some(claim.target_id.clone()),
                        event: RecordedEvent {
                            ts: legacy.ts,
                            event_type: legacy.event_type,
                            data: legacy.data,
                        },
                    }),
            );
        }
        Ok(events)
    }

    async fn legacy_meta(&self, session_id: &str) -> AppResult<Vec<(i64, ReplaySegmentMeta)>> {
        let claims = self.index.legacy_claims(session_id).await?;
        let recordings = self
            .index
            .legacy_recordings()
            .await?
            .into_iter()
            .map(|recording| (recording.target_id.clone(), recording))
            .collect::<HashMap<_, _>>();
        Ok(claims
            .into_iter()
            .filter_map(|claim| {
                let recording = recordings.get(&claim.target_id)?;
                let first_event_at = claim.claimed_at.max(recording.first_event_at);
                let last_event_at = claim
                    .released_at
                    .unwrap_or(i64::MAX)
                    .min(recording.last_event_at);
                (first_event_at <= last_event_at).then(|| {
                    (
                        recording.tab_id,
                        ReplaySegmentMeta {
                            document_id: legacy_document_id(&claim.target_id),
                            target_id: Some(claim.target_id),
                            first_event_at,
                            last_event_at,
                            size_bytes: recording.size_bytes,
                            event_count: recording.event_count,
                            has_gap: true,
                            legacy: true,
                        },
                    )
                })
            })
            .collect())
    }
}

#[derive(Debug, Clone)]
struct Window {
    claimed_at: i64,
    released_at: Option<i64>,
}

#[derive(Debug)]
struct MatchedStream {
    document_id: String,
    tab_id: i64,
    target_id: Option<String>,
    first_event_at: i64,
    last_event_at: i64,
    size_bytes: i64,
    event_count: i64,
    has_gap: bool,
    windows: Vec<Window>,
}

fn group_matches(matches: Vec<StreamMatchRow>) -> Vec<MatchedStream> {
    let mut order = Vec::new();
    let mut grouped = HashMap::<String, MatchedStream>::new();
    for row in matches {
        let document_id = row.document_id.clone();
        let entry = grouped.entry(document_id.clone()).or_insert_with(|| {
            order.push(document_id.clone());
            MatchedStream {
                document_id,
                tab_id: row.tab_id,
                target_id: row.target_id.clone(),
                first_event_at: row.first_event_at,
                last_event_at: row.last_event_at,
                size_bytes: row.size_bytes,
                event_count: row.event_count,
                has_gap: row.has_gap,
                windows: Vec::new(),
            }
        });
        entry.windows.push(Window {
            claimed_at: row.claimed_at,
            released_at: row.released_at,
        });
    }
    order
        .into_iter()
        .filter_map(|document_id| grouped.remove(&document_id))
        .collect()
}

fn event_in_windows(timestamp: i64, windows: &[Window]) -> bool {
    windows.iter().any(|window| {
        timestamp >= window.claimed_at && timestamp <= window.released_at.unwrap_or(i64::MAX)
    })
}

fn build_meta(entries: Vec<(i64, ReplaySegmentMeta)>) -> ReplayMeta {
    if entries.is_empty() {
        return ReplayMeta {
            exists: false,
            complete: true,
            first_event_at: None,
            last_event_at: None,
            size_bytes: 0,
            tabs: Vec::new(),
        };
    }
    let mut by_tab = HashMap::<i64, Vec<ReplaySegmentMeta>>::new();
    for (tab_id, segment) in entries {
        let segments = by_tab.entry(tab_id).or_default();
        if !segments
            .iter()
            .any(|candidate| candidate.document_id == segment.document_id)
        {
            segments.push(segment);
        }
    }
    let mut tabs = by_tab
        .into_iter()
        .map(|(tab_id, mut segments)| {
            segments.sort_by_key(|segment| segment.first_event_at);
            ReplayTabMeta {
                tab_id,
                complete: segments
                    .iter()
                    .all(|segment| !segment.has_gap && !segment.legacy),
                first_event_at: segments
                    .iter()
                    .map(|segment| segment.first_event_at)
                    .min()
                    .unwrap_or_default(),
                last_event_at: segments
                    .iter()
                    .map(|segment| segment.last_event_at)
                    .max()
                    .unwrap_or_default(),
                segments,
            }
        })
        .collect::<Vec<_>>();
    tabs.sort_by_key(|tab| tab.first_event_at);
    ReplayMeta {
        exists: true,
        complete: tabs.iter().all(|tab| tab.complete),
        first_event_at: tabs.iter().map(|tab| tab.first_event_at).min(),
        last_event_at: tabs.iter().map(|tab| tab.last_event_at).max(),
        size_bytes: tabs
            .iter()
            .flat_map(|tab| &tab.segments)
            .fold(0_i64, |sum, segment| sum.saturating_add(segment.size_bytes)),
        tabs,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        db::{DATABASE_FILENAME, Database, RecordingIndex},
        services::recordings::RecordingEventInput,
    };
    use serde_json::json;
    use std::time::Duration;
    use tempfile::tempdir;

    fn event(ts: i64, id: &str) -> RecordingEventInput {
        RecordingEventInput {
            ts,
            event_type: Some(json!(3)),
            data: Some(json!({ "id": id })),
        }
    }

    #[tokio::test]
    async fn joins_tab_windows_across_document_targets_and_filters_exactly() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let index = Arc::new(RecordingIndex::new(
            Database::open(dir.path().join(DATABASE_FILENAME)).await?,
        ));
        let recordings = RecordingStore::new(
            dir.path().join("recordings"),
            dir.path().join("replays"),
            index.clone(),
            10,
            Duration::from_secs(1),
        );
        recordings
            .append_batch(
                "018f47a7-1c2b-7def-8123-0123456789ab",
                11,
                Some("target-a"),
                &[event(90, "before"), event(100, "a"), event(150, "b")],
                "batch-a",
                false,
            )
            .await?;
        recordings
            .append_batch(
                "018f47a7-1c2b-7def-8123-0123456789ac",
                11,
                Some("target-b"),
                &[event(175, "c"), event(201, "after")],
                "batch-b",
                true,
            )
            .await?;
        index
            .insert_session_tab("session-a", "agent-a", 11, Some("target-a"), 100, Some(200))
            .await?;
        let replay = ReplayService::new(recordings, index);

        let events = replay.read_session("session-a").await?;
        assert_eq!(
            events
                .iter()
                .filter_map(|event| event.event.data.as_ref()?.get("id")?.as_str())
                .collect::<Vec<_>>(),
            ["a", "b", "c"]
        );
        assert_eq!(events[2].target_id.as_deref(), Some("target-b"));
        let meta = replay.meta("session-a").await?;
        assert_eq!(meta.tabs.len(), 1);
        assert_eq!(meta.tabs[0].segments.len(), 2);
        assert!(!meta.complete);
        Ok(())
    }

    #[tokio::test]
    async fn live_document_follows_newest_owned_tab_can_pin_and_never_leaks_sessions()
    -> anyhow::Result<()> {
        let dir = tempdir()?;
        let index = Arc::new(RecordingIndex::new(
            Database::open(dir.path().join(DATABASE_FILENAME)).await?,
        ));
        let recordings = RecordingStore::new(
            dir.path().join("recordings"),
            dir.path().join("replays"),
            index.clone(),
            10,
            Duration::from_secs(1),
        );
        // session-a owns tab 11 (older) and tab 12 (newest). session-b owns tab
        // 13, whose newest event predates session-a's tab 12: a global max would
        // wrongly pick tab 12, so returning tab 13 proves per-session scoping.
        recordings
            .append_batch(
                "018f47a7-1c2b-7def-8123-0123456789ab",
                11,
                Some("target-a"),
                &[event(100, "a")],
                "batch-a",
                false,
            )
            .await?;
        recordings
            .append_batch(
                "018f47a7-1c2b-7def-8123-0123456789ac",
                12,
                Some("target-b"),
                &[event(200, "b")],
                "batch-b",
                false,
            )
            .await?;
        recordings
            .append_batch(
                "018f47a7-1c2b-7def-8123-0123456789ad",
                13,
                Some("target-c"),
                &[event(150, "c")],
                "batch-c",
                false,
            )
            .await?;
        index
            .insert_session_tab("session-a", "agent-a", 11, Some("target-a"), 0, None)
            .await?;
        index
            .insert_session_tab("session-a", "agent-a", 12, Some("target-b"), 0, None)
            .await?;
        index
            .insert_session_tab("session-b", "agent-b", 13, Some("target-c"), 0, None)
            .await?;
        // Released ownership windows are historical, not live. Both tabs below
        // have release timestamps at or after their events, so stream_matches
        // still returns them for replay; only live follow must exclude them.
        recordings
            .append_batch(
                "018f47a7-1c2b-7def-8123-0123456789ae",
                14,
                Some("target-d"),
                &[event(300, "d")],
                "batch-d",
                false,
            )
            .await?;
        recordings
            .append_batch(
                "018f47a7-1c2b-7def-8123-0123456789af",
                15,
                Some("target-e"),
                &[event(200, "e")],
                "batch-e",
                false,
            )
            .await?;
        recordings
            .append_batch(
                "018f47a7-1c2b-7def-8123-0123456789b0",
                16,
                Some("target-f"),
                &[event(100, "f")],
                "batch-f",
                false,
            )
            .await?;
        // session-c: tab 14 released (newer event), tab 15 open (older event).
        index
            .insert_session_tab("session-c", "agent-c", 14, Some("target-d"), 0, Some(500))
            .await?;
        index
            .insert_session_tab("session-c", "agent-c", 15, Some("target-e"), 0, None)
            .await?;
        // session-d: its only tab is released.
        index
            .insert_session_tab("session-d", "agent-d", 16, Some("target-f"), 0, Some(200))
            .await?;
        let replay = ReplayService::new(recordings, index);

        // Auto-follow resolves to the most recently active owned document.
        let Some(live) = replay.live_document("session-a", None).await? else {
            anyhow::bail!("expected a live document");
        };
        assert_eq!(live.tab_id, 12);
        assert_eq!(live.document_id, "018f47a7-1c2b-7def-8123-0123456789ac");

        // Pinning follows the requested owned tab instead.
        let Some(pinned) = replay.live_document("session-a", Some(11)).await? else {
            anyhow::bail!("expected a pinned document");
        };
        assert_eq!(pinned.tab_id, 11);
        assert_eq!(pinned.document_id, "018f47a7-1c2b-7def-8123-0123456789ab");

        // Scoping: session-b sees only its own tab 13, never session-a's newer tab.
        let Some(other) = replay.live_document("session-b", None).await? else {
            anyhow::bail!("expected session-b's own document");
        };
        assert_eq!(other.tab_id, 13);

        // An unknown session and an unowned tab both resolve to nothing.
        assert!(
            replay
                .live_document("session-unknown", None)
                .await?
                .is_none()
        );
        assert!(replay.live_document("session-a", Some(99)).await?.is_none());

        // Auto-follow skips the newer released tab 14 for the still-open tab 15.
        let Some(open_only) = replay.live_document("session-c", None).await? else {
            anyhow::bail!("expected the still-open tab");
        };
        assert_eq!(open_only.tab_id, 15);
        // A fully released session resolves to nothing, so the SSE stream goes
        // idle instead of replaying its former document.
        assert!(replay.live_document("session-d", None).await?.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn accepted_batch_ids_lists_every_committed_batch_for_a_document() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let index = Arc::new(RecordingIndex::new(
            Database::open(dir.path().join(DATABASE_FILENAME)).await?,
        ));
        let recordings = RecordingStore::new(
            dir.path().join("recordings"),
            dir.path().join("replays"),
            index.clone(),
            10,
            Duration::from_secs(1),
        );
        let doc = "018f47a7-1c2b-7def-8123-0123456789ab";
        recordings
            .append_batch(doc, 11, None, &[event(1, "a")], "batch-a", false)
            .await?;
        recordings
            .append_batch(doc, 11, None, &[event(2, "b")], "batch-b", false)
            .await?;
        let replay = ReplayService::new(recordings, index);

        let mut ids = replay.accepted_batch_ids(doc).await?;
        ids.sort();
        assert_eq!(ids, vec!["batch-a".to_string(), "batch-b".to_string()]);
        assert!(replay.accepted_batch_ids("other-doc").await?.is_empty());
        Ok(())
    }
}
