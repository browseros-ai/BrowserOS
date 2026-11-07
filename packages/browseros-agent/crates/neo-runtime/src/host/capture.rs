//! Live views of the tabs a session works in: an MJPEG screencast that follows the session's
//! current tab, and a fragmented AV1 video that stays on one tab. Ported from claw-server's
//! `CaptureHub` (`apps/claw-server/src/services/capture/mod.rs:1-492`, origin/main) with its
//! tested rules: a capture per (tab, kind, quality) runs only while watched, is shared by every
//! viewer of that key, is ack-paced, and stops `STOP_GRACE` after its last viewer leaves, or at
//! once when the session ends (`capture/lease.rs`).
//!
//! ```text
//!  viewer ─┐               ┌─ Capture(tab, screencast, quality): Page.startScreencast ─┐
//!  viewer ─┼─ CaptureHub ──┤                                                          ├─ HostLink
//!  viewer ─┘  pick, follow └─ Capture(tab, video, quality): Page.startScreenRecording ┘
//!             end with the view   (a session each; the tab's presence keeps it painting)
//! ```
//!
//! What changed from claw-server:
//! - A follower reads the session's [`FollowView`] from the `watch` the ledger publishes,
//!   instead of re-reading the session every second per stream. It wakes on a view change, on
//!   its capture's frames or end, and on a new socket generation; only a failed switch retries
//!   on a timer.
//! - Before a capture of a tab starts, the tab's presence (`presence.rs`) turns focus emulation
//!   on, which keeps a background tab painting. Capture sessions send no `Emulation.*` command.
//! - A frame without the browser's frame time, or over the frame cap, is dropped and counted.
//! - A viewer that leaves a waiting frame or fragment unread for `VIEWER_STALL` is dropped.
//! - The revocation boundary (the design's §9 point 2): a viewer's stream re-reads the view
//!   right before it yields each frame or fragment, which is right before the writer starts
//!   that part. Once the ledger publishes `withheld: true`, or drops a tab from the view, no new
//!   part of it starts. Every yield also reads the shared taint set: an emitter can revoke a
//!   newly touched tab before the task consumes its event. Taint wakeups drive capture cleanup;
//!   suppression does not depend on that cleanup or a watch update being scheduled.
//!
//! Nothing on the frame path waits on a viewer: frames go Chromium → the link's route → the
//! capture's newest-frame `watch` → each follower → that viewer's newest-frame slot, so a slow
//! viewer only skips frames.

mod lease;
mod mp4;
mod presets;

#[cfg(test)]
mod e2e;
#[cfg(test)]
mod tests;

use std::collections::VecDeque;
use std::future;
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard};
use std::task::{Context, Poll};

use bytes::Bytes;
use futures_util::{Stream, stream};
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::{Notify, watch};
use tokio::time::{self, Instant};
use tokio_util::sync::CancellationToken;
use tracing::debug;

use crate::host::cdp;
use crate::host::ids::{EpochMs, TabId};
use crate::host::link::HostLink;
use crate::host::presence::Presences;
use crate::host::privacy::{Privacy, Taint};
use crate::host::view::{FollowView, ShownTab};
use crate::run::PageId;
use lease::{CaptureKey, FrameDrops, Kind, Lease, RawFrame, Registry};
pub(crate) use presets::Preset;
pub use presets::Quality;

/// Why a live view could not be opened. The `/v1` adapter maps each to the hosted contract's
/// error code; an unknown session is its own check, before it asks for a view.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum CaptureError {
    #[error("session not found")]
    SessionNotFound,
    /// → 409 `session_not_live`.
    #[error("session has ended")]
    SessionNotLive,
    /// → 409 `session_withheld`.
    #[error("session's visuals are withheld")]
    Withheld,
    /// → 409 `no_open_tab`.
    #[error("session has no open tab")]
    NoOpenTab,
    /// `?tab=` is not one of the session's shown tabs → 404 `tab_not_found`.
    #[error("tab is not open in this session")]
    TabNotFound,
    /// The browser did not start the capture → 500 `internal_error`.
    #[error("{0}")]
    Browser(String),
}

/// One screencast frame as a viewer receives it: the JPEG, the BrowserOS tab it shows
/// (`X-Tab-Id`, positive by type), that tab's URL (`X-Url`), and the browser's frame-swap time
/// (`X-Frame-At`), never the time it arrived.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    pub jpeg: Bytes,
    pub tab: TabId,
    pub url: String,
    pub captured_at: EpochMs,
}

