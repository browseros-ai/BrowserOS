use super::{error, internal};
use crate::{
    AppState,
    capture::{
        audit::{ListTasksQuery, TaskDetail, TaskStatus, TaskSummary, ToolDispatchRow},
        recordings::RecordingEventInput,
    },
    error::{CanonicalError, RequestId},
    ids::SessionId,
    sessions::Session,
};
use axum::{
    Extension, Json,
    body::Body,
    extract::{Path, Query, State, rejection::StringRejection},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::Response,
};
use claw_api::models::{
    AppendRecordingEventsResponse, CancelSessionResponse, Dispatch, RecordingMetadata,
    SessionDetail, SessionList, SessionStatus, SessionSummary,
};
use std::{
    collections::{BTreeSet, HashMap},
    sync::Arc,
};

#[derive(Default)]
struct SessionQuery {
    profile_id: Option<String>,
    slug: Option<String>,
    status: Option<TaskStatus>,
    site: Option<String>,
    search: Option<String>,
    since: Option<i64>,
    cursor: Option<i64>,
    limit: Option<i64>,
}

pub(super) async fn list(
    Extension(request_id): Extension<RequestId>,
    State(state): State<AppState>,
    Query(raw): Query<HashMap<String, String>>,
) -> Result<Json<SessionList>, CanonicalError> {
    let query = parse_query(&request_id, &raw)?;
    let result = state
        .audit
        .list_tasks(ListTasksQuery {
            slug: query.slug,
            status: query.status,
            site: query.site,
            search: query.search,
            since: query.since,
            cursor: query.cursor,
            limit: query.limit,
            ..ListTasksQuery::default()
        })
        .await
        .map_err(|source| internal(&request_id, source))?;
    let live = live_sessions(&state).await;
    // profile_id lives on the live session's agent, not in the audit
    // store, so a profileId filter can only match live sessions — and
    // it applies after pagination, so a filtered page may come back
    // short rather than backfilled.
    let mut items = Vec::with_capacity(result.tasks.len());
    for task in result.tasks {
        let session = live.get(task.session_id.as_str());
        let summary = contract_summary(task, session).await;
        if query
            .profile_id
            .as_ref()
            .is_none_or(|profile_id| summary.profile_id.as_ref() == Some(profile_id))
        {
            items.push(summary);
        }
    }
    let mut response = SessionList::new(items);
    response.next_cursor = result.next_cursor;
    Ok(Json(response))
}

