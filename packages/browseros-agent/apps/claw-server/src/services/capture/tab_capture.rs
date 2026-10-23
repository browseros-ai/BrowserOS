//! One running capture of one tab, shared by every viewer of that (tab, kind, quality).
//!
//! The first viewer creates it and starts its worker; viewers hold a [`Lease`]. When the last
//! lease drops, the capture lingers for [`STOP_GRACE`] so a reconnecting viewer or a follow
//! switch back does not restart the browser side, then stops. The worker also ends it on its
//! own when the tab closes or the capture connection breaks.

use super::{
    link::{Attached, Link},
    mp4::{Mp4Part, Mp4Splitter},
    presets::{Preset, Quality},
};
use crate::{clock::now_epoch_ms, error::AppResult};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use bytes::Bytes;
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::{
    sync::{broadcast, watch},
    time::Instant,
};
use tokio_util::sync::CancellationToken;

/// How long a capture outlives its last viewer.
pub(super) const STOP_GRACE: Duration = Duration::from_secs(5);
/// How often a recording's IO stream is drained. Chromium writes a fragment per keyframe
/// interval (about 2 s), so this only bounds how late a finished fragment is forwarded.
const RECORDING_READ_INTERVAL: Duration = Duration::from_millis(250);
const RECORDING_READ_SIZE: u64 = 1 << 20;
/// Reads that may fail before the recording's first bytes exist. Chromium creates the stream
/// file on its first write and answers earlier reads with "Read failed".
const EARLY_READ_FAILURES: u32 = 20;
/// Fragments a slow video viewer may fall behind before it skips ahead.
const FRAGMENT_BACKLOG: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum Kind {
    Screencast,
    Video,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) struct CaptureKey {
    pub(super) target_id: String,
    pub(super) kind: Kind,
    pub(super) quality: Quality,
}

pub(super) type Registry = Arc<Mutex<HashMap<CaptureKey, Arc<Capture>>>>;

/// A JPEG as the browser produced it.
#[derive(Debug)]
pub(super) struct RawFrame {
    pub(super) jpeg: Bytes,
    pub(super) captured_at: i64,
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
            fragments: broadcast::channel(FRAGMENT_BACKLOG).0,
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
            let mut captures = lock(&self.registry);
            if self.capture.viewers.fetch_sub(1, Ordering::SeqCst) != 1 {
                return;
            }
            if self.stop_on_release {
                stop(&mut captures, &self.capture);
                return;
            }
            self.capture.departures.fetch_add(1, Ordering::SeqCst) + 1
        };
        match tokio::runtime::Handle::try_current() {
            Ok(runtime) => {
                runtime.spawn(stop_after_grace(
                    self.registry.clone(),
                    self.capture.clone(),
                    departure,
                ));
            }
            Err(_) => self.capture.stop.cancel(),
        }
    }
}

/// Joins the running capture for `key`, if there is one.
pub(super) fn join(registry: &Registry, key: &CaptureKey) -> Option<Lease> {
    let captures = lock(registry);
    let capture = captures
        .get(key)
        .filter(|capture| !capture.ended.is_cancelled())?;
    capture.viewers.fetch_add(1, Ordering::SeqCst);
    Some(Lease {
        capture: capture.clone(),
        registry: registry.clone(),
        stop_on_release: false,
    })
}

/// Joins the running capture for `key`, or creates one and starts its worker on `link`. The
/// check repeats under the lock, so two viewers racing to start one capture share it.
pub(super) fn join_or_start(
    registry: &Registry,
    key: CaptureKey,
    link: Arc<Link>,
    preset: Preset,
) -> Lease {
    if let Some(lease) = join(registry, &key) {
        return lease;
    }
    let mut captures = lock(registry);
    if let Some(capture) = captures
        .get(&key)
        .filter(|capture| !capture.ended.is_cancelled())
    {
        capture.viewers.fetch_add(1, Ordering::SeqCst);
        return Lease {
            capture: capture.clone(),
            registry: registry.clone(),
            stop_on_release: false,
        };
    }
    let capture = Capture::new(key.clone());
    capture.viewers.store(1, Ordering::SeqCst);
    captures.insert(key, capture.clone());
    drop(captures);
    tokio::spawn(run(link, capture.clone(), registry.clone(), preset));
    Lease {
        capture,
        registry: registry.clone(),
        stop_on_release: false,
    }
}

