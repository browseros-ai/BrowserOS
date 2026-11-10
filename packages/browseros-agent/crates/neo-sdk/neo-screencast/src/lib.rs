//! Neo: shared live capture of browser tabs on a capture-owned CDP connection.
//!
//! `SessionTabs` adapts host identity and eligibility. Legacy claw-server supplies claimed
//! session tabs; neo-http supplies one page and owns its persistent rendering presence.
//! The hub shares one capture per target, kind and quality, follows eligible tabs, and releases
//! captures after the last viewer's grace period. Capture sessions never enable emulation.
//! Tracing targets are part of the host's log-filter contract and keep their historical module names.
//!
//! ```text
//! viewer -> CaptureHub -> capture lease -> target session -> Link -> browser
//!                 ^                           own socket       ^
//!             SessionTabs                             CaptureTransport
//! ```

mod clock;
mod connection;
mod failure;
mod link;
pub mod mjpeg;
pub use connection::CaptureConnection;
pub use failure::{CaptureFailure, CaptureResult};
pub use link::{Attached, Link};
mod mp4;
mod ports;
mod presets;
mod tab_capture;
#[doc(hidden)]
pub mod testing;
mod watcher;

pub use ports::{CaptureTab, CaptureTransport, CdpPortTransport, SessionTabs, SessionView};
pub use presets::Quality;
pub use watcher::{Watch, Watcher};

use bytes::Bytes;
use futures_util::{Stream, stream};
use std::future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;
use tab_capture::{CaptureKey, Kind, Lease, RawFrame, Registry};
use tokio::sync::{Mutex as AsyncMutex, broadcast, mpsc, watch};
use tokio::time;
use tokio_util::sync::CancellationToken;

