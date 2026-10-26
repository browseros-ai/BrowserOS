//! Per-tab mobile mode: one fixed Android preset drawn at a device-preview zoom.
//!
//! Chromium keeps a page session's emulation across reloads, navigations and renderer swaps,
//! and clears it when the session ends. The page registry therefore keeps the applied preset
//! per PageId and re-applies it whenever the tab gets a new session. This module owns every
//! command, the undo and the input-scale registry that mouse dispatch reads.
use crate::{
    BrowserView, CdpConnection, CoreError, PageId, ProtocolSession, SessionId, TargetError,
    WindowId,
    pages::{PageInfo, PageManager, PageSession},
    target::page_access::PageAccess,
};
use serde_json::{Value, json};
use std::{
    fmt,
    sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError, Weak},
};

pub const MOBILE_WIDTH: i64 = 400;
pub const MOBILE_HEIGHT: i64 = 906;
pub const MOBILE_DEVICE_SCALE_FACTOR: f64 = 2.0;
pub const MIN_ZOOM: f64 = 25.0;
pub const MAX_ZOOM: f64 = 400.0;
const DEFAULT_ZOOM: f64 = 100.0;

const SET_USER_AGENT: &str = "Emulation.setUserAgentOverride";
const SET_TOUCH: &str = "Emulation.setTouchEmulationEnabled";
const SET_METRICS: &str = "Emulation.setDeviceMetricsOverride";
const SET_VISIBLE_SIZE: &str = "Emulation.setVisibleSize";
const CLEAR_METRICS: &str = "Emulation.clearDeviceMetricsOverride";

/// Percent on the DevTools device-preview scale: 150 draws the page at 1.5x.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Zoom(f64);

impl Zoom {
    pub fn from_percent(percent: f64) -> Result<Self, MobileModeArgError> {
        if (MIN_ZOOM..=MAX_ZOOM).contains(&percent) {
            Ok(Self(percent))
        } else {
            Err(MobileModeArgError::ZoomOutOfRange)
        }
    }

    #[must_use]
    pub fn percent(self) -> f64 {
        self.0
    }

    #[must_use]
    pub fn scale(self) -> f64 {
        self.0 / 100.0
    }

