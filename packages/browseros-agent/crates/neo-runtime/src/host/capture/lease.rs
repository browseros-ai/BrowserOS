//! One running capture of one tab, shared by every viewer of that (tab, kind, quality), and the
//! worker that drives it. Copied from claw-server
//! (`apps/claw-server/src/services/capture/tab_capture.rs:1-455`, origin/main) with its rules
//! unchanged, over the host link (`link.rs`) and `cdp.rs`.
//!
//! The first viewer creates the capture and starts its worker; viewers hold a [`Lease`]. When
//! the last lease drops, the capture lingers for `STOP_GRACE` so a reconnecting viewer or a
//! follow switch back does not restart the browser side, then stops. A departure generation
//! keeps an earlier grace timer from cutting a later grace period short. The worker also ends
//! the capture on its own when its session goes: the tab closed, or the socket was replaced.
//!
//! One change from claw-server: a frame without a browser timestamp, or over the frame cap, is
//! dropped and counted ([`FrameDrops`]) instead of being stamped with the server's clock.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use bytes::Bytes;
use tokio::runtime::Handle;
use tokio::sync::{broadcast, watch};
use tokio::time::{self, Instant};
use tokio_util::sync::CancellationToken;
use tracing::debug;

use crate::host::capture::mp4::{Mp4Part, Mp4Splitter};
use crate::host::capture::{Preset, Quality};
use crate::host::cdp::{self, ScreencastFrame};
use crate::host::ids::EpochMs;
use crate::host::link::{Attached, HostLink};
use crate::run::PageId;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum Kind {
    Screencast,
    Video,
}

/// Chromium refuses a screencast and a screen recording in one session, so the kind is part of
/// the key and each capture attaches its own session.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) struct CaptureKey {
    pub(super) target: PageId,
    pub(super) kind: Kind,
    pub(super) quality: Quality,
}

pub(super) type Registry = Arc<Mutex<HashMap<CaptureKey, Arc<Capture>>>>;

/// A JPEG as the browser produced it, at the browser's frame-swap time.
#[derive(Debug)]
pub(super) struct RawFrame {
    pub(super) jpeg: Bytes,
    pub(super) captured_at: EpochMs,
}

/// Screencast frames the workers dropped instead of forwarding, by reason. Every one of them was
/// still acknowledged, so the screencast kept flowing.
#[derive(Debug, Default)]
pub(crate) struct FrameDrops {
    untimed: AtomicU64,
    oversized: AtomicU64,
    undecodable: AtomicU64,
}

impl FrameDrops {
    /// Frames without a positive `metadata.timestamp`: arrival time cannot be correlated with
    /// a browser action, so none is ever stamped with it.
    pub(crate) fn untimed(&self) -> u64 {
        self.untimed.load(Ordering::Relaxed)
    }

    /// Frames over [`cdp::MAX_FRAME_BYTES`], which the main host rejects.
    pub(crate) fn oversized(&self) -> u64 {
        self.oversized.load(Ordering::Relaxed)
    }

    /// Frames whose event or JPEG could not be read.
    pub(crate) fn undecodable(&self) -> u64 {
        self.undecodable.load(Ordering::Relaxed)
    }
}

pub(super) struct Capture {
    pub(super) key: CaptureKey,
    /// Changed only under the registry lock, so the grace stop and a new viewer never race.
    viewers: AtomicUsize,
    /// Counts the times the last viewer left. A grace timer stops the capture only if no one
    /// joined and left again since it started, so every departure gets its full grace period.
    departures: AtomicU64,
    stop: CancellationToken,
    pub(super) ended: CancellationToken,
    ready: watch::Sender<Option<Result<(), String>>>,
    /// The newest screencast frame: a viewer that joins, or falls behind, gets this one.
    frames: watch::Sender<Option<Arc<RawFrame>>>,
    /// The recording's initialization segment, kept for viewers that join later.
    init: watch::Sender<Option<Bytes>>,
    fragments: broadcast::Sender<Bytes>,
}