/// How often an open stream re-reads its session: to follow the current tab, to notice a closed
/// tab or an ended session, and to refresh the URL it reports.
/// Neo: retained capture timing, buffering or wire-format policy from the shared hub.
const SESSION_CHECK_INTERVAL: Duration = Duration::from_secs(1);
/// How long a followed screencast waits for the browser to start capturing its new tab before
/// it re-checks the session and tries again.
/// Neo: retained capture timing, buffering or wire-format policy from the shared hub.
const SWITCH_TIMEOUT: Duration = Duration::from_secs(5);
/// Video fragments queued for one viewer. One that falls further behind skips whole fragments,
/// so a viewer that stops reading never holds up its stream's shutdown or session checks.
/// Neo: retained capture timing, buffering or wire-format policy from the shared hub.
const VIDEO_QUEUE: usize = 4;

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
    Browser(#[from] CaptureFailure),
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
/// down. It holds only the newest frame: a slow reader gets the latest one when it reads,
/// skipping those it missed, so the stream behind it never waits on it.
pub struct Frames {
    stream: Pin<Box<dyn Stream<Item = Frame> + Send>>,
    generation: CancellationToken,
}

impl Frames {
    fn new(latest: watch::Receiver<Option<Frame>>, generation: CancellationToken) -> Self {
        Self {
            stream: Box::pin(stream::unfold(latest, |mut latest| async move {
                loop {
                    latest.changed().await.ok()?;
                    let frame = latest.borrow_and_update().clone();
                    if let Some(frame) = frame {
                        return Some((frame, latest));
                    }
                }
            })),
            generation,
        }
    }

    /// Ends when the initial capture connection generation breaks. The stream itself keeps
    /// following/retrying for legacy consumers; HTTP responses that must reconnect opt into
    /// this signal. A child token cannot cancel the shared connection or other viewers.
    #[must_use = "observe this signal to end a generation-bound response"]
    pub fn generation_ended(&self) -> CancellationToken {
        self.generation.child_token()
    }
}

impl Stream for Frames {
    type Item = Frame;
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Frame>> {
        self.stream.as_mut().poll_next(cx)
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

/// Owns capture sharing, tab following and viewer lifecycle on an independent CDP socket.
pub struct CaptureHub {
    tabs: Arc<dyn SessionTabs>,
    transport: Arc<dyn CaptureTransport>,
    shutdown: CancellationToken,
    link: AsyncMutex<Option<Arc<Link>>>,
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
            link: AsyncMutex::new(None),
            captures: Registry::default(),
        })
    }

    /// Streams JPEG frames of the session's current tab, following it as the agent moves to
    /// another tab, or of `tab` alone when given. Frames come only when the page changes, at
    /// most at the quality's frame rate; a new viewer first gets the newest frame already seen.
    ///
    /// # Errors
    /// Returns an unavailable session/tab or a browser failure while starting the capture.
    pub async fn screencast(
        self: &Arc<Self>,
        session_id: &str,
        quality: Quality,
        tab: Option<i64>,
    ) -> Result<Frames, CaptureError> {
        let view = self.live_view(session_id).await?;
        let current = pick(&view, tab)?;
        let lease = self.lease(&current, Kind::Screencast, quality).await?;
        let generation = lease.capture.generation.clone();
        let (latest, frames) = watch::channel(None);
        tokio::spawn(Arc::clone(self).follow(
            session_id.to_string(),
            quality,
            tab,
            current,
            lease,
            latest,
        ));
        Ok(Frames::new(frames, generation))
    }

    /// One viewer's live view of any number of sessions, all at `quality`: the cockpit shows
    /// every running session over one socket rather than a stream per card.
    #[must_use]
    pub fn watcher(self: &Arc<Self>, quality: Quality) -> Watcher {
        Watcher::new(Arc::clone(self), quality)
    }

    /// Streams `tab`, or the session's current tab, as fragmented AV1 MP4. The video stays on
    /// that tab: switching tabs would mean restarting the viewer's decoder.
    ///
    /// # Errors
    /// Returns an unavailable session/tab or a browser failure while starting the capture.
    pub async fn video(
        self: &Arc<Self>,
        session_id: &str,
        quality: Quality,
        tab: Option<i64>,
    ) -> Result<VideoChunks, CaptureError> {
        let view = self.live_view(session_id).await?;
        let target = pick(&view, tab)?;
        let lease = self.lease(&target, Kind::Video, quality).await?;
        let (tx, rx) = mpsc::channel(VIDEO_QUEUE);
        tokio::spawn(Arc::clone(self).play(session_id.to_string(), target, lease, tx));
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
        let lease = if let Some(lease) = tab_capture::join(&self.captures, &key) {
            lease
        } else {
            let link = self.link().await?;
            tab_capture::join_or_start(&self.captures, key, link, presets::preset(quality))
        };
        lease
            .ready()
            .await
            .map_err(|error| CaptureError::Browser(CaptureFailure::new(error)))?;
        Ok(lease)
    }

    /// The capture connection, reopened after it broke.
    async fn link(&self) -> Result<Arc<Link>, CaptureError> {
        let mut link = self.link.lock().await;
        if let Some(current) = link.as_ref().filter(|link| !link.broken.is_cancelled()) {
            return Ok(Arc::clone(current));
        }
        let opened = Link::open(self.transport.as_ref(), self.shutdown.clone()).await?;
        *link = Some(Arc::clone(&opened));
        Ok(opened)
    }

    /// Forwards one viewer's screencast, moving it to the session's current tab as that
    /// changes, and waiting through moments when the session has no tab. Nothing here waits on
    /// the viewer: frames replace the newest one, so shutdown and session checks always run.
    /// Dropping a lease on a switch or an exit is what lets an unwatched capture stop.
    async fn follow(
        self: Arc<Self>,
        session_id: String,
        quality: Quality,
        pinned: Option<i64>,
        tab: CaptureTab,
        lease: Lease,
        latest: watch::Sender<Option<Frame>>,
    ) {
        let mut showing = Some(Showing::new(tab, lease));
        let mut check = time::interval(SESSION_CHECK_INTERVAL);
        check.reset();
        loop {
            enum Next {
                Frame,
                Check,
                Stop,
            }
            let next = tokio::select! {
                () = self.shutdown.cancelled() => Next::Stop,
                () = latest.closed() => Next::Stop,
                activity = activity(showing.as_mut()) => match activity {
                    Activity::Frame => Next::Frame,
                    // The tab closed or the link broke: decide now instead of at the next tick.
                    Activity::Ended => Next::Check,
                },
                _ = check.tick() => Next::Check,
            };
            match next {
                Next::Stop => return,
                Next::Frame => {
                    if let Some(showing) = showing.as_mut() {
                        let raw = showing.frames.borrow_and_update().clone();
                        if let Some(raw) = raw {
                            latest.send_replace(Some(showing.frame(&raw)));
                        }
                    }
                }
                Next::Check => match self.resolve(&session_id, pinned).await {
                    Resolved::End => {
                        if let Some(showing) = showing.take() {
                            showing.lease.end_now();
                        }
                        return;
                    }
                    Resolved::Wait => {
                        if showing.as_ref().is_some_and(Showing::ended) {
                            showing = None;
                        }
                    }
                    Resolved::Show(next_tab) => {
                        if let Some(current) = showing.as_mut().filter(|current| {
                            !current.ended() && current.tab.target_id == next_tab.target_id
                        }) {
                            // Same tab; pick up its latest URL.
                            current.tab = next_tab;
                            continue;
                        }
                        // Starting a capture waits on the browser. Stay responsive to the viewer
                        // leaving and to shutdown meanwhile, and give up after a while so the
                        // session check runs again.
                        let switched = tokio::select! {
                            () = self.shutdown.cancelled() => return,
                            () = latest.closed() => return,
                            switched = time::timeout(
                                SWITCH_TIMEOUT,
                                self.lease(&next_tab, Kind::Screencast, quality),
                            ) => switched,
                        };
                        match switched {
                            Ok(Ok(lease)) => showing = Some(Showing::new(next_tab, lease)),
                            // Retried at the next check; until then keep the old tab if it lives.
                            Ok(Err(error)) => {
                                tracing::debug!(target: "neo_capture", %error, "screencast could not switch tabs");
                                if showing.as_ref().is_some_and(Showing::ended) {
                                    showing = None;
                                }
                            }
                            Err(_) => {
                                tracing::debug!(target: "neo_capture", "screencast tab switch timed out");
                                if showing.as_ref().is_some_and(Showing::ended) {
                                    showing = None;
                                }
                            }
                        }
                    }
                },
            }
        }
    }

    /// Forwards one viewer's video: the init segment, then fragments until the tab closes.
    /// Sends never wait: a full queue skips the fragment, which is safe because every fragment
    /// starts on a keyframe.
    async fn play(
        self: Arc<Self>,
        session_id: String,
        tab: CaptureTab,
        lease: Lease,
        tx: mpsc::Sender<Bytes>,
    ) {
        let (mut init, mut fragments) = lease.video();
        let mut check = time::interval(SESSION_CHECK_INTERVAL);
        check.reset();
        let mut started = false;
        loop {
            enum Next {
                Init,
                Fragment(Result<Bytes, broadcast::error::RecvError>),
                Check,
                Stop,
            }
            let next = tokio::select! {
                () = self.shutdown.cancelled() => Next::Stop,
                () = tx.closed() => Next::Stop,
                () = lease.capture.ended.cancelled() => Next::Stop,
                changed = init.changed(), if !started => {
                    if changed.is_ok() { Next::Init } else { Next::Stop }
                }
                fragment = fragments.recv(), if started => Next::Fragment(fragment),
                _ = check.tick() => Next::Check,
            };
            match next {
                Next::Stop | Next::Fragment(Err(broadcast::error::RecvError::Closed)) => return,
                Next::Init => {
                    let Some(segment) = init.borrow_and_update().clone() else {
                        continue;
                    };
                    // The queue is empty before the first send, so this cannot be full.
                    if tx.try_send(segment).is_err() {
                        return;
                    }
                    started = true;
                }
                Next::Fragment(Ok(fragment)) => match tx.try_send(fragment) {
                    Ok(()) | Err(mpsc::error::TrySendError::Full(_)) => {}
                    Err(mpsc::error::TrySendError::Closed(_)) => return,
                },
                Next::Fragment(Err(broadcast::error::RecvError::Lagged(_))) => {}
                Next::Check => {
                    if matches!(
                        self.resolve(&session_id, Some(tab.tab_id)).await,
                        Resolved::End
                    ) {
                        lease.end_now();
                        return;
                    }
                }
            }
        }
    }

    /// What a stream should show now. A lookup error is treated as "wait": a passing failure
    /// to read the browser's tabs should not end a partner's stream.
    async fn resolve(&self, session_id: &str, pinned: Option<i64>) -> Resolved {
        let view = match self.tabs.view(session_id).await {
            Ok(Some(view)) if view.live => view,
            Ok(_) => return Resolved::End,
            Err(error) => {
                tracing::debug!(target: "neo_capture", %error, "live view session check failed");
                return Resolved::Wait;
            }
        };
        match pinned {
            Some(tab_id) => view
                .tabs
                .into_iter()
                .find(|tab| tab.tab_id == tab_id)
                .map_or(Resolved::End, Resolved::Show),
            // A live session between tabs: the agent closed one before opening the next.
            None => view
                .tabs
                .into_iter()
                .next()
                .map_or(Resolved::Wait, Resolved::Show),
        }
    }
}