    /// The tab's view, sized to the zoomed preset. Chromium would size it unscaled, which
    /// clips the page above 100% and leaves it in a corner below.
    #[must_use]
    pub fn visible_size(self) -> (i64, i64) {
        let scale = self.scale();
        (
            (MOBILE_WIDTH as f64 * scale).round() as i64,
            (MOBILE_HEIGHT as f64 * scale).round() as i64,
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MobileMode {
    Desktop,
    Mobile(Zoom),
}

impl MobileMode {
    /// The tool's arguments, validated before anything reaches the browser.
    pub fn from_request(enabled: bool, zoom: Option<f64>) -> Result<Self, MobileModeArgError> {
        match (enabled, zoom) {
            (false, None) => Ok(Self::Desktop),
            (false, Some(_)) => Err(MobileModeArgError::ZoomWithoutMobile),
            (true, zoom) => Ok(Self::Mobile(Zoom::from_percent(
                zoom.unwrap_or(DEFAULT_ZOOM),
            )?)),
        }
    }

    #[must_use]
    pub fn zoom(self) -> Option<Zoom> {
        match self {
            Self::Desktop => None,
            Self::Mobile(zoom) => Some(zoom),
        }
    }

    #[must_use]
    pub fn visible_size(self) -> Option<(i64, i64)> {
        self.zoom().map(Zoom::visible_size)
    }
}

/// Whether an enable also fits the tab's Chrome window to the zoomed preset.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum WindowFit {
    #[default]
    Keep,
    Resize,
}

/// One validated toggle: the tool's arguments, checked before anything reaches the browser.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MobileModeRequest {
    pub mode: MobileMode,
    pub window: WindowFit,
}

impl MobileModeRequest {
    pub fn new(
        enabled: bool,
        zoom: Option<f64>,
        resize_window: Option<bool>,
    ) -> Result<Self, MobileModeArgError> {
        let window = match (enabled, resize_window.unwrap_or(false)) {
            (_, false) => WindowFit::Keep,
            (true, true) => WindowFit::Resize,
            (false, true) => return Err(MobileModeArgError::ResizeWindowWithoutMobile),
        };
        Ok(Self {
            mode: MobileMode::from_request(enabled, zoom)?,
            window,
        })
    }
}

impl From<MobileMode> for MobileModeRequest {
    fn from(mode: MobileMode) -> Self {
        Self {
            mode,
            window: WindowFit::Keep,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum MobileModeArgError {
    #[error("zoom must be a number from 25 to 400")]
    ZoomOutOfRange,
    #[error("zoom is only accepted with enabled: true")]
    ZoomWithoutMobile,
    #[error("resizeWindow is only accepted with enabled: true; disabling restores the window")]
    ResizeWindowWithoutMobile,
}

/// A Chrome window's outer bounds in DIP, including its tab strip, toolbar and frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct WindowBounds {
    pub left: i64,
    pub top: i64,
    pub width: i64,
    pub height: i64,
}

/// What an opt-in window fit, or the restore on disable, did to the tab's window.
#[derive(Debug, Clone, PartialEq)]
pub enum WindowOutcome {
    /// Resized. `exact` says per axis whether the content area now matches `requested`;
    /// Chrome's minimum window size or the screen can keep it larger or smaller.
    Fitted {
        window: WindowId,
        bounds: WindowBounds,
        requested: (i64, i64),
        exact: (bool, bool),
    },
    NotFitted {
        window: Option<WindowId>,
        reason: String,
    },
    /// Disable put the window back to the bounds this tool found it at.
    Restored {
        window: WindowId,
        bounds: WindowBounds,
    },
    NotRestored {
        window: WindowId,
        reason: String,
    },
}

/// What a failed change left the tab in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MobileModeState {
    Desktop,
    Previous,
    Unknown,
}

impl fmt::Display for MobileModeState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Desktop => "desktop (every mobile setting was cleared)",
            Self::Previous => "previous (the earlier mobile preset was restored)",
            Self::Unknown => {
                "unknown (the tab's settings could not be confirmed; call toggleMobileMode again)"
            }
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct MobileModeOutcome {
    pub page: PageId,
    pub mode: MobileMode,
    /// The requested state differs from the one recorded for this tab.
    pub changed: bool,
    /// The loaded document predates the change; only a reload gets server-chosen content.
    pub reload_required: bool,
    /// Present when a fit was requested, or when disable found a fitted window.
    pub window: Option<WindowOutcome>,
}

impl fmt::Display for WindowOutcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Fitted {
                window,
                bounds,
                requested: (width, height),
                exact,
            } => {
                write!(
                    f,
                    "window {window} resized to {}x{} (outer size, including Chrome's tab strip, toolbar and frame; every tab in it shares the size)",
                    bounds.width, bounds.height
                )?;
                match exact {
                    (true, true) => write!(f, "; its content area is exactly {width}x{height}"),
                    (false, false) => write!(
                        f,
                        "; Chrome's minimum window size or the screen kept its content area from reaching {width}x{height}"
                    ),
                    (false, true) => write!(
                        f,
                        "; its content area is {height} tall, but Chrome's minimum window size or the screen kept its width from reaching {width}"
                    ),
                    (true, false) => write!(
                        f,
                        "; its content area is {width} wide, but the screen or Chrome kept its height from reaching {height}"
                    ),
                }
            }
            Self::NotFitted { reason, .. } => write!(f, "window not resized: {reason}"),
            Self::Restored { window, bounds } => write!(
                f,
                "window {window} restored to {}x{} at ({}, {})",
                bounds.width, bounds.height, bounds.left, bounds.top
            ),
            Self::NotRestored { window, reason } => {
                write!(f, "window {window} not restored: {reason}")
            }
        }
    }
}