impl Capture {
    fn new(key: CaptureKey) -> Arc<Self> {
        Arc::new(Self {
            key,
            viewers: AtomicUsize::new(0),
            departures: AtomicU64::new(0),
            stop: CancellationToken::new(),
            ended: CancellationToken::new(),
            ready: watch::channel(None).0,
            frames: watch::channel(None).0,
            init: watch::channel(None).0,
            fragments: broadcast::channel(cdp::FRAGMENT_BACKLOG).0,
        })
    }
}

/// A viewer's hold on a capture; dropping it is how a viewer leaves.
pub(super) struct Lease {
    pub(super) capture: Arc<Capture>,
    registry: Registry,
    /// Set by [`Lease::end_now`]: the last viewer leaving stops the capture without a grace.
    stop_on_release: bool,
}

impl Lease {
    /// Waits until the browser side is running. An error means it never started.
    pub(super) async fn ready(&self) -> Result<(), String> {
        let mut ready = self.capture.ready.subscribe();
        match ready.wait_for(Option::is_some).await {
            Ok(state) => state.clone().unwrap_or(Ok(())),
            Err(_) => Err("capture stopped before it started".to_string()),
        }
    }

    /// Screencast frames, starting with the newest one already captured: a static page sends
    /// no new frames, so a viewer joining later would otherwise see nothing.
    pub(super) fn frames(&self) -> watch::Receiver<Option<Arc<RawFrame>>> {
        let mut frames = self.capture.frames.subscribe();
        frames.mark_changed();
        frames
    }

    /// Video fragments from now on, then the initialization segment. Subscribe to fragments
    /// first so none is lost between reading the init segment and the next fragment.
    pub(super) fn video(&self) -> (watch::Receiver<Option<Bytes>>, broadcast::Receiver<Bytes>) {
        let fragments = self.capture.fragments.subscribe();
        // Deliver an init segment captured before this viewer came, too.
        let mut init = self.capture.init.subscribe();
        init.mark_changed();
        (init, fragments)
    }

    /// Leaves for good, as when the session ended: if this was the last viewer the capture
    /// stops now instead of lingering for a viewer that cannot come back.
    pub(super) fn end_now(mut self) {
        self.stop_on_release = true;
    }
}

impl Drop for Lease {
    fn drop(&mut self) {
        let departure = {
            let Some(mut captures) = lock(&self.registry) else {
                // Unreachable without a panic under the lock; stopping is the safe side.
                self.capture.stop.cancel();
                return;
            };
            if self.capture.viewers.fetch_sub(1, Ordering::SeqCst) != 1 {
                return;
            }
            if self.stop_on_release {
                stop(&mut captures, &self.capture);
                return;
            }
            self.capture.departures.fetch_add(1, Ordering::SeqCst) + 1
        };
        match Handle::try_current() {
            Ok(runtime) => {
                runtime.spawn(stop_after_grace(
                    Arc::clone(&self.registry),
                    Arc::clone(&self.capture),
                    departure,
                ));
            }
            Err(_) => self.capture.stop.cancel(),
        }
    }
}

/// Joins the running capture for `key`, if there is one.
pub(super) fn join(registry: &Registry, key: &CaptureKey) -> Option<Lease> {
    let captures = lock(registry)?;
    let capture = captures
        .get(key)
        .filter(|capture| !capture.ended.is_cancelled())?;
    capture.viewers.fetch_add(1, Ordering::SeqCst);
    Some(Lease {
        capture: Arc::clone(capture),
        registry: Arc::clone(registry),
        stop_on_release: false,
    })
}