enum Resolved {
    Show(CaptureTab),
    /// Keep the stream open: the session is live but has nothing to show right now.
    Wait,
    /// Close the stream: the session ended, or its pinned tab closed.
    End,
}

/// The tab a screencast stream shows, and its hold on that tab's capture.
struct Showing {
    tab: CaptureTab,
    lease: Lease,
    frames: watch::Receiver<Option<Arc<RawFrame>>>,
}

impl Showing {
    fn new(tab: CaptureTab, lease: Lease) -> Self {
        let frames = lease.frames();
        Self { tab, lease, frames }
    }

    fn ended(&self) -> bool {
        self.lease.capture.ended.is_cancelled()
    }

    fn frame(&self, raw: &RawFrame) -> Frame {
        Frame {
            jpeg: raw.jpeg.clone(),
            tab_id: self.tab.tab_id,
            url: self.tab.url.clone(),
            captured_at: raw.captured_at,
        }
    }
}

enum Activity {
    Frame,
    Ended,
}

/// Resolves when the shown capture produces a frame or ends. Never resolves while nothing is
/// shown or once its end was seen; the next check then replaces or drops it.
async fn activity(showing: Option<&mut Showing>) -> Activity {
    let Some(showing) = showing.filter(|showing| !showing.ended()) else {
        return future::pending().await;
    };
    tokio::select! {
        () = showing.lease.capture.ended.cancelled() => Activity::Ended,
        changed = showing.frames.changed() => {
            if changed.is_ok() { Activity::Frame } else { Activity::Ended }
        }
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