/// A live screencast. It ends when the session ends or is withheld, its pinned tab leaves the
/// view, the viewer stalls, or the host side shuts down. It holds only the newest frame: a slow
/// reader gets the latest one when it reads, skipping those it missed.
pub struct Frames(Pin<Box<dyn Stream<Item = Frame> + Send>>);

/// A live fragmented MP4: the initialization segment first, then keyframe-led fragments. It
/// ends when its tab leaves the view, its capture ends, the session ends or is withheld, the
/// viewer stalls, or the host side shuts down.
pub struct VideoChunks(Pin<Box<dyn Stream<Item = Bytes> + Send>>);

/// Every live view of one browser, over the host side's own socket.
pub(crate) struct CaptureHub {
    link: Arc<HostLink>,
    presences: Arc<Presences>,
    pub(super) privacy: Arc<Privacy>,
    captures: Registry,
    drops: Arc<FrameDrops>,
    /// Ends every open stream and capture.
    shutdown: CancellationToken,
}

impl CaptureHub {
    pub(crate) fn new(
        link: Arc<HostLink>,
        presences: Arc<Presences>,
        privacy: Arc<Privacy>,
        shutdown: CancellationToken,
    ) -> Arc<Self> {
        Arc::new(Self {
            link,
            presences,
            privacy,
            captures: Registry::default(),
            drops: Arc::default(),
            shutdown,
        })
    }

    /// Streams JPEG frames of the view's current tab, following it as the session moves to
    /// another tab, or of `tab` alone when given. Returns once the browser started the capture,
    /// so a caller can answer before any frame arrives. A new viewer first gets the newest frame
    /// already seen.
    pub(crate) async fn screencast(
        self: &Arc<Self>,
        view: watch::Receiver<FollowView>,
        quality: Quality,
        tab: Option<TabId>,
    ) -> Result<Frames, CaptureError> {
        let taint = self.privacy.taint.changes();
        let shown = pick(&view.borrow(), tab, &self.privacy.taint)?;
        let lease = self.lease(&shown, Kind::Screencast, quality).await?;
        let viewer = Viewer::new();
        tokio::spawn(Arc::clone(self).follow(
            view.clone(),
            quality,
            tab,
            Showing::new(shown, lease),
            Arc::clone(&viewer),
            taint,
        ));
        Ok(Frames::new(viewer, view, Arc::clone(&self.privacy.taint)))
    }

    /// Streams `tab`, or the view's current tab, as fragmented AV1 MP4. The video stays on
    /// that tab: switching tabs would mean restarting the viewer's decoder.
    pub(crate) async fn video(
        self: &Arc<Self>,
        view: watch::Receiver<FollowView>,
        quality: Quality,
        tab: Option<TabId>,
    ) -> Result<VideoChunks, CaptureError> {
        let taint = self.privacy.taint.changes();
        let shown = pick(&view.borrow(), tab, &self.privacy.taint)?;
        let lease = self.lease(&shown, Kind::Video, quality).await?;
        let viewer = Viewer::new();
        tokio::spawn(Arc::clone(self).play(
            view.clone(),
            shown.tab,
            lease,
            Arc::clone(&viewer),
            taint,
        ));
        Ok(VideoChunks::new(
            viewer,
            view,
            shown.tab,
            Arc::clone(&self.privacy.taint),
        ))
    }

    /// Stops every capture of `target` at once, with no grace period. Withholding taints the
    /// tabs a session touched for every session (the ledger's `withhold` returns them), and a
    /// capture of a tainted tab must not linger for a viewer of another session. Its followers
    /// see the capture end and move on with their views, which no longer show the tab.
    pub(crate) fn end_captures(&self, target: &PageId) {
        lease::stop_target(&self.captures, target);
    }

    /// Screencast frames dropped instead of forwarded since the hub started.
    #[cfg(test)]
    pub(crate) fn dropped_frames(&self) -> &FrameDrops {
        &self.drops
    }

