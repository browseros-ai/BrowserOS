//! Live recording-preview stream (Server-Sent Events). Streams a session's
//! currently-active rrweb document to the cockpit so the running card can render
//! a live preview from the recording it already captures, without taking any
//! screenshots.

use super::replay::require_known_session;
use crate::{
    AppState,
    error::{CanonicalError, RequestId},
    services::{recordings::RecordedEvent, replay::LiveDocument},
};
use axum::{
    Extension,
    extract::{Path, Query, State},
    response::sse::{Event, KeepAlive, Sse},
};
use futures_util::Stream;
use serde::Deserialize;
use serde_json::json;
use std::convert::Infallible;
use tokio::sync::{broadcast, mpsc};

#[derive(Debug, Default, Deserialize)]
pub(super) struct LiveParams {
    /// Pin the preview to one owned browser tab; absent follows the live target.
    #[serde(rename = "browserTabId")]
    browser_tab_id: Option<i64>,
}

/// GET /api/v1/sessions/{session_id}/recording/live
pub(super) async fn live(
    Extension(request_id): Extension<RequestId>,
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Query(params): Query<LiveParams>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, CanonicalError> {
    require_known_session(&state, &request_id, &session_id).await?;
    let (tx, rx) = mpsc::channel::<Result<Event, Infallible>>(64);
    tokio::spawn(stream_session_preview(
        state,
        session_id,
        params.browser_tab_id,
        tx,
    ));
    let stream =
        futures_util::stream::unfold(rx, |mut rx| async move { rx.recv().await.map(|e| (e, rx)) });
    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}

/// Resolves the session's live document, bootstraps a late joiner from the most
/// recent full snapshot, then forwards freshly ingested batches. Ends when the
/// client disconnects (the mpsc receiver drops) or the subscriber falls behind.
async fn stream_session_preview(
    state: AppState,
    session_id: String,
    browser_tab_id: Option<i64>,
    tx: mpsc::Sender<Result<Event, Infallible>>,
) {
    let document = match state
        .replay
        .live_document(&session_id, browser_tab_id)
        .await
    {
        Ok(Some(document)) => document,
        Ok(None) => {
            let _ = tx.send(Ok(control("idle", "no-recording"))).await;
            return;
        }
        Err(_) => return,
    };

    // Subscribe before the bootstrap read so no batch is lost in the gap between
    // reading stored events and forwarding live ones.
    let mut receiver = state.live_recordings.subscribe(&document.document_id).await;

    let bootstrap = match state.replay.document_events(&document.document_id).await {
        Ok(events) => bootstrap_from_last_snapshot(events),
        Err(_) => return,
    };
    let mut last_ts = bootstrap.last().map_or(i64::MIN, |event| event.ts);

    if send_switch(&tx, &document).await.is_err() {
        return;
    }
    if !bootstrap.is_empty() && send_events(&tx, "bootstrap", &bootstrap).await.is_err() {
        return;
    }

    loop {
        tokio::select! {
            () = tx.closed() => return,
            received = receiver.recv() => match received {
                Ok(batch) => {
                    // Drop anything already covered by the bootstrap so a batch
                    // that landed during the read gap is never applied twice.
                    let fresh: Vec<RecordedEvent> = batch
                        .events
                        .iter()
                        .filter(|event| event.ts > last_ts)
                        .cloned()
                        .collect();
                    if fresh.is_empty() {
                        continue;
                    }
                    last_ts = fresh.last().map_or(last_ts, |event| event.ts);
                    if send_events(&tx, "append", &fresh).await.is_err() {
                        return;
                    }
                }
                // Fell too far behind the writer: reconnect and re-bootstrap
                // instead of applying stale, gapped frames.
                Err(broadcast::error::RecvError::Lagged(_)) => {
                    let _ = tx.send(Ok(control("reset", "lagged"))).await;
                    return;
                }
                Err(broadcast::error::RecvError::Closed) => return,
            },
        }
    }
}

fn control(event: &str, message: &str) -> Event {
    Event::default().event(event).data(message)
}

async fn send_switch(
    tx: &mpsc::Sender<Result<Event, Infallible>>,
    document: &LiveDocument,
) -> Result<(), ()> {
    let data = json!({ "documentId": document.document_id, "tabId": document.tab_id }).to_string();
    tx.send(Ok(Event::default().event("switch").data(data)))
        .await
        .map_err(|_| ())
}

async fn send_events(
    tx: &mpsc::Sender<Result<Event, Infallible>>,
    name: &str,
    events: &[RecordedEvent],
) -> Result<(), ()> {
    let data = serde_json::to_string(events).map_err(|_| ())?;
    tx.send(Ok(Event::default().event(name).data(data)))
        .await
        .map_err(|_| ())
}

/// rrweb needs a Meta (type 4) then FullSnapshot (type 2) to initialize. Stream
/// from the most recent snapshot so a late joiner renders immediately rather than
/// replaying the whole document. Empty when no snapshot has been recorded yet.
fn bootstrap_from_last_snapshot(events: Vec<RecordedEvent>) -> Vec<RecordedEvent> {
    let Some(full) = events
        .iter()
        .rposition(|event| rrweb_type(event) == Some(2))
    else {
        return Vec::new();
    };
    let start = events[..=full]
        .iter()
        .rposition(|event| rrweb_type(event) == Some(4))
        .unwrap_or(full)
        .min(full);
    events[start..].to_vec()
}

fn rrweb_type(event: &RecordedEvent) -> Option<u64> {
    event
        .event_type
        .as_ref()
        .and_then(serde_json::Value::as_u64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn event(ts: i64, rrweb_type: u64) -> RecordedEvent {
        RecordedEvent {
            ts,
            event_type: Some(json!(rrweb_type)),
            data: Some(json!({})),
        }
    }

    #[test]
    fn bootstrap_starts_at_the_last_meta_and_full_snapshot() {
        let events = vec![
            event(1, 4),  // stale meta
            event(2, 2),  // stale snapshot
            event(3, 3),  // incremental
            event(10, 4), // latest meta
            event(11, 2), // latest snapshot
            event(12, 3), // incremental after snapshot
        ];
        let boot = bootstrap_from_last_snapshot(events);
        assert_eq!(
            boot.iter().map(|e| e.ts).collect::<Vec<_>>(),
            vec![10, 11, 12]
        );
    }

    #[test]
    fn bootstrap_is_empty_without_a_full_snapshot() {
        let events = vec![event(1, 4), event(2, 3)];
        assert!(bootstrap_from_last_snapshot(events).is_empty());
    }
}
