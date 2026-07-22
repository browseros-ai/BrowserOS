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
    /// Lower bound in Unix-epoch milliseconds, clipped to the session's tab-ownership window.
    pub first_event_at: i64,
    /// Upper bound in Unix-epoch milliseconds, clipped to the session's tab-ownership window.
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

impl ReplayService {
    #[must_use]
    pub fn new(recordings: Arc<RecordingStore>, index: Arc<RecordingIndex>) -> Arc<Self> {
        Arc::new(Self { recordings, index })
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
}
