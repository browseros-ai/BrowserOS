//! Live views of the tabs an agent session is working in, behind the hosted `/v1/sessions/{id}`
//! `/screencast` (MJPEG) and `/video` (fragmented AV1 MP4) endpoints.
//!
//! ```text
//!  viewer ─┐                      ┌─ Capture(tab A, screencast, high) ── Page.startScreencast ──────┐
//!  viewer ─┼─ CaptureHub ─────────┤                                                                ├─ Link ─ Chromium
//!  viewer ─┘  validate, follow,   └─ Capture(tab A, video, high) ─────── Page.startScreenRecording ─┘  (own CDP websocket)
//!             end with the session
//! ```
//!
//! Capture runs only while someone watches: the first viewer of a (tab, kind, quality) starts it,
//! later viewers share it, and it stops a few seconds after the last one leaves. Only the
//! session's own open tabs are offered. Those are the tabs Chromium keeps rendering in the
//! background (the agent turns on focus emulation for them), so they produce frames even when
//! they are not the window's active tab.

mod link;
mod mp4;
mod ports;
mod presets;
mod tab_capture;
#[doc(hidden)]
pub mod testing;

pub use ports::{
    CaptureTab, CaptureTransport, CdpPortTransport, LiveSessionTabs, SessionTabs, SessionView,
};
pub use presets::Quality;

use crate::error::AppError;
use bytes::Bytes;
use futures_util::Stream;
use link::Link;
use std::{
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
    time::Duration,
};
use tab_capture::{CaptureKey, Kind, Lease, Registry};
use tokio::sync::{broadcast, mpsc};
use tokio_util::sync::CancellationToken;

/// How often an open stream re-reads its session: to follow the current tab, to notice a closed
/// tab or an ended session, and to refresh the URL it reports.
const SESSION_CHECK_INTERVAL: Duration = Duration::from_secs(1);
/// Failed attempts to move a screencast whose tab is gone before the stream gives up. A closed
/// tab can stay ranked first until its claim is released, so a few retries ride that out.
const SWITCH_ATTEMPTS: u32 = 5;

/// Why a live view could not be opened.
#[derive(Debug, thiserror::Error)]
pub enum CaptureError {
    #[error("session not found")]
    SessionNotFound,
    #[error("session has ended")]
    SessionNotLive,
    #[error("session has no open tab")]
    NoOpenTab,
    #[error("tab is not open in this session")]
    TabNotFound,
    #[error(transparent)]
    Browser(#[from] AppError),
}

/// One screencast frame as a viewer receives it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    pub jpeg: Bytes,
    /// The browser tab the frame shows.
    pub tab_id: i64,
    pub url: String,
    /// When the browser captured it, Unix epoch ms.
    pub captured_at: i64,
}

/// A live screencast; it ends when the session ends, its pinned tab closes, or the server shuts
/// down.
pub struct Frames(mpsc::Receiver<Frame>);

impl Stream for Frames {
    type Item = Frame;
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Frame>> {
        self.0.poll_recv(cx)
    }
}

/// A live fragmented MP4: the initialization segment first, then keyframe-led fragments. It
/// ends when its tab closes, the session ends, or the server shuts down.
pub struct VideoChunks(mpsc::Receiver<Bytes>);

impl Stream for VideoChunks {
    type Item = Bytes;
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Bytes>> {
        self.0.poll_recv(cx)
    }
}

pub struct CaptureHub {
    tabs: Arc<dyn SessionTabs>,
    transport: Arc<dyn CaptureTransport>,
    shutdown: CancellationToken,
    link: tokio::sync::Mutex<Option<Arc<Link>>>,
    captures: Registry,
}

impl CaptureHub {
    /// `shutdown` ends every open stream and capture.
    #[must_use]
    pub fn new(
        tabs: Arc<dyn SessionTabs>,
        transport: Arc<dyn CaptureTransport>,
        shutdown: CancellationToken,
    ) -> Arc<Self> {
        Arc::new(Self {
            tabs,
            transport,
            shutdown,
            link: tokio::sync::Mutex::new(None),
            captures: Registry::default(),
        })
    }

