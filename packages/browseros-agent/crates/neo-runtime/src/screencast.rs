//! Neo: `Page.screencast`, Playwright's client `Screencast` over its server screencast and page
//! dispatcher: a CDP screencast of the tab, whose JPEG frames reach the realm's
//! `onFrame` as `screencastFrame` events. The browser gets a frame's acknowledgement only once
//! the realm acknowledged it, so a slow `onFrame` slows the stream rather than queue it, as
//! Playwright's client acknowledges after its callback. The overlays are `overlay.rs`.
//!
//! ```text
//!  Page.screencastStart {size?, quality?, fps?, sendFrames?, record?}
//!    running → Error; fps ≤ 0 → Error; record → Error (Neo has no video encoder)
//!    sendFrames → route the tab session's frames (ScreencastFrames), then
//!      Page.startScreencast {jpeg, quality ?? 90, maxWidth × maxHeight = size, else 800 × 600
//!      fitted into 800, both even; maxFramesInFlight: fps > 30 ? 10 : 5}; each frame →
//!      event screencastFrame {frameId (the run's own count), data, timestamp, viewportWidth,
//!      viewportHeight}, or, with the realm not listening, its acknowledgement at once
//!  Page.screencastFrameAck {frameId}   → Page.screencastFrameAck of that frame; unknown → nothing
//!  Page.screencastStop {}              → Page.stopScreencast; nothing running → nothing
//!  run end (`Dispatcher::finish`)      the forwarding stops before the run's tasks are awaited,
//!                                      then each tab's Page.stopScreencast
//! ```
//!
//! A screencast belongs to the CDP session that started it: another client's screencast of the
//! same tab (BrowserOS's live view, on claw-server's own socket) has its own frames and
//! acknowledgements, so starting, pacing or stopping this one neither stops nor starves it.

use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use browseros_cdp::{CdpClient, CdpEvent, SessionId};
use neo_runtime_spec::serve::{
    PageCall, PageScreencastChapterParams, PageScreencastChapterResult,
    PageScreencastFrameAckParams, PageScreencastFrameAckResult, PageScreencastHideActionsParams,
    PageScreencastHideActionsResult, PageScreencastRemoveOverlayParams,
    PageScreencastRemoveOverlayResult, PageScreencastSetOverlayVisibleParams,
    PageScreencastSetOverlayVisibleResult, PageScreencastShowOverlayParams,
    PageScreencastShowOverlayResult, PageScreencastStartParams, PageScreencastStartResult,
    PageScreencastStopParams, PageScreencastStopResult,
};
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::dispatch::encode_result;
use crate::overlay;
use crate::page::PageSession;
use crate::progress::{CallError, PlaywrightError, Progress};
use crate::tabs::Tabs;

/// The CDP event of one screencast frame.
const FRAME_EVENT: &str = "Page.screencastFrame";

/// The size Playwright's screencast takes without `size` on a context with no viewport, as a
/// connectOverCDP default context has, before fitting it into [`MAX_SIDE`].
///
/// Playwright: `server/screencast.ts` `Screencast._startScreencast`: `const viewport = this.page.browserContext._options.viewport || { width: 800, height: 600 };`
const DEFAULT_VIEWPORT: (i64, i64) = (800, 600);

/// The longest side of a screencast frame without `size`.
///
/// Playwright: `server/screencast.ts` `Screencast._startScreencast`: `const scale = Math.min(1, 800 / Math.max(viewport.width, viewport.height));`
const MAX_SIDE: f64 = 800.0;

/// Playwright: `server/screencast.ts` `Screencast._startScreencast`: `quality: quality ?? 90,`
const DEFAULT_QUALITY: i64 = 90;

/// Neo: the reason `Page.screencastStart` with `record` is refused; the SDK refuses
/// `Screencast.start({ path })` with it before sending anything.
pub(crate) const RECORD_REFUSED: &str = "Neo does not record screencast video: Playwright encodes the frames to WebM with ffmpeg, which Neo has no counterpart of; use onFrame to receive the frames";

