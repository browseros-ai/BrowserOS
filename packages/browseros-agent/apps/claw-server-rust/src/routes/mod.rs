use crate::{
    AppState,
    domain::SessionId,
    error::{AppError, AppResult},
    mcp::streamable_http_service,
    services::{
        agents::Harness,
        audit::{ListDispatchesQuery, ListTasksQuery, TaskStatus},
        replay::ReplayService,
        replay_tabs::{ReplayTabsResponse, list_replay_tabs},
        tab_activity::EnrichedTabRecord,
    },
};
use axum::{
    Json, Router,
    body::Body,
    extract::{Path, Query, Request, State},
    http::{HeaderValue, Method, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, options, post},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{str::FromStr, time::Instant};
use tracing::{Instrument, info_span};
use ulid::Ulid;

pub fn router(state: AppState) -> Router<AppState> {
    Router::new()
        .route("/system/health", get(system_health))
        .route("/system/shutdown", post(system_shutdown))
        .route("/system/version", get(system_version))
        .route("/system/url", get(system_url))
        .route("/agents/{agent_id}/cancel", post(agents_cancel))
        .route("/tabs/activity", get(tabs_activity))
        .route("/connections", get(connections_list))
        .route("/connections/{harness}/connect", post(connections_connect))
        .route(
            "/connections/{harness}/disconnect",
            post(connections_disconnect),
        )
        .route("/audit/dispatches", get(audit_dispatches))
        .route("/audit/tasks", get(audit_tasks))
        .route("/audit/tasks/{session_id}", get(audit_task_detail))
        .route("/audit/screenshot/{dispatch_id}", get(audit_screenshot))
        .route(
            "/audit/replay/{session_id}/events",
            post(replay_post_events),
        )
        .route("/audit/replay/{session_id}", get(replay_get))
        .route("/audit/replay/{session_id}/exists", get(replay_exists))
        .route("/replay/tabs", get(replay_tabs))
        .nest_service(
            "/mcp",
            Router::new()
                .fallback_service(streamable_http_service(state))
                .layer(middleware::from_fn(mcp_request_hygiene)),
        )
        .route("/{*path}", options(preflight))
}

/// Enforces the header conventions native MCP clients follow (parity with
/// the TS server's mcp-request-hygiene middleware). A browser-page fetch
/// against the loopback MCP endpoint always carries `origin` or
/// `sec-fetch-site`; native MCP clients never do.
async fn mcp_request_hygiene(req: Request, next: Next) -> Response {
    // The nested /mcp service shadows the router's `/{*path}` preflight
    // route, and the TS server's cors layer answers OPTIONS before its
    // hygiene runs — mirror both so preflight stays 204 here too.
    if *req.method() == Method::OPTIONS {
        return StatusCode::NO_CONTENT.into_response();
    }
    let headers = req.headers();
    if headers.contains_key(header::ORIGIN) || headers.contains_key("sec-fetch-site") {
        return AppError::forbidden("unsupported request").into_response();
    }
    let needs_json = match *req.method() {
        Method::POST | Method::PUT | Method::PATCH => true,
        // rmcp's DELETE /mcp session teardown carries no body and no
        // content-type; the TS server never sees that shape (its clients
        // always send application/json), so exempt only that case.
        Method::DELETE => headers.contains_key(header::CONTENT_TYPE),
        _ => false,
    };
    if needs_json {
        let is_json = headers
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.to_ascii_lowercase().contains("application/json"));
        if !is_json {
            return AppError::unsupported_media_type("unsupported content type").into_response();
        }
    }
    next.run(req).await
}