async fn stop_after_grace(registry: Registry, capture: Arc<Capture>, departure: u64) {
    tokio::select! {
        () = tokio::time::sleep(STOP_GRACE) => {}
        () = capture.ended.cancelled() => return,
    }
    let mut captures = lock(&registry);
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

async fn run(link: Arc<Link>, capture: Arc<Capture>, registry: Registry, preset: Preset) {
    let result = match link.attach(&capture.key.target_id).await {
        Ok(session) => {
            let result = match capture.key.kind {
                Kind::Screencast => screencast(&link, &session, &capture, preset).await,
                Kind::Video => video(&link, &session, &capture, preset).await,
            };
            link.release(&session).await;
            result
        }
        Err(error) => Err(error),
    };
    if let Err(error) = result {
        tracing::debug!(target_id = %capture.key.target_id, %error, "capture ended with an error");
        capture.ready.send_if_modified(|ready| {
            let unset = ready.is_none();
            if unset {
                *ready = Some(Err(error.to_string()));
            }
            unset
        });
    }
    capture.ended.cancel();
    let mut captures = lock(&registry);
    if captures
        .get(&capture.key)
        .is_some_and(|current| Arc::ptr_eq(current, &capture))
    {
        captures.remove(&capture.key);
    }
}

/// Whether the capture should keep running; resolves when it must stop, for any reason.
async fn stopped(link: &Link, session: &Attached, capture: &Capture) {
    tokio::select! {
        () = capture.stop.cancelled() => {}
        () = session.detached.cancelled() => {}
        () = link.broken.cancelled() => {}
    }
}

async fn screencast(
    link: &Link,
    session: &Attached,
    capture: &Capture,
    preset: Preset,
) -> AppResult<()> {
    let mut frames = link.frames(session);
    link.send(
        "Page.startScreencast",
        json!({
            "format": "jpeg",
            "quality": preset.jpeg_quality,
            "maxWidth": preset.max_width,
            "maxHeight": preset.max_height,
            // One frame at a time: while it is unacknowledged Chromium drops newer frames
            // before copying them on its UI thread, which is what makes pacing cheap.
            "maxFramesInFlight": 1,
        }),
        session,
    )
    .await?;
    capture.ready.send_replace(Some(Ok(())));
    let interval = Duration::from_millis(1000 / u64::from(preset.screencast_fps.max(1)));
    let mut last_ack: Option<Instant> = None;
    loop {
        let params = tokio::select! {
            () = stopped(link, session, capture) => break,
            params = frames.recv() => params,
        };
        let Some(params) = params else { break };
        if let Some(frame) = decode_frame(&params) {
            capture.frames.send_replace(Some(Arc::new(frame)));
        }
        // The ack is the frame-rate limit: Chromium sends the next frame only after it.
        if let Some(last) = last_ack {
            tokio::select! {
                () = stopped(link, session, capture) => break,
                () = tokio::time::sleep_until(last + interval) => {}
            }
        }
        let ack = json!({ "sessionId": params.get("sessionId").cloned().unwrap_or(Value::Null) });
        if link
            .send("Page.screencastFrameAck", ack, session)
            .await
            .is_err()
        {
            break;
        }
        last_ack = Some(Instant::now());
    }
    let _ = link.send("Page.stopScreencast", json!({}), session).await;
    Ok(())
}

fn decode_frame(params: &Value) -> Option<RawFrame> {
    let jpeg = BASE64.decode(params.get("data")?.as_str()?).ok()?;
    // `timestamp` is when the browser received the frame, in seconds.
    let captured_at = params
        .pointer("/metadata/timestamp")
        .and_then(Value::as_f64)
        .map_or_else(now_epoch_ms, |seconds| (seconds * 1000.0) as i64);
    Some(RawFrame {
        jpeg: Bytes::from(jpeg),
        captured_at,
    })
}

async fn video(
    link: &Link,
    session: &Attached,
    capture: &Capture,
    preset: Preset,
) -> AppResult<()> {
    let reply = link
        .send(
            "Page.startScreenRecording",
            json!({
                "maxWidth": preset.max_width,
                "maxHeight": preset.max_height,
                "frameRate": preset.video_fps,
            }),
            session,
        )
        .await?;
    let handle = reply
        .get("stream")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            crate::error::AppError::Internal("startScreenRecording returned no stream".into())
        })?
        .to_string();
    capture.ready.send_replace(Some(Ok(())));
    let result = drain_recording(link, session, capture, &handle).await;
    let _ = link
        .send("Page.stopScreenRecording", json!({}), session)
        .await;
    let _ = link
        .send("IO.close", json!({ "handle": handle }), session)
        .await;
    result
}

async fn drain_recording(
    link: &Link,
    session: &Attached,
    capture: &Capture,
    handle: &str,
) -> AppResult<()> {
    let mut splitter = Mp4Splitter::default();
    let mut tick = tokio::time::interval(RECORDING_READ_INTERVAL);
    tick.reset();
    let mut early_failures = 0;
    let mut written = false;
    loop {
        tokio::select! {
            () = stopped(link, session, capture) => return Ok(()),
            _ = tick.tick() => {}
        }
        // While recording, `eof` only means "nothing more yet"; read until a read comes back empty.
        loop {
            let chunk = match link
                .send(
                    "IO.read",
                    json!({ "handle": handle, "size": RECORDING_READ_SIZE }),
                    session,
                )
                .await
            {
                Ok(chunk) => chunk,
                Err(_) if !written && early_failures < EARLY_READ_FAILURES => {
                    early_failures += 1;
                    break;
                }
                Err(error) => return Err(error),
            };
            let data = chunk
                .get("data")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let bytes = if chunk.get("base64Encoded").and_then(Value::as_bool) == Some(true) {
                BASE64.decode(data).map_err(|error| {
                    crate::error::AppError::Internal(format!("recording chunk: {error}"))
                })?
            } else {
                data.as_bytes().to_vec()
            };
            if bytes.is_empty() {
                break;
            }
            written = true;
            let parts = splitter
                .push(&bytes)
                .map_err(|error| crate::error::AppError::Internal(error.to_string()))?;
            for part in parts {
                match part {
                    Mp4Part::Init(init) => {
                        capture.init.send_replace(Some(init));
                    }
                    Mp4Part::Fragment(fragment) => {
                        let _ = capture.fragments.send(fragment);
                    }
                }
            }
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}
