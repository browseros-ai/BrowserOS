//! Two cockpit read models: explicit `status=live` returns a complete browser-reconciled
//! snapshot; other lists return cursor-paginated audit history, enriched from a live
//! transport session only while one exists.

use super::{error, internal};
use crate::{
    AppState,
    db::audit_log::{ListTasksQuery, TaskDetail, TaskStatus, TaskSummary, ToolDispatchRow},
    error::{CanonicalError, RequestId},
    ids::SessionId,
    services::{
        cockpit::{
            LiveActivityState, LiveSessionFilters, LiveSessionProjection, LiveStateProjection,
            LiveTabProjection,
        },
        sessions::Session,
    },
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use claw_api::models::{
    CancelSessionResponse, Dispatch, SessionBrowserTab, SessionDetail, SessionList, SessionStatus,
    SessionSummary,
};
use std::{collections::HashMap, sync::Arc};

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
    if query.status == Some(TaskStatus::Live) {
        let items = state
            .cockpit
            .list(&LiveSessionFilters {
                profile_id: query.profile_id,
                slug: query.slug,
                site: query.site,
                search: query.search,
                since: query.since,
            })
            .await
            .map_err(|source| internal(&request_id, source))?
            .into_iter()
            .map(contract_live_projection)
            .collect();
        return Ok(Json(SessionList::new(items)));
    }
    let result = state
        .audit_log
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
        .audit_log
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
        .audit_log
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

async fn live_sessions(state: &AppState) -> HashMap<String, Arc<Session>> {
    state
        .sessions
        .snapshot()
        .await
        .into_iter()
        .map(|session| (session.id().as_str().to_string(), session))
        .collect()
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

async fn contract_summary(task: TaskSummary, live: Option<&Arc<Session>>) -> SessionSummary {
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
        contract_status(task.status),
        task.error_count,
    );
    summary.profile_id = live
        .and_then(|session| session.agent().profile_id())
        .map(|profile_id| profile_id.as_str().to_string());
    summary.site = task.site;
    summary.ended_at = task.ended_at;
    summary.latest_screenshot_id = task.last_screenshot_dispatch_id;
    summary
}

fn contract_live_projection(projection: LiveSessionProjection) -> SessionSummary {
    let LiveSessionProjection {
        task,
        profile_id,
        harness,
        color,
        label,
        name,
        live,
    } = projection;
    let mut summary = SessionSummary::new(
        task.session_id,
        task.slug,
        label,
        name,
        task.started_at,
        task.duration_ms.max(0),
        task.dispatch_count,
        task.tool_sequence,
        SessionStatus::Live,
        task.error_count,
    );
    summary.profile_id = profile_id;
    summary.harness = harness;
    summary.color = Some(color);
    summary.site = task.site;
    summary.ended_at = task.ended_at;
    summary.latest_screenshot_id = task.last_screenshot_dispatch_id;
    summary.live = Some(Box::new(contract_live_state(live)));
    summary
}

fn contract_live_state(projection: LiveStateProjection) -> claw_api::models::LiveSessionState {
    claw_api::models::LiveSessionState::new(
        match projection.state {
            LiveActivityState::Active => claw_api::models::LiveSessionActivityState::Active,
            LiveActivityState::Idle => claw_api::models::LiveSessionActivityState::Idle,
        },
        projection
            .browser_tabs
            .into_iter()
            .map(contract_live_tab)
            .collect(),
    )
}

fn contract_live_tab(projection: LiveTabProjection) -> SessionBrowserTab {
    let mut tab = SessionBrowserTab::new(
        projection.browser_tab_id,
        projection.url,
        projection.title,
        projection.tool_count,
        projection
            .recent_tools
            .into_iter()
            .map(|event| claw_api::models::ToolEvent::new(event.name, event.at))
            .collect(),
    );
    tab.first_activity_at = projection.first_activity_at;
    tab.last_activity_at = projection.last_activity_at;
    tab.last_tool_name = projection.last_tool_name;
    tab
}

fn contract_status(status: TaskStatus) -> SessionStatus {
    match status {
        TaskStatus::Live => SessionStatus::Live,
        TaskStatus::Done => SessionStatus::Done,
        TaskStatus::Failed => SessionStatus::Failed,
    }
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
    );
    dispatch.screenshot_id = screenshots.contains(&row.id).then_some(row.id);
    dispatch.profile_id = profile_id.cloned();
    dispatch.page_id = row.page_id;
    dispatch.tab_id = row.tab_id;
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