    /// A running capture of `tab`, started if needed. The tab's presence comes first: its focus
    /// emulation is what makes a background tab paint (probe 1). A presence that cannot be had
    /// is no reason not to try, since a foreground tab paints without one.
    async fn lease(
        &self,
        tab: &ShownTab,
        kind: Kind,
        quality: Quality,
    ) -> Result<Lease, CaptureError> {
        if self.privacy.taint.contains(&tab.target) {
            return Err(CaptureError::NoOpenTab);
        }
        if !self.link.capture_ready() {
            return Err(CaptureError::Browser(
                "the tab directory is being reseeded".to_string(),
            ));
        }
        let key = CaptureKey {
            target: tab.target.clone(),
            kind,
            quality,
        };
        let lease = if let Some(lease) = lease::join(&self.captures, &key) {
            lease
        } else {
            if let Err(error) = self.presences.ensure(&tab.target).await {
                debug!(target = tab.target.as_str(), %error, "a capture without a presence");
            }
            lease::join_or_start(
                &self.captures,
                key,
                Arc::clone(&self.link),
                presets::preset(quality),
                Arc::clone(&self.drops),
            )
            .ok_or_else(|| CaptureError::Browser("the capture registry is unusable".into()))?
        };
        lease.ready().await.map_err(CaptureError::Browser)?;
        if self.privacy.taint.contains(&tab.target) {
            lease.end_now();
            return Err(CaptureError::NoOpenTab);
        }
        Ok(lease)
    }

    /// Forwards one viewer's screencast, moving it to the view's current tab as that changes,
    /// and waiting through moments when the session has no tab. Nothing here waits on the
    /// viewer. Dropping a lease, on a switch or on exit, is what lets an unwatched capture stop.
    async fn follow(
        self: Arc<Self>,
        mut view: watch::Receiver<FollowView>,
        quality: Quality,
        pinned: Option<TabId>,
        showing: Showing,
        viewer: Arc<Viewer<Frame>>,
        mut taint: watch::Receiver<()>,
    ) {
        let mut showing = Some(showing);
        let mut generations = self.link.generations();
        let mut retry: Option<Instant> = None;
        let ended = loop {
            let event = tokio::select! {
                () = self.shutdown.cancelled() => Event::Stop,
                () = viewer.gone.cancelled() => Event::Stop,
                () = viewer.stalled() => Event::Stop,
                activity = activity(showing.as_mut()) => match activity {
                    Activity::Frame => Event::Frame,
                    // The tab closed or the socket went: decide now.
                    Activity::Ended => Event::Resolve,
                },
                changed = view.changed() => {
                    if changed.is_ok() { Event::Resolve } else { Event::End }
                }
                _ = taint.changed() => Event::Resolve,
                // A new socket: re-lease what the old one ended.
                changed = generations.changed() => {
                    if changed.is_ok() { Event::Resolve } else { Event::Stop }
                }
                () = sleep_until(retry) => Event::Resolve,
            };
            let next = match event {
                Event::Frame => {
                    if let Some(showing) = &mut showing {
                        showing.forward(&viewer);
                    }
                    continue;
                }
                Event::Resolve => {
                    self.show_current(&mut view, quality, pinned, &mut showing, &viewer)
                        .await
                }
                Event::Stop => Next::Stop,
                Event::End => Next::End,
            };
            match next {
                Next::Continue => retry = None,
                Next::Retry => retry = Some(Instant::now() + cdp::SWITCH_RETRY),
                Next::Stop => break false,
                Next::End => break true,
            }
        };
        // Leaving gives the capture its grace period, unless the newest view ends this stream:
        // then no viewer of it can come back. Judged from the view, not from which wake-up came
        // first, because a withheld view also ends the viewer's stream, so both arrive together.
        let over = ended
            || matches!(
                resolve(&view.borrow(), pinned, &self.privacy.taint),
                Resolved::End
            );
        if over && let Some(showing) = showing.take() {
            showing.lease.end_now();
        }
        viewer.close();
    }

