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
use claw_api::models::{RecordingMetadata, RecordingSegmentMetadata, RecordingTabMetadata};

pub(super) async fn recording(
    Extension(request_id): Extension<RequestId>,
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<Json<RecordingMetadata>, CanonicalError> {
    require_known_session(&state, &request_id, &session_id).await?;
    let metadata = state
        .replay
        .meta(&session_id)
        .await
        .map_err(|source| internal(&request_id, source))?;
    let tabs = metadata
        .tabs
        .into_iter()
        .map(|tab| {
            let segments = tab
                .segments
                .into_iter()
                .map(|segment| {
                    let mut contract = RecordingSegmentMetadata::new(
                        segment.document_id,
                        segment.first_event_at,
                        segment.last_event_at,
                        segment.size_bytes,
                        segment.event_count,
                        segment.has_gap,
                    );
                    contract.target_id = segment.target_id;
                    contract.legacy = segment.legacy.then_some(true);
                    contract
                })
                .collect();
            RecordingTabMetadata::new(
                tab.tab_id,
                tab.complete,
                tab.first_event_at,
                tab.last_event_at,
                segments,
            )
        })
        .collect();
    let mut response = RecordingMetadata::new(
        metadata.exists,
        metadata.complete,
        metadata.size_bytes,
        tabs,
    );
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
        .replay
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

async fn require_known_session(
    state: &AppState,
    request_id: &RequestId,
    session_id: &str,
) -> Result<(), CanonicalError> {
    if state.sessions.contains(&SessionId::new(session_id)).await {
        return Ok(());
    }
    if state
        .audit_log
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
