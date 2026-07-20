//! Read-time attribution from logical-tab ownership windows to document streams.

use crate::{
    capture::{
        audit::AuditService,
        recordings::{RecordedEvent, RecordingStore, legacy_document_id},
    },
    db::audit::entities::{
        prelude::{TabClaims, TabRecordings},
        tab_claims,
    },
    error::AppResult,
};
use sea_orm::{ColumnTrait, DbBackend, EntityTrait, FromQueryResult, QueryFilter, Statement};
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
    pub first_event_at: i64,
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
    audit: Arc<AuditService>,
}

impl ReplayService {
    #[must_use]
    pub fn new(recordings: Arc<RecordingStore>, audit: Arc<AuditService>) -> Arc<Self> {
        Arc::new(Self { recordings, audit })
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
        let statement = Statement::from_sql_and_values(
            DbBackend::Sqlite,
            r#"SELECT
                rs.document_id, rs.tab_id, rs.target_id,
                rs.first_event_at, rs.last_event_at, rs.size_bytes,
                rs.event_count, rs.has_gap,
                st.claimed_at, st.released_at
              FROM session_tabs st
              JOIN recording_streams rs
                ON rs.tab_id = st.tab_id
               AND rs.last_event_at >= st.claimed_at
               AND rs.first_event_at <= COALESCE(st.released_at, 9223372036854775807)
              WHERE st.session_id = ?
              ORDER BY rs.first_event_at"#,
            [session_id.into()],
        );
        Ok(StreamMatchRow::find_by_statement(statement)
            .all(self.audit.connection())
            .await?)
    }

    async fn read_legacy_session(&self, session_id: &str) -> AppResult<Vec<ReplayEvent>> {
        let claims = TabClaims::find()
            .filter(tab_claims::Column::SessionId.eq(session_id))
            .all(self.audit.connection())
            .await?;
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
        let claims = TabClaims::find()
            .filter(tab_claims::Column::SessionId.eq(session_id))
            .all(self.audit.connection())
            .await?;
        let recordings = TabRecordings::find()
            .all(self.audit.connection())
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

#[derive(Debug, Clone, FromQueryResult)]
struct StreamMatchRow {
    document_id: String,
    tab_id: i64,
    target_id: Option<String>,
    first_event_at: i64,
    last_event_at: i64,
    size_bytes: i64,
    event_count: i64,
    has_gap: bool,
    claimed_at: i64,
    released_at: Option<i64>,
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
        capture::recordings::RecordingEventInput,
        db::audit::entities::{prelude::SessionTabs, session_tabs},
    };
    use sea_orm::ActiveValue::{NotSet, Set};
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
        let audit = Arc::new(AuditService::open(dir.path().join("audit.sqlite")).await?);
        let recordings = RecordingStore::new(
            dir.path().join("recordings"),
            audit.clone(),
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
        SessionTabs::insert(session_tabs::ActiveModel {
            id: NotSet,
            session_id: Set("session-a".to_string()),
            agent_id: Set("agent-a".to_string()),
            tab_id: Set(11),
            opened_target_id: Set(Some("target-a".to_string())),
            claimed_at: Set(100),
            released_at: Set(Some(200)),
        })
        .exec(audit.connection())
        .await?;
        let replay = ReplayService::new(recordings, audit);

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