    /// Shows what the newest view asks for: keeps the current tab (picking up its URL), switches
    /// to the new current tab within `SWITCH_TIMEOUT`, or waits for one.
    async fn show_current(
        &self,
        view: &mut watch::Receiver<FollowView>,
        quality: Quality,
        pinned: Option<TabId>,
        showing: &mut Option<Showing>,
        viewer: &Viewer<Frame>,
    ) -> Next {
        loop {
            let next = match resolve(&view.borrow_and_update(), pinned, &self.privacy.taint) {
                Resolved::Show(next) => next,
                Resolved::Wait => {
                    if let Some(old) = showing
                        .as_ref()
                        .filter(|old| self.privacy.taint.contains(&old.tab.target))
                    {
                        self.end_captures(&old.tab.target);
                    }
                    // The view shows no tab, so the old one will not be shown again: let its
                    // capture's grace period start. A quick return to it rejoins the capture.
                    *showing = None;
                    return Next::Continue;
                }
                Resolved::End => return Next::End,
            };
            if let Some(current) = showing
                .as_mut()
                .filter(|current| !current.ended() && current.tab.target == next.target)
            {
                current.tab = next;
                return Next::Continue;
            }
            // Starting a capture waits on the browser. Stay responsive to the viewer leaving,
            // to shutdown and to a newer view meanwhile.
            let switched = tokio::select! {
                () = self.shutdown.cancelled() => return Next::Stop,
                () = viewer.gone.cancelled() => return Next::Stop,
                changed = view.changed() => match changed {
                    Ok(()) => continue,
                    Err(_) => return Next::End,
                },
                switched = time::timeout(
                    cdp::SWITCH_TIMEOUT,
                    self.lease(&next, Kind::Screencast, quality),
                ) => switched,
            };
            match switched {
                Ok(Ok(lease)) => {
                    *showing = Some(Showing::new(next, lease));
                    return Next::Continue;
                }
                Ok(Err(error)) => debug!(%error, "a screencast could not switch tabs"),
                Err(_) => debug!("a screencast's tab switch timed out"),
            }
            // Until the retry, keep the old tab if it lives.
            drop_ended(showing);
            return Next::Retry;
        }
    }

    /// Forwards one viewer's video: the init segment, then fragments, until its tab leaves the
    /// view or its capture ends. Pushes never wait: a full queue skips the fragment, which is
    /// safe because every fragment starts on a keyframe.
    async fn play(
        self: Arc<Self>,
        mut view: watch::Receiver<FollowView>,
        tab: TabId,
        lease: Lease,
        viewer: Arc<Viewer<Bytes>>,
        mut taint: watch::Receiver<()>,
    ) {
        let (mut init, mut fragments) = lease.video();
        let mut started = false;
        let ended = loop {
            let event = tokio::select! {
                () = self.shutdown.cancelled() => Play::Stop,
                () = viewer.gone.cancelled() => Play::Stop,
                () = viewer.stalled() => Play::Stop,
                () = lease.capture.ended.cancelled() => Play::Stop,
                _ = taint.changed() => Play::Check,
                changed = init.changed(), if !started => {
                    if changed.is_ok() { Play::Init } else { Play::Stop }
                }
                fragment = fragments.recv(), if started => Play::Fragment(fragment),
                changed = view.changed() => {
                    if changed.is_ok() { Play::Check } else { Play::End }
                }
            };
            match event {
                Play::Init => {
                    let Some(segment) = init.borrow_and_update().clone() else {
                        continue;
                    };
                    // The queue is empty before the first push, so it cannot be full.
                    viewer.push(segment);
                    started = true;
                }
                Play::Fragment(Ok(fragment)) => viewer.push(fragment),
                Play::Fragment(Err(RecvError::Lagged(_))) => {}
                Play::Fragment(Err(RecvError::Closed)) | Play::Stop => break false,
                Play::Check => {
                    if matches!(
                        resolve(&view.borrow_and_update(), Some(tab), &self.privacy.taint),
                        Resolved::End
                    ) {
                        break true;
                    }
                }
                Play::End => break true,
            }
        };
        // As for a screencast: no grace period once the newest view ends this stream.
        if ended
            || matches!(
                resolve(&view.borrow(), Some(tab), &self.privacy.taint),
                Resolved::End
            )
        {
            lease.end_now();
        }
        viewer.close();
    }
}

enum Event {
    Frame,
    Resolve,
    Stop,
    End,
}

/// What a follower does after an event.
enum Next {
    Continue,
    /// A switch failed; try again after `SWITCH_RETRY` unless something changes first.
    Retry,
    /// Leave, as the viewer did: the capture gets its grace period while the view still shows
    /// this stream.
    Stop,
    /// Leave for good: the session ended or was withheld, so no grace.
    End,
}

enum Play {
    Init,
    Fragment(Result<Bytes, RecvError>),
    Check,
    Stop,
    End,
}

/// What a stream should show now.
enum Resolved {
    Show(ShownTab),
    /// Keep the stream open: the session is live but has nothing to show right now.
    Wait,
    /// Close the stream: the session ended or is withheld, or its pinned tab left the view.
    End,
}