/// A window this tool resized: restored on disable only while it still has `applied`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct FittedWindow {
    window: WindowId,
    original: WindowBounds,
    applied: WindowBounds,
}

/// The preset applied to one PageId. Re-applying it on a new session needs no version read.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct MobileRecord {
    zoom: Zoom,
    user_agent: String,
    metadata: Value,
    window: Option<FittedWindow>,
}

impl MobileRecord {
    pub(crate) fn new(major: u32, zoom: Zoom) -> Self {
        Self {
            zoom,
            user_agent: format!(
                "Mozilla/5.0 (Linux; Android 10; K) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/{major}.0.0.0 Mobile Safari/537.36"
            ),
            // No brands or full versions: Chromium fills them from the running browser, so
            // they always agree with the UA's major version.
            metadata: json!({"platform":"Android","platformVersion":"10.0.0","architecture":"",
                "model":"K","mobile":true,"formFactors":["Mobile"]}),
            window: None,
        }
    }

    pub(crate) fn zoom(&self) -> Zoom {
        self.zoom
    }

    fn commands(&self) -> [(&'static str, Value); 4] {
        let (width, height) = self.zoom.visible_size();
        [
            (
                SET_USER_AGENT,
                json!({"userAgent":self.user_agent,"userAgentMetadata":self.metadata}),
            ),
            (SET_TOUCH, json!({"enabled":true,"maxTouchPoints":1})),
            (
                SET_METRICS,
                json!({"width":MOBILE_WIDTH,"height":MOBILE_HEIGHT,
                    "deviceScaleFactor":MOBILE_DEVICE_SCALE_FACTOR,"mobile":true,
                    "scale":self.zoom.scale(),"screenWidth":MOBILE_WIDTH,
                    "screenHeight":MOBILE_HEIGHT,
                    "screenOrientation":{"type":"portraitPrimary","angle":0},
                    "dontSetVisibleSize":true}),
            ),
            (SET_VISIBLE_SIZE, json!({"width":width,"height":height})),
        ]
    }
}

/// The UA major comes from the browser process (`product` is "Chrome/<version>"), never from
/// page JavaScript, which the page controls.
pub(crate) fn browser_major(version: &Value) -> Result<u32, CoreError> {
    let product = version["product"].as_str().unwrap_or_default();
    product
        .split_once('/')
        .filter(|(name, _)| matches!(*name, "Chrome" | "HeadlessChrome"))
        .and_then(|(_, version)| version.split('.').next())
        .and_then(|major| major.parse::<u32>().ok())
        .filter(|major| *major > 0)
        .ok_or_else(|| {
            CoreError::Message(format!(
                "mobile_mode_failed: Browser.getVersion reported product {product:?}, not Chrome/<version>; nothing was changed"
            ))
        })
}

pub(crate) struct StepFailure {
    step: &'static str,
    error: CoreError,
}

impl StepFailure {
    fn into_error(self, state: MobileModeState) -> CoreError {
        CoreError::MobileMode {
            step: self.step.into(),
            state,
            detail: self.error.to_string(),
        }
    }
}

impl fmt::Display for StepFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} failed: {}", self.step, self.error)
    }
}

/// Sends the preset in its fixed order and stops at the first failure.
pub(crate) async fn apply(
    session: &ProtocolSession,
    record: &MobileRecord,
) -> Result<(), StepFailure> {
    for (step, params) in record.commands() {
        session
            .send_value(step, params)
            .await
            .map_err(|error| StepFailure { step, error })?;
    }
    Ok(())
}

/// Clears exactly the settings `apply` owns, sending every clear even after a failure. Clearing
/// the metrics override also restores the view size; focus emulation is not ours to touch.
pub(crate) async fn clear(session: &ProtocolSession) -> Result<(), StepFailure> {
    let mut failed = None;
    for (step, params) in [
        (CLEAR_METRICS, json!({})),
        (SET_TOUCH, json!({"enabled":false})),
        (SET_USER_AGENT, json!({"userAgent":""})),
    ] {
        if let Err(error) = session.send_value(step, params).await {
            failed.get_or_insert(StepFailure { step, error });
        }
    }
    failed.map_or(Ok(()), Err)
}