/// Neo: the `Runtime`'s screencast frames. browseros-cdp delivers one event method on one
/// dedicated channel per client (`events_targeted`, which replaces an earlier one), so the
/// first screencast a run starts takes `Page.screencastFrame` for the whole `Runtime`, and
/// each frame goes to the tab session whose screencast sent it; a subscription per run would
/// cut the previous run's frames off. Frames stay off the shared broadcast ring, where
/// ~100 KB JPEGs would pin memory and a lagging reader would lose a frame the browser waits
/// to see acknowledged. BrowserOS's live view reads its frames on a socket of its own
/// (claw-server `services/capture/link.rs`), never this one.
#[derive(Clone, Default)]
pub(crate) struct ScreencastFrames(Arc<Routes>);

#[derive(Default)]
struct Routes {
    sessions: Mutex<HashMap<SessionId, mpsc::UnboundedSender<CdpEvent>>>,
    /// Whether the router task holds the client's `Page.screencastFrame` channel.
    listening: Mutex<bool>,
}

impl ScreencastFrames {
    /// `session`'s frames from now on, replacing an earlier route of it. The first route takes
    /// the client's frames for the `Runtime`.
    fn route(&self, cdp: &CdpClient, session: &SessionId) -> mpsc::UnboundedReceiver<CdpEvent> {
        let (sender, frames) = mpsc::unbounded_channel();
        if let Ok(mut sessions) = self.0.sessions.lock() {
            // A route whose screencast stopped, or whose tab closed, is dropped here at the latest.
            sessions.retain(|_, sender| !sender.is_closed());
            sessions.insert(session.clone(), sender);
        }
        self.listen(cdp);
        frames
    }

    /// Takes `Page.screencastFrame` on `cdp` once, for as long as the client delivers it and
    /// the `Runtime` lives.
    fn listen(&self, cdp: &CdpClient) {
        let Ok(mut listening) = self.0.listening.lock() else {
            return;
        };
        if *listening {
            return;
        }
        *listening = true;
        let mut frames = cdp.events_targeted(FRAME_EVENT);
        let routes = Arc::downgrade(&self.0);
        tokio::spawn(async move {
            while let Some(frame) = frames.recv().await {
                let Some(routes) = routes.upgrade() else {
                    return;
                };
                deliver(&routes, frame);
            }
            // The client replaced or dropped the channel: the next route takes it again.
            if let Some(routes) = routes.upgrade()
                && let Ok(mut listening) = routes.listening.lock()
            {
                *listening = false;
            }
        });
    }
}

/// Neo: one frame to the route of the session that sent it; a frame no route wants (its
/// screencast stopped) is dropped, and so is a route whose receiver went.
fn deliver(routes: &Routes, frame: CdpEvent) {
    let Some(session) = frame.session_id.clone() else {
        return;
    };
    let Ok(mut sessions) = routes.sessions.lock() else {
        return;
    };
    if let Some(sender) = sessions.get(&session)
        && sender.send(frame).is_err()
    {
        sessions.remove(&session);
    }
}

/// Neo: the run's screencast of a tab, if one runs, and its frame count.
#[derive(Default)]
pub(crate) struct Screencast {
    running: Mutex<Option<Running>>,
    /// The id of the last frame sent to the realm, counted across the run's screencasts.
    ///
    /// Playwright: `server/dispatchers/pageDispatcher.ts` `PageDispatcher.screencastStart`: `const frameId = ++this._screencastFrameId;`
    last_frame: AtomicI64,
}

/// Neo: a screencast sending frames (Playwright keeps none for a start that wants no frames).
struct Running {
    /// Stops the frame forwarding.
    forwarding: CancellationToken,
    /// Frames sent to the realm and not acknowledged yet: the run's frame id → the browser's.
    unacked: Arc<Mutex<HashMap<i64, i64>>>,
}