fn resolve(view: &FollowView, pinned: Option<TabId>, taint: &Taint) -> Resolved {
    if !view.live || view.withheld {
        return Resolved::End;
    }
    match pinned {
        Some(tab) => view
            .tabs
            .iter()
            .find(|shown| shown.tab == tab && !taint.contains(&shown.target))
            .cloned()
            .map_or(Resolved::End, Resolved::Show),
        // A live session between tabs: it closed one before opening the next.
        None => view
            .tabs
            .iter()
            .find(|shown| !taint.contains(&shown.target))
            .cloned()
            .map_or(Resolved::Wait, Resolved::Show),
    }
}

/// The tab a new view opens on, or why there is nothing to watch.
fn pick(view: &FollowView, tab: Option<TabId>, taint: &Taint) -> Result<ShownTab, CaptureError> {
    if !view.live {
        return Err(CaptureError::SessionNotLive);
    }
    if view.withheld {
        return Err(CaptureError::Withheld);
    }
    if !view.tabs.iter().any(|shown| !taint.contains(&shown.target)) {
        return Err(CaptureError::NoOpenTab);
    }
    match tab {
        Some(tab) => view
            .tabs
            .iter()
            .find(|shown| shown.tab == tab && !taint.contains(&shown.target))
            .cloned()
            .ok_or(CaptureError::TabNotFound),
        None => view
            .tabs
            .iter()
            .find(|shown| !taint.contains(&shown.target))
            .cloned()
            .ok_or(CaptureError::NoOpenTab),
    }
}

/// Whether a view lets a viewer see a tab now.
enum Shows {
    Tab,
    /// Live and not withheld, but not showing this tab: an unpinned screencast skips the frame
    /// while its follower moves on.
    OtherTabs,
    /// Ended or withheld.
    Nothing,
}

/// The revocation check: every frame and fragment passes it as the viewer's stream yields it,
/// which is right before the writer starts that part.
fn shows(view: &FollowView, tab: TabId, taint: &Taint) -> Shows {
    if !view.live || view.withheld {
        Shows::Nothing
    } else if view
        .tabs
        .iter()
        .any(|shown| shown.tab == tab && !taint.contains(&shown.target))
    {
        Shows::Tab
    } else {
        Shows::OtherTabs
    }
}

/// The tab a screencast stream shows, and its hold on that tab's capture.
struct Showing {
    tab: ShownTab,
    lease: Lease,
    frames: watch::Receiver<Option<Arc<RawFrame>>>,
}

impl Showing {
    fn new(tab: ShownTab, lease: Lease) -> Self {
        let frames = lease.frames();
        Self { tab, lease, frames }
    }

    fn ended(&self) -> bool {
        self.lease.capture.ended.is_cancelled()
    }

    /// Hands the capture's newest frame to the viewer, stamped with the tab it shows.
    fn forward(&mut self, viewer: &Viewer<Frame>) {
        let raw = self.frames.borrow_and_update().clone();
        if let Some(raw) = raw {
            viewer.replace(Frame {
                jpeg: raw.jpeg.clone(),
                tab: self.tab.tab,
                url: self.tab.url.clone(),
                captured_at: raw.captured_at,
            });
        }
    }
}

/// Forgets a shown capture that ended, so the next resolve starts a new one.
fn drop_ended(showing: &mut Option<Showing>) {
    if showing.as_ref().is_some_and(Showing::ended) {
        *showing = None;
    }
}

enum Activity {
    Frame,
    Ended,
}

/// Resolves when the shown capture produces a frame or ends. Never resolves while nothing is
/// shown or once its end was seen; the next resolve then replaces or drops it.
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

async fn sleep_until(deadline: Option<Instant>) {
    match deadline {
        Some(deadline) => time::sleep_until(deadline).await,
        None => future::pending().await,
    }
}

/// What one viewer has not taken yet, shared by its follower or player (the writer) and its
/// stream (the reader). The writer never waits on the reader: a screencast keeps only the
/// newest frame, and a video skips whole fragments once `VIDEO_QUEUE` wait.
struct Viewer<T> {
    waiting: Mutex<Waiting<T>>,
    filled: Notify,
    /// Cancelled when the viewer's stream is dropped or ends.
    gone: CancellationToken,
}

