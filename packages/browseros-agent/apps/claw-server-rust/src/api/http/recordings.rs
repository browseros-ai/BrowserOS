use super::{error, internal};
use crate::{
    AppState,
    error::{CanonicalError, RequestId},
    services::recordings::RecordingEventInput,
};
use axum::{
    Extension, Json,
    extract::{State, rejection::StringRejection},
    http::{HeaderMap, StatusCode, header},
};
use claw_api::models::AppendRecordingEventsResponse;

pub(super) async fn append_document_events(
    Extension(request_id): Extension<RequestId>,
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Result<String, StringRejection>,
) -> Result<Json<AppendRecordingEventsResponse>, CanonicalError> {
    let body = recording_body(&request_id, body)?;
    require_ndjson(&request_id, &headers)?;
    let tab_id = positive_recording_header(&request_id, &headers, "x-recording-tab-id")?;
    let document_id = required_header(&request_id, &headers, "x-recording-document-id")?;
    let batch_id = required_header(&request_id, &headers, "x-recording-batch-id")?;
    let gap_header = gap_header(&request_id, &headers)?;
    if !is_chrome_document_id(&document_id) {
        return Err(error(
            &request_id,
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "recording tab, document, batch, and gap headers are invalid",
        ));
    }
    let parsed = parse_recording_events(&body);
    let appended = state
        .recording_ingest
        .append_document(
            &document_id,
            tab_id,
            &parsed.events,
            &batch_id,
            gap_header || parsed.dropped_lines > 0,
        )
        .await
        .map_err(|source| internal(&request_id, source))?;
    Ok(Json(AppendRecordingEventsResponse::new(if appended {
        i64::try_from(parsed.events.len()).unwrap_or(i64::MAX)
    } else {
        0
    })))
}

/// Tolerant parse of recorder-supplied NDJSON: lines that are not JSON
/// or lack an integer `ts` are dropped, never fatal.
struct ParsedRecordingEvents {
    events: Vec<RecordingEventInput>,
    dropped_lines: usize,
}

fn parse_recording_events(body: &str) -> ParsedRecordingEvents {
    let mut events = Vec::new();
    let mut dropped_lines = 0;
    for line in body.lines().filter(|line| !line.trim().is_empty()) {
        let Ok(event) = serde_json::from_str::<serde_json::Value>(line) else {
            dropped_lines += 1;
            continue;
        };
        let Some(ts) = event.get("ts").and_then(serde_json::Value::as_i64) else {
            dropped_lines += 1;
            continue;
        };
        events.push(RecordingEventInput {
            ts,
            event_type: event.get("type").cloned(),
            data: event.get("data").cloned(),
        });
    }
    ParsedRecordingEvents {
        events,
        dropped_lines,
    }
}

fn recording_body(
    request_id: &RequestId,
    body: Result<String, StringRejection>,
) -> Result<String, CanonicalError> {
    body.map_err(|rejection| {
        if rejection.status() == StatusCode::PAYLOAD_TOO_LARGE {
            error(
                request_id,
                StatusCode::PAYLOAD_TOO_LARGE,
                "recording_payload_too_large",
                &format!(
                    "recording payload exceeds {} byte limit",
                    super::RECORDING_INGEST_MAX_BYTES
                ),
            )
        } else {
            error(
                request_id,
                StatusCode::BAD_REQUEST,
                "invalid_request",
                "recording payload must be valid UTF-8",
            )
        }
    })
}

fn require_ndjson(request_id: &RequestId, headers: &HeaderMap) -> Result<(), CanonicalError> {
    let valid = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| {
            value
                .to_ascii_lowercase()
                .starts_with("application/x-ndjson")
        });
    if valid {
        Ok(())
    } else {
        Err(error(
            request_id,
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "content-type must be application/x-ndjson",
        ))
    }
}

fn required_header(
    request_id: &RequestId,
    headers: &HeaderMap,
    name: &str,
) -> Result<String, CanonicalError> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| {
            error(
                request_id,
                StatusCode::BAD_REQUEST,
                "invalid_request",
                "recording tab, document, batch, and gap headers are invalid",
            )
        })
}

fn gap_header(request_id: &RequestId, headers: &HeaderMap) -> Result<bool, CanonicalError> {
    match headers
        .get("x-recording-has-gap")
        .and_then(|value| value.to_str().ok())
    {
        None | Some("false") => Ok(false),
        Some("true") => Ok(true),
        Some(_) => Err(error(
            request_id,
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "recording tab, document, batch, and gap headers are invalid",
        )),
    }
}

fn is_chrome_document_id(value: &str) -> bool {
    value.len() == 32 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
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
