//! Neo: `Page.screencast` (Playwright's `client/screencast.ts` over `server/screencast.ts` and
//! the page dispatcher): a CDP screencast of the tab, whose JPEG frames reach the realm's
//! `onFrame` as `screencastFrame` events. The browser gets a frame's acknowledgement only once
//! the realm acknowledged it, so a slow `onFrame` slows the stream rather than queue it, as
//! Playwright's client acknowledges after its callback. The overlays are `overlay.rs`.
//!
//! ```text
//!  Page.screencastStart {size?, quality?, fps?, sendFrames?, record?}
//!    running → Error; fps ≤ 0 → Error; record → Error (Neo has no video encoder)
//!    sendFrames → Page.startScreencast {jpeg, quality ?? 90, maxWidth × maxHeight = size, else
//!      800 × 600 fitted into 800, both even; maxFramesInFlight: fps > 30 ? 10 : 5}, on the tab's
//!      own session; each Page.screencastFrame → event screencastFrame {frameId, data,
//!      timestamp, viewportWidth, viewportHeight}, or, with nobody listening, its ack at once
//!  Page.screencastFrameAck {frameId}   → Page.screencastFrameAck; an unknown frame → nothing
//!  Page.screencastStop {}              → Page.stopScreencast; nothing running → nothing
//!  run end (`Dispatcher::finish`)      as Page.screencastStop, on every tab the run touched
//! ```
//!
//! Another CDP client's screencast of the same tab (BrowserOS's live view) runs on its own
//! session with its own frames and acknowledgements: starting, pacing or stopping this one
//! neither stops nor starves it.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use browseros_cdp::CdpEvent;
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
use tokio::sync::broadcast::error::RecvError;
use tokio_util::sync::CancellationToken;

use crate::overlay;
use crate::page::PageSession;
use crate::progress::{CallError, PlaywrightError, Progress};
use crate::dispatch::encode_result;

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

/// Neo: the reason `Screencast.start({ path })` is refused, as the SDK states it too.
pub(crate) const RECORD_REFUSED: &str = "Neo does not record screencast video: Playwright encodes it to WebM with ffmpeg, which Neo has no counterpart of; use onFrame for the frames";

/// Neo: the tab's screencast, if one runs.
#[derive(Default)]
pub(crate) struct Screencast {
    running: Mutex<Option<Running>>,
}

/// Neo: a started screencast. `frames` stops the frame forwarding; `None` when it sends no
/// frames (Playwright starts no browser screencast for a client that wants none).
struct Running {
    frames: Option<CancellationToken>,
    /// Frames sent to the realm and not acknowledged yet, by frame id (CDP's frame number).
    unacked: Arc<Mutex<BTreeSet<i64>>>,
}