    /// Streams JPEG frames of the session's current tab, following it as the agent moves to
    /// another tab, or of `tab` alone when given. Frames come only when the page changes, at
    /// most at the quality's frame rate; a new viewer first gets the newest frame already seen.
    pub async fn screencast(
        self: &Arc<Self>,
        session_id: &str,
        quality: Quality,
        tab: Option<i64>,
    ) -> Result<Frames, CaptureError> {
        let view = self.live_view(session_id).await?;
        let current = pick(&view, tab)?;
        let lease = self.lease(&current, Kind::Screencast, quality).await?;
        let (tx, rx) = mpsc::channel(1);
        tokio::spawn(
            self.clone()
                .follow(session_id.to_string(), quality, tab, current, lease, tx),
        );
        Ok(Frames(rx))
    }

    /// Streams `tab`, or the session's current tab, as fragmented AV1 MP4. The video stays on
    /// that tab: switching tabs would mean restarting the viewer's decoder.
    pub async fn video(
        self: &Arc<Self>,
        session_id: &str,
        quality: Quality,
        tab: Option<i64>,
    ) -> Result<VideoChunks, CaptureError> {
        let view = self.live_view(session_id).await?;
        let target = pick(&view, tab)?;
        let lease = self.lease(&target, Kind::Video, quality).await?;
        let (tx, rx) = mpsc::channel(2);
        tokio::spawn(self.clone().play(session_id.to_string(), target, lease, tx));
        Ok(VideoChunks(rx))
    }

    async fn live_view(&self, session_id: &str) -> Result<SessionView, CaptureError> {
        match self.tabs.view(session_id).await? {
            None => Err(CaptureError::SessionNotFound),
            Some(view) if !view.live => Err(CaptureError::SessionNotLive),
            Some(view) if view.tabs.is_empty() => Err(CaptureError::NoOpenTab),
            Some(view) => Ok(view),
        }
    }

    /// A running capture of `tab`, started if needed.
    async fn lease(
        &self,
        tab: &CaptureTab,
        kind: Kind,
        quality: Quality,
    ) -> Result<Lease, CaptureError> {
        let key = CaptureKey {
            target_id: tab.target_id.clone(),
            kind,
            quality,
        };
        let lease = match tab_capture::join(&self.captures, &key) {
            Some(lease) => lease,
            None => {
                let link = self.link().await?;
                tab_capture::join_or_start(&self.captures, key, link, presets::preset(quality))
            }
        };
        lease
            .ready()
            .await
            .map_err(|error| CaptureError::Browser(AppError::Internal(error)))?;
        Ok(lease)
    }

    /// The capture connection, reopened after it broke.
    async fn link(&self) -> Result<Arc<Link>, CaptureError> {
        let mut link = self.link.lock().await;
        if let Some(current) = link.as_ref().filter(|link| !link.broken.is_cancelled()) {
            return Ok(current.clone());
        }
        let opened = Link::open(self.transport.as_ref(), self.shutdown.clone()).await?;
        *link = Some(opened.clone());
        Ok(opened)
    }