/// Neo: serves the page's `screencast*` commands; the overlay ones are `overlay.rs`.
pub(crate) async fn serve(
    tabs: &Tabs,
    page: &Arc<PageSession>,
    call: PageCall,
    progress: &Progress,
) -> Result<Option<Value>, CallError> {
    match call {
        PageCall::ScreencastStart(params) => {
            start(tabs, page, &params, progress).await?;
            encode_result::<PageScreencastStartParams>(&PageScreencastStartResult {
                artifact: None,
            })
        }
        PageCall::ScreencastFrameAck(params) => {
            ack(page, params.frame_id).await;
            encode_result::<PageScreencastFrameAckParams>(&PageScreencastFrameAckResult {})
        }
        PageCall::ScreencastStop(_) => {
            stop(page).await;
            encode_result::<PageScreencastStopParams>(&PageScreencastStopResult {})
        }
        PageCall::ScreencastShowOverlay(params) => {
            let id = overlay::show(page, params.html, params.duration, progress).await?;
            encode_result::<PageScreencastShowOverlayParams>(&PageScreencastShowOverlayResult {
                id,
            })
        }
        PageCall::ScreencastRemoveOverlay(params) => {
            overlay::remove(page, &params.id).await;
            encode_result::<PageScreencastRemoveOverlayParams>(
                &PageScreencastRemoveOverlayResult {},
            )
        }
        PageCall::ScreencastChapter(params) => {
            overlay::chapter(
                page,
                &params.title,
                params.description.as_deref(),
                params.duration,
                progress,
            )
            .await?;
            encode_result::<PageScreencastChapterParams>(&PageScreencastChapterResult {})
        }
        PageCall::ScreencastSetOverlayVisible(params) => {
            overlay::set_visible(page, params.visible).await;
            encode_result::<PageScreencastSetOverlayVisibleParams>(
                &PageScreencastSetOverlayVisibleResult {},
            )
        }
        // Neo: the SDK refuses `showActions` (FU-T-1), so no action is ever annotated, and hiding
        // the annotations has nothing to do.
        PageCall::ScreencastHideActions(_) => {
            encode_result::<PageScreencastHideActionsParams>(&PageScreencastHideActionsResult {})
        }
        _ => Err(PlaywrightError::error("neo-runtime: not a screencast command").into()),
    }
}

/// Serves `Page.screencastStart`.
///
/// Playwright: `server/dispatchers/pageDispatcher.ts` `PageDispatcher.screencastStart`: `throw new Error('Screencast is already running');`
async fn start(
    tabs: &Tabs,
    page: &Arc<PageSession>,
    params: &PageScreencastStartParams,
    progress: &Progress,
) -> Result<(), CallError> {
    let forwarding = CancellationToken::new();
    let unacked = Arc::new(Mutex::new(HashMap::new()));
    {
        let mut running = page.screencast.running.lock().map_err(|_| state_error())?;
        if running.is_some() {
            return Err(PlaywrightError::error("Screencast is already running").into());
        }
        if let Some(fps) = params.fps
            && fps <= 0
        {
            // Playwright: `server/dispatchers/pageDispatcher.ts` `PageDispatcher.screencastStart`: `if (params.fps !== undefined && params.fps <= 0)`
            return Err(PlaywrightError::error(format!(
                "\"fps\" must be a positive number, got {fps}"
            ))
            .into());
        }
        if params.record == Some(true) {
            return Err(PlaywrightError::error(RECORD_REFUSED).into());
        }
        // Playwright: `server/dispatchers/pageDispatcher.ts` `PageDispatcher.screencastStart`: `if (params.sendFrames) {`
        if params.send_frames != Some(true) {
            return Ok(());
        }
        *running = Some(Running {
            forwarding: forwarding.clone(),
            unacked: Arc::clone(&unacked),
        });
    }
    // Routed before the screencast starts, so its first frame is never missed.
    let frames = tabs
        .screencast_frames()
        .route(page.events.cdp(), page.session_id());
    forward(page, frames, forwarding.clone(), unacked);
    let size = params.size.as_ref().map(|size| (size.width, size.height));
    // The browser's answer is awaited but not judged, as Playwright's `.catch(() => {})`: a
    // screencast that did not start sends no frames.
    let sent = progress
        .race(page.send(
            "Page.startScreencast",
            start_params(size, params.quality, params.fps),
        ))
        .await;
    if let Err(error) = sent {
        forwarding.cancel();
        if let Ok(mut running) = page.screencast.running.lock() {
            *running = None;
        }
        return Err(error);
    }
    Ok(())
}