struct Waiting<T> {
    items: VecDeque<T>,
    /// Since when something has waited with nothing taken: the stall clock.
    since: Option<Instant>,
    closed: bool,
}

impl<T> Viewer<T> {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            waiting: Mutex::new(Waiting {
                items: VecDeque::new(),
                since: None,
                closed: false,
            }),
            filled: Notify::new(),
            gone: CancellationToken::new(),
        })
    }

    /// Replaces whatever waits with `item`: of screencast frames, only the newest matters.
    fn replace(&self, item: T) {
        if let Some(mut waiting) = self.waiting()
            && !waiting.closed
        {
            waiting.items.clear();
            waiting.items.push_back(item);
            waiting.since.get_or_insert_with(Instant::now);
        }
        self.filled.notify_one();
    }

    /// Queues `item` unless `VIDEO_QUEUE` items already wait, in which case it is skipped.
    fn push(&self, item: T) {
        if let Some(mut waiting) = self.waiting()
            && !waiting.closed
            && waiting.items.len() < cdp::VIDEO_QUEUE
        {
            waiting.items.push_back(item);
            waiting.since.get_or_insert_with(Instant::now);
        }
        self.filled.notify_one();
    }

    /// Drops what waits and ends the stream at its next read.
    fn close(&self) {
        if let Some(mut waiting) = self.waiting() {
            waiting.closed = true;
            waiting.items.clear();
            waiting.since = None;
        }
        self.filled.notify_one();
    }

    /// The next item, or `None` once the writer closed the viewer.
    async fn take(&self) -> Option<T> {
        loop {
            let filled = self.filled.notified();
            {
                let mut waiting = self.waiting()?;
                if waiting.closed {
                    return None;
                }
                if let Some(item) = waiting.items.pop_front() {
                    waiting.since = (!waiting.items.is_empty()).then(Instant::now);
                    return Some(item);
                }
            }
            filled.await;
        }
    }

    /// Resolves once something has waited `VIEWER_STALL` with nothing taken. The writer's loop
    /// makes a fresh one after every put, so it always measures the current wait.
    async fn stalled(&self) {
        let Some(since) = self.since() else {
            return future::pending().await;
        };
        time::sleep_until(since + cdp::VIEWER_STALL).await;
        if self.since() != Some(since) {
            future::pending::<()>().await;
        }
    }

    fn since(&self) -> Option<Instant> {
        self.waiting().and_then(|waiting| waiting.since)
    }

    /// `None` once a panic under the lock poisoned it, which nothing here does; the viewer then
    /// reads as closed.
    fn waiting(&self) -> Option<MutexGuard<'_, Waiting<T>>> {
        self.waiting.lock().ok()
    }
}

impl Frames {
    fn new(
        viewer: Arc<Viewer<Frame>>,
        view: watch::Receiver<FollowView>,
        taint: Arc<Taint>,
    ) -> Self {
        let leaving = viewer.gone.clone().drop_guard();
        Self(Box::pin(stream::unfold(
            (viewer, view, leaving, taint),
            |(viewer, view, leaving, taint)| async move {
                loop {
                    let frame = viewer.take().await?;
                    let shows = shows(&view.borrow(), frame.tab, &taint);
                    match shows {
                        Shows::Tab => return Some((frame, (viewer, view, leaving, taint))),
                        Shows::OtherTabs => {}
                        Shows::Nothing => return None,
                    }
                }
            },
        )))
    }
}

impl Stream for Frames {
    type Item = Frame;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Frame>> {
        self.0.as_mut().poll_next(cx)
    }
}

impl VideoChunks {
    fn new(
        viewer: Arc<Viewer<Bytes>>,
        view: watch::Receiver<FollowView>,
        tab: TabId,
        taint: Arc<Taint>,
    ) -> Self {
        let leaving = viewer.gone.clone().drop_guard();
        Self(Box::pin(stream::unfold(
            (viewer, view, leaving, taint),
            move |(viewer, view, leaving, taint)| async move {
                let chunk = viewer.take().await?;
                // A video stays on its tab: once the view stops showing it, the stream ends.
                let shows = shows(&view.borrow(), tab, &taint);
                matches!(shows, Shows::Tab).then_some((chunk, (viewer, view, leaving, taint)))
            },
        )))
    }
}

impl Stream for VideoChunks {
    type Item = Bytes;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Bytes>> {
        self.0.as_mut().poll_next(cx)
    }
}