    /// Forwards one viewer's screencast, moving it to the session's current tab as that
    /// changes. Dropping the lease on a switch or an exit is what lets an unwatched capture
    /// stop.
    async fn follow(
        self: Arc<Self>,
        session_id: String,
        quality: Quality,
        pinned: Option<i64>,
        mut tab: CaptureTab,
        mut lease: Lease,
        tx: mpsc::Sender<Frame>,
    ) {
        let mut frames = lease.frames();
        let mut check = tokio::time::interval(SESSION_CHECK_INTERVAL);
        check.reset();
        let mut failed_switches = 0;
        loop {
            enum Next {
                Frame,
                Check,
                Ended,
                Stop,
            }
            let next = tokio::select! {
                () = self.shutdown.cancelled() => Next::Stop,
                () = tx.closed() => Next::Stop,
                () = lease.capture.ended.cancelled() => Next::Ended,
                changed = frames.changed() => if changed.is_ok() { Next::Frame } else { Next::Ended },
                _ = check.tick() => Next::Check,
            };
            match next {
                Next::Stop => return,
                Next::Frame => {
                    let Some(raw) = frames.borrow_and_update().clone() else {
                        continue;
                    };
                    let frame = Frame {
                        jpeg: raw.jpeg.clone(),
                        tab_id: tab.tab_id,
                        url: tab.url.clone(),
                        captured_at: raw.captured_at,
                    };
                    if tx.send(frame).await.is_err() {
                        return;
                    }
                }
                Next::Ended | Next::Check => {
                    // A pinned tab's capture ending means the tab closed (or the link broke).
                    if matches!(next, Next::Ended) && pinned.is_some() {
                        return;
                    }
                    let Some(next_tab) = self.next_tab(&session_id, pinned).await else {
                        return;
                    };
                    if next_tab.target_id != tab.target_id || lease.capture.ended.is_cancelled() {
                        match self.lease(&next_tab, Kind::Screencast, quality).await {
                            Ok(next_lease) => {
                                lease = next_lease;
                                frames = lease.frames();
                                failed_switches = 0;
                            }
                            Err(error) => {
                                tracing::debug!(%error, "screencast could not switch tabs");
                                if lease.capture.ended.is_cancelled() {
                                    // Nothing to show until the switch works; wait a beat so
                                    // the ended capture does not spin this loop.
                                    failed_switches += 1;
                                    if failed_switches >= SWITCH_ATTEMPTS {
                                        return;
                                    }
                                    check.tick().await;
                                }
                                continue;
                            }
                        }
                    }
                    tab = next_tab;
                }
            }
        }
    }

    /// Forwards one viewer's video: the init segment, then fragments until the tab closes.
    async fn play(
        self: Arc<Self>,
        session_id: String,
        tab: CaptureTab,
        lease: Lease,
        tx: mpsc::Sender<Bytes>,
    ) {
        let (mut init, mut fragments) = lease.video();
        let mut check = tokio::time::interval(SESSION_CHECK_INTERVAL);
        check.reset();
        let init_segment = loop {
            if let Some(init) = init.borrow_and_update().clone() {
                break init;
            }
            tokio::select! {
                () = self.shutdown.cancelled() => return,
                () = tx.closed() => return,
                () = lease.capture.ended.cancelled() => return,
                changed = init.changed() => if changed.is_err() { return },
                _ = check.tick() => if !self.still_open(&session_id, tab.tab_id).await { return },
            }
        };
        if tx.send(init_segment).await.is_err() {
            return;
        }
        loop {
            tokio::select! {
                () = self.shutdown.cancelled() => return,
                () = tx.closed() => return,
                () = lease.capture.ended.cancelled() => return,
                _ = check.tick() => if !self.still_open(&session_id, tab.tab_id).await { return },
                fragment = fragments.recv() => match fragment {
                    Ok(fragment) => if tx.send(fragment).await.is_err() { return },
                    // Every fragment starts on a keyframe, so skipping ahead keeps the video valid.
                    Err(broadcast::error::RecvError::Lagged(_)) => {}
                    Err(broadcast::error::RecvError::Closed) => return,
                },
            }
        }
    }

    /// The tab a stream should show now, or `None` when it should end.
    async fn next_tab(&self, session_id: &str, pinned: Option<i64>) -> Option<CaptureTab> {
        let view = match self.tabs.view(session_id).await {
            Ok(Some(view)) if view.live => view,
            Ok(_) => return None,
            Err(error) => {
                tracing::debug!(%error, "live view session check failed");
                return None;
            }
        };
        match pinned {
            Some(tab_id) => view.tabs.into_iter().find(|tab| tab.tab_id == tab_id),
            None => view.tabs.into_iter().next(),
        }
    }

    async fn still_open(&self, session_id: &str, tab_id: i64) -> bool {
        self.next_tab(session_id, Some(tab_id)).await.is_some()
    }
}

fn pick(view: &SessionView, tab: Option<i64>) -> Result<CaptureTab, CaptureError> {
    match tab {
        Some(tab_id) => view
            .tabs
            .iter()
            .find(|tab| tab.tab_id == tab_id)
            .cloned()
            .ok_or(CaptureError::TabNotFound),
        None => view.tabs.first().cloned().ok_or(CaptureError::NoOpenTab),
    }
}

#[cfg(test)]
mod tests;