pub async fn request_context(req: Request, next: Next) -> Response {
    let request_id = Ulid::new().to_string();
    let method = req.method().clone();
    let path = req.uri().path().to_string();
    let span = info_span!("http_request", %request_id, %method, %path);
    async move {
        let start = Instant::now();
        let mut response = next.run(req).await;
        // One structured line per failed request; sub-400 traffic stays
        // unlogged on purpose (claw-app polls several endpoints).
        let status = response.status().as_u16();
        if status >= 400 {
            let duration_ms = start.elapsed().as_millis() as u64;
            if status >= 500 {
                tracing::error!(%method, %path, status, duration_ms, "request failed");
            } else {
                tracing::warn!(%method, %path, status, duration_ms, "request failed");
            }
        }
        let headers = response.headers_mut();
        headers.insert(
            header::ACCESS_CONTROL_ALLOW_ORIGIN,
            HeaderValue::from_static("*"),
        );
        headers.insert(
            header::ACCESS_CONTROL_ALLOW_METHODS,
            HeaderValue::from_static("GET,POST,PATCH,DELETE,OPTIONS"),
        );
        headers.insert(
            header::ACCESS_CONTROL_ALLOW_HEADERS,
            HeaderValue::from_static(
                "accept,content-type,authorization,mcp-session-id,mcp-protocol-version,last-event-id",
            ),
        );
        if let Ok(value) = HeaderValue::from_str(&request_id) {
            headers.insert("x-request-id", value);
        }
        response
    }
    .instrument(span)
    .await
}

async fn preflight() -> StatusCode {
    StatusCode::NO_CONTENT
}

async fn system_health(State(state): State<AppState>) -> Json<Value> {
    let cdp = state.browser.state();
    Json(json!({
        "status": "ok",
        "cdp": cdp,
        "sessions": {
            "count": state.sessions.count().await
        }
    }))
}

async fn system_shutdown(State(state): State<AppState>) -> AppResult<Json<Value>> {
    let drained = state.sessions.shutdown().await?;
    state.screencast.stop();
    state.browser.stop();
    if let Some(tx) = state.shutdown.lock().await.take() {
        let _ = tx.send(());
    }
    Ok(Json(json!({ "status": "ok", "drainedSessions": drained })))
}

async fn system_version() -> Json<Value> {
    Json(json!({
        "name": env!("CARGO_PKG_NAME"),
        "version": env!("CARGO_PKG_VERSION"),
    }))
}

async fn system_url(State(state): State<AppState>) -> Json<Value> {
    Json(json!({ "url": state.config.local_server_url() }))
}

async fn agents_cancel(
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
) -> Response {
    let cancelled = state.sessions.cancel_by_agent(&agent_id).await;
    if cancelled == 0 {
        // claw-app parses this 404 body as a CancelAgentResult (idle state,
        // not a failure), so it must keep the TS shape, not `{"error":...}`.
        return (
            StatusCode::NOT_FOUND,
            Json(json!({
                "ok": false,
                "cancelled": 0,
                "reason": "no active dispatches for this agent",
            })),
        )
            .into_response();
    }
    tracing::info!(%agent_id, cancelled, "cancelled in-flight dispatches for agent");
    Json(json!({ "ok": true, "cancelled": cancelled })).into_response()
}

async fn tabs_activity(State(state): State<AppState>) -> AppResult<Json<Value>> {
    state.screencast.note_read();
    let profiles = state.agents.list_profiles().await?;
    let tabs = state.tab_activity.snapshot().await;
    let mut enriched = Vec::with_capacity(tabs.len());
    for record in tabs {
        let profile = profiles
            .iter()
            .find(|profile| profile.id == record.agent_id);
        enriched.push(EnrichedTabRecord {
            agent_label: profile
                .map(|profile| profile.name.clone())
                .unwrap_or_else(|| record.slug.clone()),
            harness: profile.map(|profile| profile.harness.to_string()),
            color: None,
            screencast: state.screencast.frame_for(record.page_id).await,
            record,
        });
    }
    Ok(Json(json!({ "tabs": enriched })))
}

async fn connections_list(State(state): State<AppState>) -> AppResult<Json<Value>> {
    Ok(Json(json!({
        "connections": state.harness.list_browseros_connections().await?
    })))
}

async fn connections_connect(
    State(state): State<AppState>,
    Path(harness): Path<String>,
) -> AppResult<Json<Value>> {
    let harness = Harness::from_str(&harness)?;
    let result = state
        .harness
        .connect_browseros(harness, &state.config.public_mcp_url())
        .await?;
    Ok(Json(serde_json::to_value(result)?))
}