pub(super) async fn get(
    Extension(request_id): Extension<RequestId>,
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<Json<SessionDetail>, CanonicalError> {
    let task = state
        .audit
        .get_task(&session_id)
        .await
        .map_err(|source| internal(&request_id, source))?
        .ok_or_else(|| {
            error(
                &request_id,
                StatusCode::NOT_FOUND,
                "session_not_found",
                "session not found",
            )
        })?;
    let live = state.sessions.lookup(&SessionId::new(session_id)).await;
    Ok(Json(contract_detail(task, live.as_ref()).await))
}

pub(super) async fn cancel(
    Extension(request_id): Extension<RequestId>,
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<Json<CancelSessionResponse>, CanonicalError> {
    let session_id = SessionId::new(session_id);
    if let Some(cancelled) = state.sessions.cancel_by_session(&session_id).await {
        return Ok(Json(CancelSessionResponse::new(
            i64::try_from(cancelled).unwrap_or(i64::MAX),
        )));
    }
    let known = state
        .audit
        .get_task(session_id.as_str())
        .await
        .map_err(|source| internal(&request_id, source))?
        .is_some();
    Err(if known {
        error(
            &request_id,
            StatusCode::CONFLICT,
            "session_not_live",
            "session is not live",
        )
    } else {
        error(
            &request_id,
            StatusCode::NOT_FOUND,
            "session_not_found",
            "session not found",
        )
    })
}

pub(super) async fn recording(
    Extension(request_id): Extension<RequestId>,
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<Json<RecordingMetadata>, CanonicalError> {
    require_known_session(&state, &request_id, &session_id).await?;
    let metadata = state
        .replays
        .meta(&session_id)
        .await
        .map_err(|source| internal(&request_id, source))?;
    // page_ids = tabs still claimed by this session whose targets have
    // recorded events — the per-tab views a replay can offer.
    let target_ids = metadata
        .targets
        .iter()
        .map(|target| target.target_id.as_str())
        .collect::<std::collections::HashSet<_>>();
    let page_ids = state
        .live_tab_activity()
        .await
        .into_iter()
        .filter(|tab| tab.session_id == session_id && target_ids.contains(tab.target_id.as_str()))
        .map(|tab| i64::from(tab.page_id))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let mut response = RecordingMetadata::new(metadata.exists, metadata.size_bytes, page_ids);
    response.first_event_at = metadata.first_event_at;
    response.last_event_at = metadata.last_event_at;
    Ok(Json(response))
}

pub(super) async fn download_events(
    Extension(request_id): Extension<RequestId>,
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<Response, CanonicalError> {
    require_known_session(&state, &request_id, &session_id).await?;
    let events = state
        .replays
        .read_session(&session_id)
        .await
        .map_err(|source| internal(&request_id, source))?;
    let mut ndjson = String::new();
    for event in events {
        ndjson.push_str(
            &serde_json::to_string(&event)
                .map_err(|source| internal(&request_id, source.into()))?,
        );
        ndjson.push('\n');
    }
    let mut response = Response::new(Body::from(ndjson));
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/x-ndjson"),
    );
    Ok(response)
}

pub(super) async fn append_events(
    Extension(request_id): Extension<RequestId>,
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    headers: HeaderMap,
    body: Result<String, StringRejection>,
) -> Result<Json<AppendRecordingEventsResponse>, CanonicalError> {
    let body = body.map_err(|rejection| {
        if rejection.status() == StatusCode::PAYLOAD_TOO_LARGE {
            error(
                &request_id,
                StatusCode::PAYLOAD_TOO_LARGE,
                "recording_payload_too_large",
                &format!(
                    "recording payload exceeds {} byte limit",
                    claw_api::RECORDING_INGEST_MAX_BYTES
                ),
            )
        } else {
            error(
                &request_id,
                StatusCode::BAD_REQUEST,
                "invalid_request",
                "recording payload must be valid UTF-8",
            )
        }
    })?;
    let session_key = SessionId::new(session_id.clone());
    if !state.sessions.contains(&session_key).await {
        let known = state
            .audit
            .get_task(&session_id)
            .await
            .map_err(|source| internal(&request_id, source))?
            .is_some();
        return Err(if known {
            error(
                &request_id,
                StatusCode::GONE,
                "session_ended",
                "session has ended",
            )
        } else {
            error(
                &request_id,
                StatusCode::NOT_FOUND,
                "session_not_found",
                "session not found",
            )
        });
    }
    let is_ndjson = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| {
            value
                .to_ascii_lowercase()
                .starts_with("application/x-ndjson")
        });
    if !is_ndjson {
        return Err(error(
            &request_id,
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "content-type must be application/x-ndjson",
        ));
    }
    let tab_id = positive_recording_header(&request_id, &headers, "x-recording-tab-id")?;
    let page_id = positive_recording_header(&request_id, &headers, "x-recording-page-id")?;
    let target_id = recording_target_header(&request_id, &headers)?;
    let events = parse_recording_events(&body);
    // Batches are pinned to the (tab, page, target) incarnation the
    // recorder captured them from. Any drift — the tab reclaimed by
    // another session, a navigation that swapped the target — makes the
    // batch undeliverable rather than attributing its events to the
    // wrong replay; the 409 tells the recorder to drop its association.
    let Some(target) = state.live_tab_activity().await.into_iter().find(|tab| {
        tab.session_id == session_id
            && tab.tab_id == tab_id
            && i64::from(tab.page_id) == page_id
            && tab.target_id == target_id
    }) else {
        return Err(error(
            &request_id,
            StatusCode::CONFLICT,
            "recording_association_changed",
            "recording tab association changed",
        ));
    };
    let batch_id = headers
        .get("x-recording-batch-id")
        .and_then(|value| value.to_str().ok());
    let appended = state
        .recordings
        .append_batch_with_id(&target.target_id, target.tab_id, &events, batch_id)
        .await
        .map_err(|source| internal(&request_id, source))?;
    let accepted = if appended {
        i64::try_from(events.len()).unwrap_or(i64::MAX)
    } else {
        0
    };
    Ok(Json(AppendRecordingEventsResponse::new(accepted)))
}

/// Tolerant parse of recorder-supplied NDJSON: lines that are not JSON
/// or lack an integer `ts` are dropped, never fatal.
fn parse_recording_events(body: &str) -> Vec<RecordingEventInput> {
    body.lines()
        .filter_map(|line| {
            let event = serde_json::from_str::<serde_json::Value>(line).ok()?;
            let ts = event.get("ts")?.as_i64()?;
            Some(RecordingEventInput {
                ts,
                event_type: event.get("type").cloned(),
                data: event.get("data").cloned(),
            })
        })
        .collect()
}

fn positive_recording_header(
    request_id: &RequestId,
    headers: &HeaderMap,
    name: &str,
) -> Result<i64, CanonicalError> {
    let value = headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<i64>().ok())
        .filter(|value| *value > 0)
        .ok_or_else(|| {
            error(
                request_id,
                StatusCode::BAD_REQUEST,
                "invalid_request",
                "recording tab, page, and target headers are required",
            )
        })?;
    Ok(value)
}

fn recording_target_header(
    request_id: &RequestId,
    headers: &HeaderMap,
) -> Result<String, CanonicalError> {
    headers
        .get("x-recording-target-id")
        .and_then(|value| value.to_str().ok())
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| {
            error(
                request_id,
                StatusCode::BAD_REQUEST,
                "invalid_request",
                "recording tab, page, and target headers are required",
            )
        })
}