/// Joins the running capture for `key`, or creates one and starts its worker on `link`. The
/// check repeats under the lock, so two viewers racing to start one capture share it. `None`
/// only if the registry is unusable.
pub(super) fn join_or_start(
    registry: &Registry,
    key: CaptureKey,
    link: Arc<HostLink>,
    preset: Preset,
    drops: Arc<FrameDrops>,
) -> Option<Lease> {
    if let Some(lease) = join(registry, &key) {
        return Some(lease);
    }
    let mut captures = lock(registry)?;
    if let Some(capture) = captures
        .get(&key)
        .filter(|capture| !capture.ended.is_cancelled())
    {
        capture.viewers.fetch_add(1, Ordering::SeqCst);
        return Some(Lease {
            capture: Arc::clone(capture),
            registry: Arc::clone(registry),
            stop_on_release: false,
        });
    }
    let capture = Capture::new(key.clone());
    capture.viewers.store(1, Ordering::SeqCst);
    captures.insert(key, Arc::clone(&capture));
    drop(captures);
    tokio::spawn(run(
        link,
        Arc::clone(&capture),
        Arc::clone(registry),
        preset,
        drops,
    ));
    Some(Lease {
        capture,
        registry: Arc::clone(registry),
        stop_on_release: false,
    })
}

/// Stops every capture of `target` now, without a grace period, whoever watches it.
pub(super) fn stop_target(registry: &Registry, target: &PageId) {
    let Some(mut captures) = lock(registry) else {
        return;
    };
    let doomed: Vec<Arc<Capture>> = captures
        .values()
        .filter(|capture| capture.key.target == *target)
        .cloned()
        .collect();
    for capture in &doomed {
        stop(&mut captures, capture);
    }
}

async fn stop_after_grace(registry: Registry, capture: Arc<Capture>, departure: u64) {
    tokio::select! {
        () = time::sleep(cdp::STOP_GRACE) => {}
        () = capture.ended.cancelled() => return,
    }
    let Some(mut captures) = lock(&registry) else {
        capture.stop.cancel();
        return;
    };
    // Someone is watching again, or left again later and owns a newer grace period.
    if capture.viewers.load(Ordering::SeqCst) > 0
        || capture.departures.load(Ordering::SeqCst) != departure
    {
        return;
    }
    stop(&mut captures, &capture);
}

/// Unregisters the capture and tells its worker to stop. Takes the locked registry so a new
/// viewer cannot join between the check and the stop.
fn stop(captures: &mut HashMap<CaptureKey, Arc<Capture>>, capture: &Arc<Capture>) {
    if captures
        .get(&capture.key)
        .is_some_and(|current| Arc::ptr_eq(current, capture))
    {
        captures.remove(&capture.key);
    }
    capture.stop.cancel();
}

async fn run(
    link: Arc<HostLink>,
    capture: Arc<Capture>,
    registry: Registry,
    preset: Preset,
    drops: Arc<FrameDrops>,
) {
    let result = match link.attach(&capture.key.target).await {
        Ok(session) => {
            let result = match capture.key.kind {
                Kind::Screencast => screencast(&link, &session, &capture, preset, &drops).await,
                Kind::Video => video(&session, &capture, preset).await,
            };
            link.release(&session).await;
            result
        }
        Err(error) => Err(error.to_string()),
    };
    if let Err(error) = result {
        debug!(target = capture.key.target.as_str(), %error, "capture ended with an error");
        capture.ready.send_if_modified(|ready| {
            let unset = ready.is_none();
            if unset {
                *ready = Some(Err(error));
            }
            unset
        });
    }
    capture.ended.cancel();
    if let Some(mut captures) = lock(&registry)
        && captures
            .get(&capture.key)
            .is_some_and(|current| Arc::ptr_eq(current, &capture))
    {
        captures.remove(&capture.key);
    }
}

/// Resolves when the capture must stop, for any reason: no viewer is left, or its session went
/// with its tab or its socket.
async fn stopped(session: &Attached, capture: &Capture) {
    tokio::select! {
        () = capture.stop.cancelled() => {}
        () = session.gone() => {}
    }
}