/// Neo: `Page.startScreencast`'s parameters for Playwright's `size`, `quality` and `fps`.
///
/// Playwright: `server/screencast.ts` `Screencast._startScreencast`: `width: this._size.width & ~1,`
fn start_params(size: Option<(i64, i64)>, quality: Option<i64>, fps: Option<i64>) -> Value {
    let (width, height) = size.unwrap_or_else(|| {
        let (width, height) = DEFAULT_VIEWPORT;
        #[expect(
            clippy::cast_precision_loss,
            reason = "a viewport side is far below 2^52"
        )]
        let scale = (MAX_SIDE / width.max(height) as f64).min(1.0);
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_precision_loss,
            reason = "Math.floor of a viewport side scaled down, as Playwright's"
        )]
        let fitted = |side: i64| (side as f64 * scale).floor() as i64;
        (fitted(width), fitted(height))
    });
    // Playwright: `server/chromium/crPage.ts` `CRPage.startScreencast`: `maxFramesInFlight: (options.fps ?? 0) > 30 ? 10 : 5,`
    let in_flight = if fps.is_some_and(|fps| fps > 30) {
        10
    } else {
        5
    };
    json!({
        "format": "jpeg",
        "quality": quality.unwrap_or(DEFAULT_QUALITY),
        "maxWidth": width & !1,
        "maxHeight": height & !1,
        "maxFramesInFlight": in_flight,
    })
}

/// Neo: sends the tab session's frames to the realm until `forwarding` stops, the tab closes
/// or the route goes; a frame the realm does not take is acknowledged at once.
fn forward(
    page: &Arc<PageSession>,
    mut frames: mpsc::UnboundedReceiver<CdpEvent>,
    forwarding: CancellationToken,
    unacked: Arc<Mutex<HashMap<i64, i64>>>,
) {
    let tab = Arc::clone(page);
    page.spawn_while_open(async move {
        loop {
            let frame = tokio::select! {
                biased;
                () = forwarding.cancelled() => return,
                frame = frames.recv() => frame,
            };
            let Some(frame) = frame else {
                return;
            };
            let Some(browser_id) = frame.params["sessionId"].as_i64() else {
                continue;
            };
            let id = tab.screencast.last_frame.fetch_add(1, Ordering::Relaxed) + 1;
            if let Ok(mut unacked) = unacked.lock() {
                unacked.insert(id, browser_id);
            }
            if !tab.events.screencast_frame(&frame_event(id, &frame.params)) {
                ack(&tab, id).await;
            }
        }
    });
}

/// Neo: Playwright's `screencastFrame` event for frame `id`: the JPEG as given (base64, the wire
/// form of `binary`), the frame's swap time in ms since the epoch, and the viewport the browser
/// reports.
///
/// Playwright: `server/chromium/crPage.ts` `FrameSession._onScreencastFrame`: `frameSwapWallTime: payload.metadata.timestamp ? payload.metadata.timestamp * 1000 : Date.now(),`
fn frame_event(id: i64, params: &Value) -> Value {
    let metadata = &params["metadata"];
    let timestamp = metadata["timestamp"]
        .as_f64()
        .filter(|seconds| *seconds != 0.0)
        .map_or_else(now_ms, |seconds| seconds * 1000.0);
    json!({
        "frameId": id,
        "data": params["data"],
        "timestamp": timestamp,
        "viewportWidth": metadata["deviceWidth"],
        "viewportHeight": metadata["deviceHeight"],
    })
}

fn now_ms() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0.0, |elapsed| elapsed.as_secs_f64() * 1000.0)
}

/// Serves `Page.screencastFrameAck`: the browser may send the next frame. A frame this run's
/// screencast did not send, or one sent before `stop`, is ignored, as Playwright ignores an
/// unknown frame id.
///
/// Playwright: `server/dispatchers/pageDispatcher.ts` `PageDispatcher.screencastFrameAck`: `if (!promise)`
async fn ack(page: &PageSession, frame_id: i64) {
    let browser_id = page.screencast.running.lock().ok().and_then(|running| {
        running.as_ref().and_then(|running| {
            running
                .unacked
                .lock()
                .ok()
                .and_then(|mut unacked| unacked.remove(&frame_id))
        })
    });
    if let Some(browser_id) = browser_id {
        let _acked = page
            .send("Page.screencastFrameAck", json!({ "sessionId": browser_id }))
            .await;
    }
}

