use super::{error, internal};
use crate::{
    AppState,
    error::{CanonicalError, RequestId},
    ids::SessionId,
};
use axum::{
    Extension, Json,
    body::Body,
    extract::{Path, State},
    http::{HeaderValue, StatusCode, header},
    response::Response,
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use claw_api::models::{Tab, TabList, TabStatus, ToolEvent};

pub(super) async fn list(
    Extension(request_id): Extension<RequestId>,
    State(state): State<AppState>,
) -> Result<Json<TabList>, CanonicalError> {
    // Wakes the screencast idle governor so previews keep refreshing
    // while anyone is watching the tab list.
    state.screencast.note_read();
    let profiles = state
        .agents
        .list_profiles()
        .await
        .map_err(|source| internal(&request_id, source))?;
    let mut items = Vec::new();
    for record in state.live_tab_activity().await {
        let session = state
            .sessions
            .lookup(&SessionId::new(record.session_id.clone()))
            .await;
        let profile = session
            .as_ref()
            .and_then(|session| session.agent().profile_id())
            .and_then(|profile_id| {
                profiles
                    .iter()
                    .find(|profile| profile.id == profile_id.as_str())
            });
        // Label preference: profile name, then the live agent's label,
        // then the recorded slug — the only identity that survives once
        // the session ends.
        let label = profile
            .map(|profile| profile.name.clone())
            .or_else(|| {
                session
                    .as_ref()
                    .map(|session| session.agent().label().to_string())
            })
            .unwrap_or_else(|| record.slug.clone());
        let recent_tools = record
            .recent_tools
            .into_iter()
            .map(|event| ToolEvent::new(event.name, event.at))
            .collect();
        let preview_captured_at = state
            .screencast
            .frame_for(record.page_id, &record.target_id)
            .await
            .map(|frame| frame.captured_at);
        let mut tab = Tab::new(
            record.tab_id,
            i64::from(record.page_id),
            record.target_id,
            record.slug.clone(),
            label,
            record.url,
            record.title,
            if record.status == "active" {
                TabStatus::Active
            } else {
                TabStatus::Idle
            },
            record.first_tool_at,
            record.last_tool_at,
            record.last_tool_name,
            i64::try_from(record.tool_count).unwrap_or(i64::MAX),
            recent_tools,
        );
        tab.session_id = session
            .as_ref()
            .map(|session| session.id().as_str().to_string());
        tab.profile_id = profile.map(|profile| profile.id.clone());
        tab.harness = profile.map(|profile| profile.harness.to_string());
        tab.color = Some(crate::tabs::hex_for_slug(&record.slug).to_string());
        tab.preview_captured_at = preview_captured_at;
        items.push(tab);
    }
    Ok(Json(TabList::new(items)))
}

pub(super) async fn preview(
    Extension(request_id): Extension<RequestId>,
    State(state): State<AppState>,
    Path(page_id): Path<String>,
) -> Result<Response, CanonicalError> {
    let page_id = positive_page_id(&request_id, &page_id)?;
    let target_id = state
        .live_tab_activity()
        .await
        .into_iter()
        .find(|record| record.page_id == page_id)
        .map(|record| record.target_id);
    let frame = match target_id {
        Some(target_id) => state.screencast.frame_for(page_id, &target_id).await,
        None => None,
    }
    .ok_or_else(|| {
        error(
            &request_id,
            StatusCode::NOT_FOUND,
            "preview_not_found",
            "tab preview not found",
        )
    })?;
    let bytes = STANDARD.decode(frame.jpeg_base64).map_err(|source| {
        tracing::error!(request_id = %request_id.0, error = %source, "cached preview is invalid");
        error(
            &request_id,
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal_error",
            "internal server error",
        )
    })?;
    // An empty cached frame is treated as missing, not as an error —
    // the client's fallback is the same either way.
    if bytes.is_empty() {
        return Err(error(
            &request_id,
            StatusCode::NOT_FOUND,
            "preview_not_found",
            "tab preview not found",
        ));
    }
    // Superseded by the next screencast frame — never serve from cache.
    Ok(jpeg_response(bytes, "private, max-age=0, must-revalidate"))
}

pub(super) async fn screenshot(
    Extension(request_id): Extension<RequestId>,
    State(state): State<AppState>,
    Path(dispatch_id): Path<String>,
) -> Result<Response, CanonicalError> {
    let dispatch_id = positive_i64(&request_id, &dispatch_id, "dispatchId")?;
    match state.screenshots.read(&dispatch_id.to_string()).await {
        // Written once at capture time — safe to cache hard.
        Ok(bytes) => Ok(jpeg_response(bytes, "public, max-age=86400, immutable")),
        Err(source) if source.status() == StatusCode::NOT_FOUND => Err(error(
            &request_id,
            StatusCode::NOT_FOUND,
            "screenshot_not_found",
            "dispatch screenshot not found",
        )),
        Err(source) => Err(internal(&request_id, source)),
    }
}

fn positive_page_id(request_id: &RequestId, raw: &str) -> Result<u32, CanonicalError> {
    let value = positive_i64(request_id, raw, "pageId")?;
    u32::try_from(value).map_err(|_| invalid_id(request_id, "pageId"))
}

fn positive_i64(request_id: &RequestId, raw: &str, name: &str) -> Result<i64, CanonicalError> {
    let value = raw
        .parse::<i64>()
        .map_err(|_| invalid_id(request_id, name))?;
    if value <= 0 {
        return Err(invalid_id(request_id, name));
    }
    Ok(value)
}

fn invalid_id(request_id: &RequestId, name: &str) -> CanonicalError {
    error(
        request_id,
        StatusCode::BAD_REQUEST,
        "invalid_request",
        &format!("{name} must be positive"),
    )
}

fn jpeg_response(bytes: Vec<u8>, cache_control: &'static str) -> Response {
    let mut response = Response::new(Body::from(bytes));
    response
        .headers_mut()
        .insert(header::CONTENT_TYPE, HeaderValue::from_static("image/jpeg"));
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static(cache_control),
    );
    response
}