/// One toggle request with its admitted page session. `record` is the preset to apply, or None
/// to restore desktop.
pub(crate) struct Toggle {
    pub(crate) view: BrowserView,
    pub(crate) pages: Arc<PageManager>,
    pub(crate) page: PageId,
    pub(crate) session: PageSession,
    pub(crate) access: Option<Arc<PageAccess>>,
    pub(crate) record: Option<MobileRecord>,
    pub(crate) window: WindowFit,
}

/// Run as an owned task so a dropped caller cannot stop it between commands.
pub(crate) async fn toggle(request: Toggle) -> Result<MobileModeOutcome, CoreError> {
    let Toggle {
        view,
        pages,
        page,
        session,
        access,
        record,
        window,
    } = request;
    // A screenshot saves the emulation and view size and restores them afterwards, so a
    // change made during a capture would be undone by it.
    let capture_lock = crate::screenshot::screenshot_mutex(&session.session);
    let _capture = capture_lock.lock().await;
    let protocol = &session.session;
    let previous = pages.mobile_record(&page).await;
    let mut outcome = MobileModeOutcome {
        page: page.clone(),
        mode: record.as_ref().map_or(MobileMode::Desktop, |record| {
            MobileMode::Mobile(record.zoom)
        }),
        changed: previous.as_ref().map(MobileRecord::zoom)
            != record.as_ref().map(MobileRecord::zoom),
        reload_required: !matches!(session.url.as_str(), "" | "about:blank"),
        window: None,
    };
    let Some(mut record) = record else {
        // Desktop is the desired state whatever the clears report.
        let fitted = pages
            .take_mobile_record(&page)
            .await
            .and_then(|record| record.window);
        forget_input_scale(protocol.connection(), &session.session_id);
        let cleared = clear(protocol).await;
        if let Some(fitted) = fitted {
            outcome.window = Some(restore_window(&view, fitted).await);
        }
        return match cleared {
            Ok(()) => Ok(outcome),
            Err(failure) => {
                let mut error = failure.into_error(MobileModeState::Unknown);
                if let (CoreError::MobileMode { detail, .. }, Some(window)) =
                    (&mut error, &outcome.window)
                {
                    detail.push_str(&format!("; {window}"));
                }
                Err(error)
            }
        };
    };
    let epoch = protocol.connection().connection_epoch();
    if let Err(failure) = apply(protocol, &record).await {
        // Chromium stores the UA before it validates the metadata, so the undo re-sends a
        // complete state, including the step that failed.
        let restored = match &previous {
            Some(previous) => apply(protocol, previous)
                .await
                .is_ok()
                .then_some(MobileModeState::Previous),
            None => clear(protocol)
                .await
                .is_ok()
                .then_some(MobileModeState::Desktop),
        };
        return Err(failure.into_error(restored.unwrap_or(MobileModeState::Unknown)));
    }
    let info = confirm_target(&pages, &page, &session, access.as_deref())
        .await
        .map_err(|error| CoreError::MobileMode {
            step: "page target check".into(),
            state: MobileModeState::Unknown,
            detail: error.to_string(),
        })?;
    record.window = previous.and_then(|previous| previous.window);
    if window == WindowFit::Resize {
        let (fit, fitted) =
            fit_window(&view, protocol, info.window_id, record.zoom, record.window).await;
        outcome.window = Some(fit);
        record.window = fitted;
    }
    let scale = record.zoom.scale();
    pages.set_mobile_record(page, record).await;
    if protocol.connection().connection_epoch() == epoch {
        register_input_scale(protocol.connection(), epoch, &session.session_id, scale);
    }
    Ok(outcome)
}