async fn connections_disconnect(
    State(state): State<AppState>,
    Path(harness): Path<String>,
) -> AppResult<Json<Value>> {
    let harness = Harness::from_str(&harness)?;
    let result = state.harness.disconnect_browseros(harness).await?;
    Ok(Json(serde_json::to_value(result)?))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DispatchesQuery {
    agent_id: Option<String>,
    session_id: Option<String>,
    cursor: Option<i64>,
    limit: Option<i64>,
}

async fn audit_dispatches(
    State(state): State<AppState>,
    Query(query): Query<DispatchesQuery>,
) -> AppResult<Json<Value>> {
    validate_limit(query.limit, 500)?;
    let result = state
        .audit
        .list_dispatches(ListDispatchesQuery {
            agent_id: query.agent_id,
            session_id: query.session_id,
            cursor: query.cursor,
            limit: query.limit,
        })
        .await?;
    Ok(Json(serde_json::to_value(result)?))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TasksQuery {
    agent_id: Option<String>,
    status: Option<TaskStatus>,
    site: Option<String>,
    search: Option<String>,
    since: Option<i64>,
    cursor: Option<i64>,
    limit: Option<i64>,
}

async fn audit_tasks(
    State(state): State<AppState>,
    Query(query): Query<TasksQuery>,
) -> AppResult<Json<Value>> {
    validate_limit(query.limit, 100)?;
    let result = state
        .audit
        .list_tasks(ListTasksQuery {
            agent_id: query.agent_id,
            status: query.status,
            site: query.site,
            search: query.search,
            since: query.since,
            cursor: query.cursor,
            limit: query.limit,
        })
        .await?;
    Ok(Json(serde_json::to_value(result)?))
}

async fn audit_task_detail(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> AppResult<Json<Value>> {
    let task = state
        .audit
        .get_task(&session_id)
        .await?
        .ok_or_else(|| AppError::not_found("not found"))?;
    Ok(Json(serde_json::to_value(task)?))
}

async fn audit_screenshot(
    State(state): State<AppState>,
    Path(dispatch_id): Path<String>,
) -> AppResult<impl IntoResponse> {
    let bytes = state.screenshots.read(&dispatch_id).await?;
    Ok((
        [
            (header::CONTENT_TYPE, HeaderValue::from_static("image/jpeg")),
            (
                header::CACHE_CONTROL,
                HeaderValue::from_static("public, max-age=86400, immutable"),
            ),
        ],
        bytes,
    ))
}

async fn replay_post_events(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    body: String,
) -> AppResult<Json<Value>> {
    if !state
        .sessions
        .contains(&SessionId::new(session_id.clone()))
        .await
    {
        return Err(AppError::gone("session not live"));
    }
    if body.is_empty() {
        return Ok(Json(json!({ "ok": true, "accepted": 0 })));
    }
    let lines: Vec<String> = body
        .lines()
        .filter(|line| !line.is_empty())
        .map(|line| ReplayService::annotate_with_session_id(line, &session_id))
        .collect();
    let accepted = lines.len();
    state.replay.append_events(&session_id, &lines).await?;
    Ok(Json(json!({ "ok": true, "accepted": accepted })))
}

async fn replay_get(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> AppResult<Response> {
    let stat = state.replay.stat_session(&session_id).await?;
    if !stat.has_data {
        return Err(AppError::not_found("no replay for this session"));
    }
    let bytes = state.replay.read_events(&session_id).await?;
    let mut response = Response::new(Body::from(bytes));
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/x-ndjson"),
    );
    if let Ok(value) = HeaderValue::from_str(&stat.size_bytes.to_string()) {
        headers.insert(header::CONTENT_LENGTH, value);
    }
    headers.insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    Ok(response)
}

async fn replay_exists(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> AppResult<Json<Value>> {
    let stat = state.replay.stat_session(&session_id).await?;
    let mut value = serde_json::to_value(stat)?;
    if let Value::Object(obj) = &mut value {
        obj.insert("ok".to_string(), Value::Bool(true));
    }
    Ok(Json(value))
}

async fn replay_tabs(State(state): State<AppState>) -> Json<ReplayTabsResponse> {
    Json(list_replay_tabs(&state.sessions, &state.tab_activity).await)
}

fn validate_limit(limit: Option<i64>, cap: i64) -> AppResult<()> {
    if let Some(limit) = limit
        && (limit <= 0 || limit > cap)
    {
        return Err(AppError::bad_request("limit out of range"));
    }
    Ok(())
}
