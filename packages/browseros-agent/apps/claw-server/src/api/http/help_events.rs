//! Help-request change stream (Server-Sent Events). Tells a listener, the extension's background
//! worker, the moment the open help requests change, so it can alert the person without polling.
//! Events carry no request details: the listener re-reads `GET /api/v1/sessions?status=live`,
//! which stays the one place a help request is shaped.

use crate::AppState;
use axum::{
    extract::State,
    response::sse::{Event, KeepAlive, Sse},
};
use futures_util::Stream;
use serde_json::json;
use std::convert::Infallible;

/// The event name every change is sent under.
const HELP_EVENT: &str = "help";

/// GET /api/v1/help/events
pub(super) async fn events(
    State(state): State<AppState>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let changes = state.help.subscribe();
    // The first event goes out at once, so a listener that connects mid-request reads straight
    // away; later events follow each change.
    let stream = futures_util::stream::unfold((changes, true), |(mut changes, first)| async move {
        if !first && changes.changed().await.is_err() {
            return None;
        }
        let version = *changes.borrow_and_update();
        let event = Event::default()
            .event(HELP_EVENT)
            .data(json!({ "version": version }).to_string());
        Some((Ok(event), (changes, false)))
    });
    Sse::new(stream).keep_alive(KeepAlive::default())
}