/// Success is only claimed for the target the commands reached.
async fn confirm_target(
    pages: &PageManager,
    page: &PageId,
    session: &PageSession,
    access: Option<&PageAccess>,
) -> Result<PageInfo, CoreError> {
    if let Some(access) = access {
        return access.fresh().await;
    }
    match pages.get_info(page.clone()).await {
        Some(info) if info.target_id == session.target_id => Ok(info),
        _ => Err(TargetError::PageTargetChanged(page.clone()).into()),
    }
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct NativeWindowBounds {
    left: i64,
    top: i64,
    width: i64,
    height: i64,
    window_state: Option<String>,
}

async fn read_window(
    view: &BrowserView,
    window: WindowId,
) -> Result<(WindowBounds, String), StepFailure> {
    const STEP: &str = "Browser.getWindowBounds";
    let reply = view
        .mobile_window_command(window, STEP, json!({}))
        .await
        .map_err(|error| StepFailure { step: STEP, error })?;
    let native: NativeWindowBounds =
        serde_json::from_value(reply["bounds"].clone()).map_err(|err| StepFailure {
            step: STEP,
            error: CoreError::Message(err.to_string()),
        })?;
    Ok((
        WindowBounds {
            left: native.left,
            top: native.top,
            width: native.width,
            height: native.height,
        },
        native.window_state.unwrap_or_else(|| "normal".into()),
    ))
}

async fn set_contents(
    view: &BrowserView,
    window: WindowId,
    width: i64,
    height: i64,
) -> Result<(), StepFailure> {
    const STEP: &str = "Browser.setContentsSize";
    view.mobile_window_command(window, STEP, json!({"width":width,"height":height}))
        .await
        .map(|_| ())
        .map_err(|error| StepFailure { step: STEP, error })
}

/// Fits the tab's whole Chrome window to the zoomed preset with Chrome's own content-size
/// command, which keeps the tab strip, toolbar and frame. Never activates, moves or restores
/// the window. Returns the fit to remember: the new one, or the earlier one when nothing moved.
async fn fit_window(
    view: &BrowserView,
    protocol: &ProtocolSession,
    window: Option<WindowId>,
    zoom: Zoom,
    previous: Option<FittedWindow>,
) -> (WindowOutcome, Option<FittedWindow>) {
    let Some(window) = window else {
        let reason = "the tab has no window".to_string();
        return (WindowOutcome::NotFitted { window, reason }, previous);
    };
    let not_fitted = |reason: String| WindowOutcome::NotFitted {
        window: Some(window),
        reason,
    };
    let (before, state) = match read_window(view, window).await {
        Ok(read) => read,
        Err(failure) => {
            return (
                not_fitted(format!("{failure}; nothing was resized")),
                previous,
            );
        }
    };
    if state != "normal" {
        let reason = format!(
            "window {window} is {state}; Chrome only fits a normal window, so restore it to its normal size and call again (nothing was resized)"
        );
        return (not_fitted(reason), previous);
    }
    // A later fit of the same, untouched window keeps the bounds it had before the first one.
    let original = previous
        .filter(|fit| fit.window == window && fit.applied == before)
        .map_or(before, |fit| fit.original);
    let (width, height) = zoom.visible_size();
    let resized = async {
        set_contents(view, window, width, height).await?;
        let (fitted, _) = read_window(view, window).await?;
        // A one-pixel probe shows per axis whether Chrome's minimum size or the screen
        // limited the window: an exact fit grows by one, a clamped axis does not.
        set_contents(view, window, width + 1, height + 1).await?;
        let (probe, _) = read_window(view, window).await?;
        set_contents(view, window, width, height).await?;
        let (applied, _) = read_window(view, window).await?;
        Ok::<_, StepFailure>((fitted, probe, applied))
    }
    .await;
    match resized {
        Ok((fitted, probe, applied)) => {
            // Resizing the window resets the tab's view to the content area. Re-send the
            // zoomed view so a clamped window does not show the page wider than its preset.
            let _ = protocol
                .send_value(SET_VISIBLE_SIZE, json!({"width":width,"height":height}))
                .await;
            (
                WindowOutcome::Fitted {
                    window,
                    bounds: applied,
                    requested: (width, height),
                    exact: (
                        probe.width == fitted.width + 1,
                        probe.height == fitted.height + 1,
                    ),
                },
                Some(FittedWindow {
                    window,
                    original,
                    applied,
                }),
            )
        }
        Err(failure) => (
            not_fitted(format!(
                "{failure}; the window may have changed size and is not restored automatically"
            )),
            previous,
        ),
    }
}

/// Restores a fitted window, but never over a later resize, move or state change by anyone.
async fn restore_window(view: &BrowserView, fitted: FittedWindow) -> WindowOutcome {
    let window = fitted.window;
    let not_restored = |reason: String| WindowOutcome::NotRestored { window, reason };
    match read_window(view, window).await {
        Ok((bounds, state)) if bounds == fitted.applied && state == "normal" => {}
        Ok(_) => {
            return not_restored(format!(
                "window {window} was resized, moved or changed state after the fit; left as is"
            ));
        }
        Err(failure) => return not_restored(format!("{failure}; left as is")),
    }
    let restored = async {
        view.mobile_window_command(
            window,
            "Browser.setWindowBounds",
            json!({"bounds":fitted.original}),
        )
        .await
        .map_err(|error| StepFailure {
            step: "Browser.setWindowBounds",
            error,
        })?;
        read_window(view, window).await
    }
    .await;
    match restored {
        Ok((bounds, _)) => WindowOutcome::Restored { window, bounds },
        Err(failure) => not_restored(format!("{failure}; the window may have changed size")),
    }
}

/// A zoomed page session's mouse scale. Keyed by connection, socket epoch and session id, so no
/// other browser, connection incarnation or session can pick up a page's zoom.
struct InputScale {
    // Holding the weak handle keeps the allocation, so the address is never reused while listed.
    connection: Weak<dyn CdpConnection>,
    epoch: u64,
    session: SessionId,
    scale: f64,
}

fn input_scales() -> MutexGuard<'static, Vec<InputScale>> {
    static SCALES: OnceLock<Mutex<Vec<InputScale>>> = OnceLock::new();
    SCALES
        .get_or_init(|| Mutex::new(Vec::new()))
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
}

