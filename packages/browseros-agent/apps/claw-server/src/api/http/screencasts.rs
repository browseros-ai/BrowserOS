//! `/api/v1/screencasts`: the cockpit's live view of its running sessions, one WebSocket for every
//! card on the page. A stream per card would hold one of Chrome's six connections to this server
//! each, and enough running sessions would stall every other request the cockpit makes. What is
//! captured, followed, retried and stopped is the [`Watcher`]'s business; this module only speaks
//! the protocol.
//!
//! - Client → server, text: a `ScreencastWatchMessage`, e.g.
//!   `{"watch":[{"sessionId":"s1"},{"sessionId":"s2","browserTabId":12}]}`, the whole set
//!   whenever it changes; each message replaces the previous set. A message that doesn't parse
//!   is ignored.
//! - Server → client, binary, one frame per message: a big-endian `u32` header length, a
//!   `ScreencastFrameHeader` as JSON, then the JPEG. The header echoes the watch exactly (no
//!   `browserTabId` when following) and names the tab the frame shows.
//!
//! Both shapes are in the claw-api contract's schemas; the route itself is not, since OpenAPI
//! has no way to describe a WebSocket.
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
use claw_api::models::{ScreencastFrameHeader, ScreencastWatchMessage};
use futures_util::{
    SinkExt, StreamExt,
    stream::{SplitSink, SplitStream},
};
use std::time::Duration;
use tokio::sync::watch;

/// How long one frame may take to reach the client before the connection is given up. Frames are
/// tens of KB over loopback, so only a client that stopped reading gets near it.
const SEND_TIMEOUT: Duration = Duration::from_secs(10);

pub(super) async fn screencasts(
    State(state): State<AppState>,
    upgrade: WebSocketUpgrade,
) -> Response {
    upgrade.on_upgrade(move |socket| serve(socket, state.capture.watcher(Quality::Low)))
}

/// Runs one socket until the client closes it, a frame can't be written, or the server shuts
/// down. Reading and writing run side by side: a client that has stopped reading frames can still
/// change its watches or close, and closing drops the writer and its watcher at once, releasing
/// every capture the socket held.
async fn serve(socket: WebSocket, watcher: Watcher) {
    let (outgoing, incoming) = socket.split();
    let (sets, wanted) = watch::channel(Vec::new());
    tokio::select! {
        () = read(incoming, sets) => {}
        () = write(outgoing, watcher, wanted) => {}
    }
}

/// Hands each watch set to the writer; only the newest set matters, since each replaces the last.
async fn read(mut incoming: SplitStream<WebSocket>, sets: watch::Sender<Vec<Watch>>) {
    while let Some(Ok(message)) = incoming.next().await {
        match message {
            Message::Text(text) => {
                if let Some(watches) = watches(text.as_str()) {
                    sets.send_replace(watches);
                }
            }
            Message::Close(_) => return,
            _ => {}
        }
    }
}

/// Sends frames as fast as the client takes them; the watcher keeps only the newest per watch
/// meanwhile. A send that stalls past [`SEND_TIMEOUT`] ends the connection rather than keeping
/// captures running for a client that is not reading; a live client reconnects.
async fn write(
    mut outgoing: SplitSink<WebSocket, Message>,
    mut watcher: Watcher,
    mut wanted: watch::Receiver<Vec<Watch>>,
) {
    loop {
        enum Next {
            Watch,
            Frame(Watch, Frame),
            Stop,
        }
        let next = tokio::select! {
            changed = wanted.changed() => if changed.is_ok() { Next::Watch } else { Next::Stop },
            frame = watcher.next() => frame.map_or(Next::Stop, |(watch, frame)| Next::Frame(watch, frame)),
        };
        match next {
            Next::Watch => watcher.set(wanted.borrow_and_update().clone()),
            Next::Frame(watch, frame) => {
                let sent = tokio::time::timeout(
                    SEND_TIMEOUT,
                    outgoing.send(Message::Binary(encode(&watch, &frame))),
                )
                .await;
                if !matches!(sent, Ok(Ok(()))) {
                    return;
                }
            }
            Next::Stop => return,
        }
    }
}

fn watches(text: &str) -> Option<Vec<Watch>> {
    let message = serde_json::from_str::<ScreencastWatchMessage>(text).ok()?;
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

fn encode(watch: &Watch, frame: &Frame) -> Bytes {
    let header = serde_json::to_vec(&ScreencastFrameHeader {
        session_id: watch.session_id.clone(),
        browser_tab_id: watch.tab,
        tab_id: frame.tab_id,
        url: frame.url.clone(),
        captured_at: frame.captured_at,
    })
    .unwrap_or_default();
    let mut message = BytesMut::with_capacity(4 + header.len() + frame.jpeg.len());
    message.put_u32(u32::try_from(header.len()).unwrap_or(u32::MAX));
    message.put_slice(&header);
    message.put_slice(&frame.jpeg);
    message.freeze()
}