/// Neo: serves the page's `screencast*` commands; the overlay ones are `overlay.rs`.
pub(crate) async fn serve(
    page: &Arc<PageSession>,
    call: PageCall,
    progress: &Progress,
) -> Result<Option<Value>, CallError> {
    match call {
        PageCall::ScreencastStart(params) => {
            start(page, &params, progress).await?;
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
    page: &Arc<PageSession>,
    params: &PageScreencastStartParams,
    progress: &Progress,
) -> Result<(), CallError> {
    if let Some(fps) = params.fps
        && fps <= 0
    {
        // Playwright: `server/dispatchers/pageDispatcher.ts` `PageDispatcher.screencastStart`: `if (params.fps !== undefined && params.fps <= 0)`
        return Err(
            PlaywrightError::error(format!("\"fps\" must be a positive number, got {fps}")).into(),
        );
    }
    if params.record == Some(true) {
        return Err(PlaywrightError::error(RECORD_REFUSED).into());
    }
    let unacked = Arc::new(Mutex::new(BTreeSet::new()));
    {
        let mut running = page.screencast.running.lock().map_err(|_| state_error())?;
        if running.is_some() {
            return Err(PlaywrightError::error("Screencast is already running").into());
        }
        *running = Some(Running {
            frames: None,
            unacked: Arc::clone(&unacked),
        });
    }
    if params.send_frames != Some(true) {
        return Ok(());
    }
    let stop = CancellationToken::new();
    // Subscribed before the screencast starts, so its first frame is never missed.
    forward(page, page.session.cdp.events(), stop.clone(), unacked);
    let size = params.size.as_ref().map(|size| (size.width, size.height));
    // The browser's answer is not awaited for its outcome, as Playwright's `.catch(() => {})`:
    // a screencast that did not start sends no frames. The call still ends with the run, the
    // tab or its deadline.
    let sent = progress
        .race(page.send(
            "Page.startScreencast",
            start_params(size, params.quality, params.fps),
        ))
        .await;
    if let Err(error) = sent {
        stop.cancel();
        clear(page);
        return Err(error);
    }
    if let Ok(mut running) = page.screencast.running.lock()
        && let Some(running) = running.as_mut()
    {
        running.frames = Some(stop);
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
    let in_flight = if fps.is_some_and(|fps| fps > 30) { 10 } else { 5 };
    json!({
        "format": "jpeg",
        "quality": quality.unwrap_or(DEFAULT_QUALITY),
        "maxWidth": width & !1,
        "maxHeight": height & !1,
        "maxFramesInFlight": in_flight,
    })
}

/// Neo: forwards the tab's own `Page.screencastFrame` events to the realm until `stop`, each
/// as Playwright's `screencastFrame`; a frame nobody listens for is acknowledged at once.
fn forward(
    page: &Arc<PageSession>,
    mut events: tokio::sync::broadcast::Receiver<CdpEvent>,
    stop: CancellationToken,
    unacked: Arc<Mutex<BTreeSet<i64>>>,
) {
    let tab = Arc::clone(page);
    let session = page.session.id.clone();
    page.session.tokio.spawn(async move {
        loop {
            let event = tokio::select! {
                () = stop.cancelled() => return,
                event = events.recv() => event,
            };
            let event = match event {
                Ok(event) => event,
                Err(RecvError::Lagged(_)) => continue,
                Err(RecvError::Closed) => return,
            };
            if event.method != "Page.screencastFrame" || event.session_id.as_ref() != Some(&session)
            {
                continue;
            }
            let Some(frame_id) = event.params["sessionId"].as_i64() else {
                continue;
            };
            if let Ok(mut unacked) = unacked.lock() {
                unacked.insert(frame_id);
            }
            if !tab.events.screencast_frame(&frame(frame_id, &event.params)) {
                ack(&tab, frame_id).await;
            }
        }
    });
}

/// Neo: Playwright's `screencastFrame` event for CDP frame `frame_id`: the JPEG as given (base64,
/// the wire form of `binary`), the frame's swap time in ms since the epoch, and its viewport.
///
/// Playwright: `server/chromium/crPage.ts` `FrameSession._onScreencastFrame`: `frameSwapWallTime: payload.metadata.timestamp ? payload.metadata.timestamp * 1000 : Date.now(),`
fn frame(frame_id: i64, params: &Value) -> Value {
    let metadata = &params["metadata"];
    let timestamp = metadata["timestamp"].as_f64().map_or_else(now_ms, |seconds| seconds * 1000.0);
    #[expect(
        clippy::cast_possible_truncation,
        reason = "a viewport side in DIP, far below i64's range"
    )]
    let side = |key: &str| metadata[key].as_f64().unwrap_or(0.0).round() as i64;
    json!({
        "frameId": frame_id,
        "data": params["data"],
        "timestamp": timestamp,
        "viewportWidth": side("deviceWidth"),
        "viewportHeight": side("deviceHeight"),
    })
}

fn now_ms() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0.0, |elapsed| elapsed.as_secs_f64() * 1000.0)
}

/// Serves `Page.screencastFrameAck`: the browser may send the next frame. A frame this
/// screencast did not send (an old one, after `stop`) is ignored, as Playwright ignores an
/// unknown frame id.
///
/// Playwright: `server/dispatchers/pageDispatcher.ts` `PageDispatcher.screencastFrameAck`: `if (!promise)`
async fn ack(page: &PageSession, frame_id: i64) {
    let known = page
        .screencast
        .running
        .lock()
        .ok()
        .and_then(|running| {
            running
                .as_ref()
                .and_then(|running| running.unacked.lock().ok().map(|mut unacked| unacked.remove(&frame_id)))
        })
        .unwrap_or(false);
    if known {
        let _acked = page
            .send("Page.screencastFrameAck", json!({ "sessionId": frame_id }))
            .await;
    }
}

/// Serves `Page.screencastStop`: the browser's screencast on this tab's session stops; nothing
/// running answers at once.
///
/// Playwright: `server/dispatchers/pageDispatcher.ts` `PageDispatcher.screencastStop`: `this._page.screencast.removeClient(client);`
async fn stop(page: &PageSession) {
    let Some(running) = page.screencast.running.lock().ok().and_then(|mut running| running.take())
    else {
        return;
    };
    if let Some(frames) = running.frames {
        frames.cancel();
        let _stopped = page.send("Page.stopScreencast", json!({})).await;
    }
}

/// Neo: at the run's end, as `Page.screencastStop`: no screencast outlives the run that started it.
pub(crate) async fn stop_at_run_end(page: &PageSession) {
    stop(page).await;
}

fn clear(page: &PageSession) {
    if let Ok(mut running) = page.screencast.running.lock() {
        *running = None;
    }
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
    }

    #[test]
    fn tests_that_a_frame_carries_its_swap_time_and_viewport() {
        let params = json!({
            "data": "/9j/AA==",
            "sessionId": 7,
            "metadata": {"timestamp": 1.5, "deviceWidth": 1279.6, "deviceHeight": 720.0},
        });
        assert_eq!(
            frame(7, &params),
            json!({"frameId": 7, "data": "/9j/AA==", "timestamp": 1500.0, "viewportWidth": 1280, "viewportHeight": 720})
        );
    }
}
