//! `/api/v1/screencasts`: the cockpit's live view of its running sessions, one WebSocket for every
//! card on the page. A stream per card would hold one of Chrome's six connections to this server
//! each, and enough running sessions would stall every other request the cockpit makes. What is
//! captured, followed, retried and stopped is the [`Watcher`]'s business; this module only speaks
//! the protocol.
//!
//! - Client → server, text: `{"watch":[{"sessionId":"s1"},{"sessionId":"s2","browserTabId":12}]}`,
//!   the whole set whenever it changes; each message replaces the previous set. A message that
//!   doesn't parse is ignored.
//! - Server → client, binary, one frame per message: a big-endian `u32` header length, a JSON
//!   header `{"sessionId","browserTabId"?,"tabId","url","capturedAt"}`, then the JPEG.
//!   `sessionId` and `browserTabId` echo the watch exactly (no `browserTabId` when following);
//!   `tabId` is the tab the frame shows and `capturedAt` is Unix epoch ms.
//!
//! Frames are `low` quality. Only claw-app (or a native client) may connect: `request_context`
//! checks the origin before the upgrade.

use crate::{
    AppState,
    services::capture::{Frame, Quality, Watch, Watcher},
};
use axum::{
    extract::{
        State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    response::Response,
};
use bytes::{BufMut, Bytes, BytesMut};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};

pub(super) async fn screencasts(
    State(state): State<AppState>,
    upgrade: WebSocketUpgrade,
) -> Response {
    upgrade.on_upgrade(move |socket| serve(socket, state.capture.watcher(Quality::Low)))
}

/// Runs one socket until either side closes it or the server shuts down. The watcher lives
/// exactly as long as the socket, so a cockpit that goes away releases its captures.
async fn serve(socket: WebSocket, mut watcher: Watcher) {
    let (mut outgoing, mut incoming) = socket.split();
    loop {
        enum Next {
            Watch(Vec<Watch>),
            Frame(Watch, Frame),
            Skip,
            Stop,
        }
        let next = tokio::select! {
            message = incoming.next() => match message {
                Some(Ok(Message::Text(text))) => watches(text.as_str()).map_or(Next::Skip, Next::Watch),
                Some(Ok(Message::Close(_)) | Err(_)) | None => Next::Stop,
                Some(Ok(_)) => Next::Skip,
            },
            frame = watcher.next() => frame.map_or(Next::Stop, |(watch, frame)| Next::Frame(watch, frame)),
        };
        match next {
            Next::Watch(watches) => watcher.set(watches),
            Next::Frame(watch, frame) => {
                if outgoing
                    .send(Message::Binary(encode(&watch, &frame)))
                    .await
                    .is_err()
                {
                    return;
                }
            }
            Next::Skip => {}
            Next::Stop => return,
        }
    }
}

#[derive(Deserialize)]
struct WatchMessage {
    watch: Vec<WatchEntry>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WatchEntry {
    session_id: String,
    browser_tab_id: Option<i64>,
}

fn watches(text: &str) -> Option<Vec<Watch>> {
    let message = serde_json::from_str::<WatchMessage>(text).ok()?;
    Some(
        message
            .watch
            .into_iter()
            .map(|entry| Watch {
                session_id: entry.session_id,
                tab: entry.browser_tab_id,
            })
            .collect(),
    )
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FrameHeader<'a> {
    session_id: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    browser_tab_id: Option<i64>,
    tab_id: i64,
    url: &'a str,
    captured_at: i64,
}

fn encode(watch: &Watch, frame: &Frame) -> Bytes {
    let header = serde_json::to_vec(&FrameHeader {
        session_id: &watch.session_id,
        browser_tab_id: watch.tab,
        tab_id: frame.tab_id,
        url: &frame.url,
        captured_at: frame.captured_at,
    })
    .unwrap_or_default();
    let mut message = BytesMut::with_capacity(4 + header.len() + frame.jpeg.len());
    message.put_u32(u32::try_from(header.len()).unwrap_or(u32::MAX));
    message.put_slice(&header);
    message.put_slice(&frame.jpeg);
    message.freeze()
}