/// Serves `Page.screencastStop`: the tab session's screencast stops; with none running it
/// answers at once.
///
/// Playwright: `server/dispatchers/pageDispatcher.ts` `PageDispatcher.screencastStop`: `this._page.screencast.removeClient(client);`
async fn stop(page: &PageSession) {
    if halt(page) {
        stop_browser(page).await;
    }
}

/// Neo: at the run's end, before the run's tasks are awaited (each frame forwarder is one), the
/// tab's frames stop reaching the realm; whether a screencast ran.
pub(crate) fn halt(page: &PageSession) -> bool {
    let running = page
        .screencast
        .running
        .lock()
        .ok()
        .and_then(|mut running| running.take());
    running.is_some_and(|running| {
        running.forwarding.cancel();
        true
    })
}

/// Neo: the browser's screencast on the tab session stops; its failure is ignored, as
/// Playwright's `_sendMayFail`.
///
/// Playwright: `server/chromium/crPage.ts` `CRPage.stopScreencast`: `this._mainFrameSession._client._sendMayFail('Page.stopScreencast').catch(() => {});`
pub(crate) async fn stop_browser(page: &PageSession) {
    let _stopped = page.send("Page.stopScreencast", json!({})).await;
}

fn state_error() -> CallError {
    PlaywrightError::error("neo-runtime: the tab's screencast cannot be read").into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tests_that_start_params_fit_playwrights_defaults_and_keep_sides_even() {
        assert_eq!(
            start_params(None, None, None),
            json!({"format": "jpeg", "quality": 90, "maxWidth": 800, "maxHeight": 600, "maxFramesInFlight": 5})
        );
        assert_eq!(
            start_params(Some((321, 201)), Some(40), Some(60)),
            json!({"format": "jpeg", "quality": 40, "maxWidth": 320, "maxHeight": 200, "maxFramesInFlight": 10})
        );
        assert_eq!(start_params(None, None, Some(30))["maxFramesInFlight"], 5);
    }

    #[test]
    fn tests_that_a_frame_carries_its_swap_time_and_the_reported_viewport() {
        let params = json!({
            "data": "/9j/AA==",
            "sessionId": 7,
            "metadata": {"timestamp": 1.5, "deviceWidth": 1279.5, "deviceHeight": 720},
        });
        assert_eq!(
            frame_event(3, &params),
            json!({"frameId": 3, "data": "/9j/AA==", "timestamp": 1500.0, "viewportWidth": 1279.5, "viewportHeight": 720})
        );
        let untimed = frame_event(4, &json!({"data": "", "metadata": {"timestamp": 0}}));
        assert!(untimed["timestamp"].as_f64().is_some_and(|ms| ms > 1.0e12));
    }

    #[test]
    fn tests_that_a_frame_reaches_only_its_sessions_route_and_a_gone_route_is_dropped() {
        let routes = Routes::default();
        let (sender, mut frames) = mpsc::unbounded_channel();
        let (gone, gone_frames) = mpsc::unbounded_channel();
        drop(gone_frames);
        if let Ok(mut sessions) = routes.sessions.lock() {
            sessions.insert(SessionId::from("ours"), sender);
            sessions.insert(SessionId::from("stopped"), gone);
        }
        let frame = |session: Option<&str>| CdpEvent {
            method: FRAME_EVENT.to_string(),
            params: json!({"sessionId": 1}),
            session_id: session.map(SessionId::from),
        };
        deliver(&routes, frame(Some("other")));
        deliver(&routes, frame(None));
        deliver(&routes, frame(Some("ours")));
        deliver(&routes, frame(Some("stopped")));
        assert_eq!(
            frames.try_recv().ok().and_then(|frame| frame.session_id),
            Some(SessionId::from("ours"))
        );
        assert!(frames.try_recv().is_err(), "another session's frame was routed here");
        assert!(
            routes
                .sessions
                .lock()
                .is_ok_and(|sessions| !sessions.contains_key(&SessionId::from("stopped")))
        );
    }
}