async fn screencast(
    link: &HostLink,
    session: &Attached,
    capture: &Capture,
    preset: Preset,
    drops: &FrameDrops,
) -> Result<(), String> {
    let mut frames = link.frames(session);
    cdp::start_screencast(session, preset)
        .await
        .map_err(|error| error.to_string())?;
    capture.ready.send_replace(Some(Ok(())));
    let interval = Duration::from_millis(1000 / u64::from(preset.screencast_fps.max(1)));
    let mut last_ack: Option<Instant> = None;
    loop {
        let params = tokio::select! {
            () = stopped(session, capture) => break,
            params = frames.recv() => params,
        };
        let Some(params) = params else { break };
        let frame = cdp::screencast_frame(&params);
        let ack = frame.number;
        if let Some(frame) = raw_frame(frame, drops) {
            capture.frames.send_replace(Some(Arc::new(frame)));
        }
        // The ack is the frame-rate limit: Chromium sends the next frame only after it.
        if let Some(last) = last_ack {
            tokio::select! {
                () = stopped(session, capture) => break,
                () = time::sleep_until(last + interval) => {}
            }
        }
        let Some(ack) = ack else { continue };
        if cdp::ack_frame(session, ack).await.is_err() {
            break;
        }
        last_ack = Some(Instant::now());
    }
    cdp::stop_screencast(session).await;
    Ok(())
}

/// The frame a viewer may get, or `None` (counted) for one that carries no browser time, is
/// over the frame cap, or cannot be read.
fn raw_frame(frame: ScreencastFrame, drops: &FrameDrops) -> Option<RawFrame> {
    let Some(jpeg) = frame.jpeg else {
        drops.undecodable.fetch_add(1, Ordering::Relaxed);
        return None;
    };
    let Some(captured_at) = frame.timestamp.and_then(EpochMs::from_frame_seconds) else {
        drops.untimed.fetch_add(1, Ordering::Relaxed);
        return None;
    };
    if jpeg.len() > cdp::MAX_FRAME_BYTES {
        drops.oversized.fetch_add(1, Ordering::Relaxed);
        return None;
    }
    Some(RawFrame {
        jpeg: Bytes::from(jpeg),
        captured_at,
    })
}

async fn video(session: &Attached, capture: &Capture, preset: Preset) -> Result<(), String> {
    let handle = cdp::start_recording(session, preset)
        .await
        .map_err(|error| error.to_string())?;
    capture.ready.send_replace(Some(Ok(())));
    let result = drain_recording(session, capture, &handle).await;
    cdp::stop_recording(session, &handle).await;
    result
}

async fn drain_recording(
    session: &Attached,
    capture: &Capture,
    handle: &str,
) -> Result<(), String> {
    let mut splitter = Mp4Splitter::default();
    let mut tick = time::interval(cdp::RECORDING_READ_INTERVAL);
    tick.reset();
    let mut early_failures = 0;
    let mut written = false;
    loop {
        tokio::select! {
            () = stopped(session, capture) => return Ok(()),
            _ = tick.tick() => {}
        }
        // While recording, `eof` only means "nothing more yet"; read until a read comes back
        // empty.
        loop {
            let chunk = match cdp::read_recording(session, handle).await {
                Ok(chunk) => chunk,
                Err(_) if !written && early_failures < cdp::EARLY_READ_FAILURES => {
                    early_failures += 1;
                    break;
                }
                Err(error) => return Err(error.to_string()),
            };
            let bytes = chunk
                .bytes()
                .map_err(|error| format!("recording chunk: {error}"))?;
            if bytes.is_empty() {
                break;
            }
            written = true;
            let parts = splitter.push(&bytes).map_err(|error| error.to_string())?;
            for part in parts {
                match part {
                    Mp4Part::Init(init) => {
                        capture.init.send_replace(Some(init));
                    }
                    Mp4Part::Fragment(fragment) => {
                        // Fails only while no viewer listens.
                        let _ = capture.fragments.send(fragment);
                    }
                }
            }
        }
    }
}

/// The registry, or `None` once a panic under its lock poisoned it, which nothing here does.
fn lock<T>(mutex: &Mutex<T>) -> Option<MutexGuard<'_, T>> {
    mutex.lock().ok()
}