fn same_connection(entry: &InputScale, connection: &Arc<dyn CdpConnection>) -> bool {
    std::ptr::addr_eq(entry.connection.as_ptr(), Arc::as_ptr(connection))
}

/// Chromium does not correct CDP mouse coordinates for the emulation zoom and Blink divides
/// them by it, so CSS-px coordinates must be multiplied on the way out. 1.0 everywhere else.
pub(crate) fn input_scale(session: &ProtocolSession) -> f64 {
    let Some(id) = session.session_id() else {
        return 1.0;
    };
    let connection = session.connection();
    let epoch = connection.connection_epoch();
    input_scales()
        .iter()
        .find(|entry| {
            same_connection(entry, connection) && entry.epoch == epoch && &entry.session == id
        })
        .map_or(1.0, |entry| entry.scale)
}

pub(crate) fn register_input_scale(
    connection: &Arc<dyn CdpConnection>,
    epoch: u64,
    session: &SessionId,
    scale: f64,
) {
    let mut scales = input_scales();
    scales.retain(|entry| {
        entry.connection.strong_count() > 0
            && !(same_connection(entry, connection) && &entry.session == session)
    });
    scales.push(InputScale {
        connection: Arc::downgrade(connection),
        epoch,
        session: session.clone(),
        scale,
    });
}

pub(crate) fn forget_input_scale(connection: &Arc<dyn CdpConnection>, session: &SessionId) {
    input_scales()
        .retain(|entry| !(same_connection(entry, connection) && &entry.session == session));
}

/// Session ids belong to one socket; a reconnect retires every earlier registration.
pub(crate) fn forget_input_scales_before(connection: &Arc<dyn CdpConnection>, epoch: u64) {
    input_scales().retain(|entry| !(same_connection(entry, connection) && entry.epoch != epoch));
}

#[cfg(test)]
mod tests;