async fn require_known_session(
    state: &AppState,
    request_id: &RequestId,
    session_id: &str,
) -> Result<(), CanonicalError> {
    if state.sessions.contains(&SessionId::new(session_id)).await {
        return Ok(());
    }
    if state
        .audit
        .get_task(session_id)
        .await
        .map_err(|source| internal(request_id, source))?
        .is_some()
    {
        return Ok(());
    }
    Err(error(
        request_id,
        StatusCode::NOT_FOUND,
        "session_not_found",
        "session not found",
    ))
}

async fn live_sessions(state: &AppState) -> HashMap<String, Arc<Session>> {
    state
        .sessions
        .snapshot()
        .await
        .into_iter()
        .map(|session| (session.id().as_str().to_string(), session))
        .collect()
}

async fn contract_summary(task: TaskSummary, live: Option<&Arc<Session>>) -> SessionSummary {
    // A live session can still rename itself, so prefer its current
    // label; once ended, the audited title is all that remains.
    let name = match live {
        Some(session) => session.label().await,
        None => task.title.clone(),
    };
    let mut summary = SessionSummary::new(
        task.session_id,
        task.slug,
        task.agent_label,
        name,
        task.started_at,
        task.duration_ms.max(0),
        task.dispatch_count,
        task.tool_sequence,
        match task.status {
            TaskStatus::Live => SessionStatus::Live,
            TaskStatus::Done => SessionStatus::Done,
            TaskStatus::Failed => SessionStatus::Failed,
        },
        task.error_count,
    );
    summary.profile_id = live
        .and_then(|session| session.agent().profile_id())
        .map(|profile_id| profile_id.as_str().to_string());
    summary.site = task.site;
    summary.ended_at = task.ended_at;
    summary.last_screenshot_dispatch_id = task.last_screenshot_dispatch_id;
    summary
}

async fn contract_detail(task: TaskDetail, live: Option<&Arc<Session>>) -> SessionDetail {
    let screenshots = task
        .screenshot_dispatch_ids
        .into_iter()
        .collect::<std::collections::HashSet<_>>();
    let profile_id = live
        .and_then(|session| session.agent().profile_id())
        .map(|profile_id| profile_id.as_str().to_string());
    let dispatches = task
        .dispatches
        .into_iter()
        .map(|row| contract_dispatch(row, &screenshots, profile_id.as_ref()))
        .collect();
    SessionDetail::new(contract_summary(task.summary, live).await, dispatches)
}

fn contract_dispatch(
    row: ToolDispatchRow,
    screenshots: &std::collections::HashSet<i64>,
    profile_id: Option<&String>,
) -> Dispatch {
    let mut dispatch = Dispatch::new(
        row.id,
        row.created_at,
        row.slug,
        row.agent_label,
        row.session_id,
        row.tool_name,
        screenshots.contains(&row.id),
    );
    dispatch.profile_id = profile_id.cloned();
    dispatch.page_id = row.page_id;
    dispatch.target_id = row.target_id;
    dispatch.url = row.url;
    dispatch.title = row.title;
    dispatch.args_json = row.args_json;
    dispatch.result_meta = row.result_meta;
    dispatch.duration_ms = row.duration_ms;
    dispatch
}

fn parse_query(
    request_id: &RequestId,
    raw: &HashMap<String, String>,
) -> Result<SessionQuery, CanonicalError> {
    let status = match raw.get("status").map(String::as_str) {
        None => None,
        Some("live") => Some(TaskStatus::Live),
        Some("done") => Some(TaskStatus::Done),
        Some("failed") => Some(TaskStatus::Failed),
        Some(_) => return Err(invalid_query(request_id, "invalid status")),
    };
    Ok(SessionQuery {
        profile_id: raw.get("profileId").cloned(),
        slug: raw.get("slug").cloned(),
        status,
        site: raw.get("site").cloned(),
        search: raw.get("search").cloned(),
        since: parse_integer(request_id, raw, "since", 0, i64::MAX)?,
        cursor: parse_integer(request_id, raw, "cursor", 1, i64::MAX)?,
        limit: parse_integer(request_id, raw, "limit", 1, 100)?,
    })
}

fn parse_integer(
    request_id: &RequestId,
    raw: &HashMap<String, String>,
    key: &str,
    minimum: i64,
    maximum: i64,
) -> Result<Option<i64>, CanonicalError> {
    let Some(value) = raw.get(key) else {
        return Ok(None);
    };
    let value = value
        .parse::<i64>()
        .map_err(|_| invalid_query(request_id, "invalid integer query parameter"))?;
    if value < minimum || value > maximum {
        return Err(invalid_query(request_id, "query parameter out of range"));
    }
    Ok(Some(value))
}

fn invalid_query(request_id: &RequestId, message: &str) -> CanonicalError {
    error(
        request_id,
        StatusCode::BAD_REQUEST,
        "invalid_request",
        message,
    )
}
