//! `GET /v1/display`: Neo's one screen as RFB over a WebSocket (design D12). The display itself
//! (admission, the filter, the owner's first input, follow and lifecycle) is
//! `neo_takeover::Displays`; this module is what is HTTP and WebSocket about it.
//!
//! 1. The query is parsed: a 400 for a bad or contradictory parameter. `release=` binds a
//!    control display to its takeover (`Takeovers::binding`): a 404 when no takeover has the id,
//!    and a 409 `takeover_released` once its release has begun, so a host learns it from the
//!    status and not from a socket that opens and closes with 4002.
//! 2. The request must be a WebSocket upgrade.
//! 3. `Displays::admit` runs before the upgrade, so a refusal is an HTTP status and not a socket
//!    that opens and dies: 409 `display_paused` for a view while the owner has control or a
//!    private tab is in front (decision 34), 503 `browser_unavailable` when the browser cannot
//!    list its tabs for a view to check the front, 503 `display_unavailable` when Xvnc cannot be reached.
//! 4. The upgrade takes no subprotocol, and echoes `binary` only when the viewer offers it:
//!    noVNC 1.3.0 offers none, so requiring one would lock it out (decision 26).
//! 5. Binary frames carry RFB bytes both ways, and the display ends with a close that says why:
//!
//! | Code | When |
//! |---|---|
//! | 4001 `paused` | a view, when the owner takes control or a private tab is in front: try again later |
//! | 4002 `released` | a control display, when its takeover is released |
//! | 1008 | the viewer sent a message the filter refuses (the reason says which) |
//! | 1003 | the viewer sent a text frame |
//! | 1011 | Xvnc closed the connection |
//! | none | the viewer left |
//!
//! Browsers cannot set `Authorization` on a WebSocket, and a token in the URL is refused, so a
//! browser's noVNC reaches this route through zuzu's relay, which sends the header.

use crate::api_error::ApiError;
use crate::limits::{CLOSE_FRAME_WAIT, DISPLAY_MESSAGE_MAX};
use crate::query::{self, DisplayQuery};
use crate::service::ServiceState;
use axum::body::Bytes;
use axum::extract::ws::rejection::WebSocketUpgradeRejection;
use axum::extract::ws::{CloseFrame, Message, Utf8Bytes, WebSocket, WebSocketUpgrade, close_code};
use axum::extract::{RawQuery, State};
use axum::response::{IntoResponse, Response};
use futures_util::{SinkExt, StreamExt};
use neo_takeover::{Admitted, DisplayEnd, DisplayRequest};
use std::future::ready;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// A view closed because the owner has control or a private tab is in front. RFC 6455 leaves
/// 4000 to 4999 to applications.
const PAUSED: u16 = 4001;
/// A bound control display closed because its takeover was released.
const RELEASED: u16 = 4002;

/// Opens a display: parses, admits, then upgrades.
///
/// # Errors
/// 400 for a bad query, 404 for a `release` that names no takeover, 409 `takeover_released`
/// for one whose release has begun, 409 `display_paused` for a view while the owner has
/// control or a private tab is in front, 503 `browser_unavailable` when the browser cannot list
/// its tabs, and 503 `display_unavailable` when Xvnc is unreachable.
pub(crate) async fn open_display(
    State(state): State<Arc<ServiceState>>,
    RawQuery(raw): RawQuery,
    upgrade: Result<WebSocketUpgrade, WebSocketUpgradeRejection>,
) -> Result<Response, ApiError> {
    let request = match query::display(raw.as_deref())? {
        DisplayQuery::View(follow) => DisplayRequest::View { follow },
        DisplayQuery::Control(None) => DisplayRequest::Control { binding: None },
        DisplayQuery::Control(Some(takeover)) => DisplayRequest::Control {
            binding: Some(state.takeovers.binding(takeover)?),
        },
    };
    // Checked before admission, so a request that cannot become a WebSocket never registers a
    // display or dials Xvnc. Axum's status (400, 405 or 426) gets the contract's error body
    // from the request-id stamp.
    let upgrade = match upgrade {
        Ok(upgrade) => upgrade,
        Err(rejection) => return Ok(rejection.into_response()),
    };
    let admitted = state.displays.admit(request).await?;
    Ok(upgrade
        .protocols(["binary"])
        .max_message_size(DISPLAY_MESSAGE_MAX)
        .max_frame_size(DISPLAY_MESSAGE_MAX)
        .on_failed_upgrade(|error| tracing::debug!(%error, "a display's upgrade failed"))
        .on_upgrade(move |socket| relay(admitted, socket)))
}

/// One display's life on its WebSocket: binary frames in are RFB bytes for the display, its RFB
/// bytes go out as binary frames, and its end becomes the close frame.
async fn relay(admitted: Admitted, socket: WebSocket) {
    let (mut to_socket, mut from_socket) = socket.split();
    let text_frame = AtomicBool::new(false);
    let from_viewer = (&mut from_socket)
        .take_while(|message| {
            let more = match message {
                Ok(Message::Binary(_) | Message::Ping(_) | Message::Pong(_)) => true,
                Ok(Message::Text(_)) => {
                    text_frame.store(true, Ordering::Relaxed);
                    false
                }
                Ok(Message::Close(_)) | Err(_) => false,
            };
            ready(more)
        })
        .filter_map(|message| {
            ready(match message {
                Ok(Message::Binary(bytes)) => Some(bytes),
                _ => None,
            })
        });
    let to_viewer =
        (&mut to_socket).with(|bytes: Bytes| ready(Ok::<_, axum::Error>(Message::Binary(bytes))));
    let end = admitted.serve(from_viewer, to_viewer).await;
    let close = if text_frame.load(Ordering::Relaxed) {
        Some(CloseFrame {
            code: close_code::UNSUPPORTED,
            reason: Utf8Bytes::from_static("Neo's display takes binary frames only"),
        })
    } else {
        close_frame(end)
    };
    // Bounded, as a viewer that stopped reading would otherwise hold this task open. A viewer
    // that closed first is answered by closing the sink: tungstenite queued the answer to its
    // close frame when it read it, and refuses a close frame of our own after theirs.
    let closed = match close {
        Some(frame) => {
            let sent = to_socket.send(Message::Close(Some(frame)));
            tokio::time::timeout(CLOSE_FRAME_WAIT, sent).await
        }
        None => tokio::time::timeout(CLOSE_FRAME_WAIT, to_socket.close()).await,
    };
    if let Ok(Err(error)) = closed {
        tracing::debug!(%error, "a display's viewer was gone before its close frame");
    }
}

/// The close a viewer gets for how its display ended; `None` for a viewer that left.
fn close_frame(end: DisplayEnd) -> Option<CloseFrame> {
    let (code, reason) = match end {
        DisplayEnd::ViewerLeft => return None,
        DisplayEnd::Paused => (
            PAUSED,
            "paused: the owner has control or a private tab is in front; try again later",
        ),
        DisplayEnd::Released => (RELEASED, "released: the takeover ended"),
        DisplayEnd::Refused(why) => (close_code::POLICY, why),
        DisplayEnd::ScreenClosed => (close_code::ERROR, "Neo's screen closed the connection"),
    };
    Some(CloseFrame {
        code,
        reason: Utf8Bytes::from_static(reason),
    })
}
