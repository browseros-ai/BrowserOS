//! Hosted viewport modes for one tab. Mobile and desktop emulate a fixed device and fit the
//! tab's window around it before each page operation; default leaves the tab natural.
//!
//! Hosts enable the feature by attaching one [`Viewports`] to their views; every entry point is
//! on [`BrowserView`], so admission and scope stay where they are.

mod device;
#[cfg(test)]
mod tests;
mod window;

use crate::{BrowserView, CoreError, PageId, SessionId, WindowId};
use device::Fixed;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};
use tokio::sync::{OnceCell, OwnedRwLockReadGuard, OwnedRwLockWriteGuard, RwLock};

pub const MIN_ZOOM: f64 = 25.0;
pub const MAX_ZOOM: f64 = 400.0;
pub const DEFAULT_ZOOM: f64 = 100.0;
/// Bounds for a requested device width or height, in CSS px.
pub const MIN_SIZE: i64 = 100;
pub const MAX_SIZE: i64 = 4096;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ViewportMode {
    #[default]
    Default,
    Mobile,
    Desktop,
}

/// A validated request. Zoom is a percentage of the mobile preview and exists only for mobile;
/// a size replaces a fixed mode's device size. Leaving either out means the mode's own value,
/// never the tab's previous one.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ViewportRequest {
    mode: ViewportMode,
    zoom: Option<f64>,
    size: Option<Size>,
}

impl ViewportRequest {
    pub fn new(mode: ViewportMode, zoom: Option<f64>) -> Result<Self, String> {
        match (mode, zoom) {
            (ViewportMode::Mobile, None) => Ok(Self {
                mode,
                zoom: Some(DEFAULT_ZOOM),
                size: None,
            }),
            (ViewportMode::Mobile, Some(zoom))
                if zoom.is_finite() && (MIN_ZOOM..=MAX_ZOOM).contains(&zoom) =>
            {
                Ok(Self {
                    mode,
                    zoom: Some(zoom),
                    size: None,
                })
            }
            (ViewportMode::Mobile, Some(_)) => Err(format!(
                "zoom must be a percentage from {MIN_ZOOM} to {MAX_ZOOM}"
            )),
            (_, Some(_)) => Err("zoom applies only to mobile".to_string()),
            (_, None) => Ok(Self {
                mode,
                zoom: None,
                size: None,
            }),
        }
    }

    #[must_use]
    pub fn mode(&self) -> ViewportMode {
        self.mode
    }

    /// The mobile preview zoom in percent; `None` for desktop and default.
    #[must_use]
    pub fn zoom(&self) -> Option<f64> {
        self.zoom
    }

    /// The device viewport in CSS px, for mobile or desktop.
    pub fn with_size(self, size: Size) -> Result<Self, String> {
        if self.mode == ViewportMode::Default {
            return Err("width and height apply only to mobile and desktop".to_string());
        }
        if ![size.width, size.height]
            .iter()
            .all(|side| (MIN_SIZE..=MAX_SIZE).contains(side))
        {
            return Err(format!(
                "width and height must be from {MIN_SIZE} to {MAX_SIZE} CSS px"
            ));
        }
        Ok(Self {
            size: Some(size),
            ..self
        })
    }

    /// The requested device size; `None` means the mode's own.
    #[must_use]
    pub fn size(&self) -> Option<Size> {
        self.size
    }

    fn fixed(self) -> Option<Fixed> {
        match self.mode {
            ViewportMode::Mobile => Some(Fixed::Mobile {
                zoom: self.zoom.unwrap_or(DEFAULT_ZOOM),
                size: self.size.unwrap_or(device::MOBILE_SIZE),
            }),
            ViewportMode::Desktop => Some(Fixed::Desktop {
                size: self.size.unwrap_or(device::DESKTOP_SIZE),
            }),
            ViewportMode::Default => None,
        }
    }
}

/// What the page and its window actually report after a change. Nothing here is copied from
/// the request: a value that did not take effect is reported as the page shows it.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ViewportReport {
    pub page: u32,
    pub mode: ViewportMode,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub zoom: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<Size>,
    pub viewport: PageMetrics,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub window: Option<WindowFit>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PageMetrics {
    pub width: f64,
    pub height: f64,
    pub screen_width: f64,
    pub screen_height: f64,
    pub device_scale_factor: f64,
    pub touch_points: i64,
    /// `navigator.userAgentData.mobile`; absent outside secure contexts.
    pub mobile: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Size {
    pub width: i64,
    pub height: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowBounds {
    pub left: i64,
    pub top: i64,
    pub width: i64,
    pub height: i64,
    pub window_state: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Dimension {
    Width,
    Height,
}

/// Why a window dimension could not match the preview: the screen's work area or the
/// browser's minimum window size. `Unknown` when the screen could not be read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum LimitedBy {
    Screen,
    Minimum,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowLimit {
    pub dimension: Dimension,
    pub by: LimitedBy,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowFit {
    pub window_id: i64,
    /// The preview size the window was fitted around, in DIP.
    pub contents: Size,
    /// The window's outer bounds after fitting, as the browser reports them.
    pub bounds: WindowBounds,
    /// Whether the contents area matches `contents` exactly in both dimensions.
    pub exact: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub limited: Vec<WindowLimit>,
    /// The window state it was restored from before fitting, when it was not normal.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub restored_from: Option<String>,
}

/// Per-process viewport state. A host creates one and attaches it to every view whose
/// calls may use the feature. PageIds are never reused within a process, so a closed tab's
/// entry can only go stale, never apply to another tab.
#[derive(Default)]
pub struct Viewports {
    tabs: Mutex<HashMap<PageId, Tab>>,
    windows: Mutex<HashMap<WindowId, Arc<RwLock<WindowSlot>>>>,
    chrome_version: OnceCell<String>,
}

/// A tab in a fixed mode. Default tabs have no entry.
#[derive(Debug, Clone)]
struct Tab {
    fixed: Fixed,
    /// The page session the emulation lives on; it dies with that session.
    applied: Option<SessionId>,
}

#[derive(Default)]
struct WindowSlot {
    /// Bounds this window had after its last fit, and the contents it was fitted for.
    fitted: Option<(Size, WindowBounds)>,
}

/// Holds the page's window for one operation, so no other tab's fit can change the window
/// while the operation reads, captures or acts. Dropping it releases the window.
/// A fixed tab's operation owns its window; default tabs beside it only exclude fits.
#[derive(Default)]
pub struct ViewportLease {
    _shared: Option<OwnedRwLockReadGuard<WindowSlot>>,
    _exclusive: Option<OwnedRwLockWriteGuard<WindowSlot>>,
}

impl Viewports {
    pub(crate) fn pointer_scale(&self, page: &PageId) -> f64 {
        self.tab(page).map_or(1.0, |tab| tab.fixed.scale())
    }

    pub(crate) async fn set(
        &self,
        view: &BrowserView,
        page: PageId,
        request: ViewportRequest,
    ) -> Result<ViewportReport, CoreError> {
        let dialog_open = || {
            CoreError::Message(format!(
                "a JavaScript dialog is open on page {page}; accept or dismiss it first"
            ))
        };
        if view.signals().pending_dialog(&page).is_some() {
            return Err(dialog_open());
        }
        self.forget_closed(view).await;
        let info = view.page(page.clone()).await?;
        let mut slot = self.lock_window(view, info.window_id).await;
        if view.signals().pending_dialog(&page).is_some() {
            return Err(dialog_open());
        }
        let session = view.page_session(page.clone()).await?.session_id;
        let Some(fixed) = request.fixed() else {
            // Forgotten only once cleared: a failed clear leaves the tab in its mode.
            if self.tab(&page).and_then(|tab| tab.applied) == Some(session.clone()) {
                for (method, params) in device::clear_commands() {
                    view.send_at_capture(method, params, Some(&session)).await?;
                }
            }
            self.forget(&page);
            let metrics = device::measure(view, &session).await?;
            view.page(page.clone()).await?;
            return Ok(report(&page, request, metrics, None));
        };
        // Recorded before applying: a failure leaves the tab pending, so the next operation
        // retries instead of acting on a half-applied device.
        self.record(&page, fixed, None);
        let chrome_version = self.chrome_version(view).await?;
        for (method, params) in fixed.commands(&chrome_version) {
            view.send_at_capture(method, params, Some(&session)).await?;
        }
        let metrics = device::settle(view, &session, &page, fixed).await?;
        self.record(&page, fixed, Some(session.clone()));
        let placement = window::locate(view, &page, &info.target_id, info.window_id).await?;
        let fit = window::fit(view, &session, placement, fixed.preview(), true).await?;
        if let Some(slot) = slot.as_mut() {
            slot.fitted = Some((fit.contents, fit.bounds.clone()));
        }
        view.page(page.clone()).await?;
        Ok(report(&page, request, metrics, Some(fit)))
    }

    pub(crate) async fn prepare(
        &self,
        view: &BrowserView,
        page: PageId,
    ) -> Result<ViewportLease, CoreError> {
        // An open dialog blocks the renderer; only the dialog actions can run, and they need
        // no geometry. They must not wait for the window either: work stalled by the same
        // dialog may hold it. The next operation after the dialog restores the tab.
        if view.signals().pending_dialog(&page).is_some() {
            return Ok(ViewportLease::default());
        }
        if self.tab(&page).is_none() {
            // A default tab changes nothing, but must not act while a fixed tab's operation
            // owns the window it shares.
            return Ok(self.hold(view, page).await);
        }
        let info = view.page(page.clone()).await?;
        let mut slot = self.lock_window(view, info.window_id).await;
        // Re-read under the lock: a setViewport may have changed the tab while we waited.
        let Some(tab) = self.tab(&page) else {
            return Ok(ViewportLease {
                _shared: None,
                _exclusive: slot,
            });
        };
        // The dialog may have opened while this operation waited for the window.
        if view.signals().pending_dialog(&page).is_some() {
            return Ok(ViewportLease {
                _shared: None,
                _exclusive: slot,
            });
        }
        let session = view.page_session(page.clone()).await?.session_id;
        if tab.applied.as_ref() != Some(&session) {
            let chrome_version = self.chrome_version(view).await?;
            for (method, params) in tab.fixed.commands(&chrome_version) {
                view.send_at_capture(method, params, Some(&session)).await?;
            }
            device::settle(view, &session, &page, tab.fixed).await?;
            self.record(&page, tab.fixed, Some(session.clone()));
        }
        let placement = window::locate(view, &page, &info.target_id, info.window_id).await?;
        let contents = tab.fixed.preview();
        let last = slot
            .as_ref()
            .and_then(|slot| slot.fitted.as_ref())
            .filter(|(fitted, _)| *fitted == contents)
            .map(|(_, bounds)| bounds);
        if !window::unchanged(&placement, last) {
            let fit = window::fit(view, &session, placement, contents, false).await?;
            if let Some(slot) = slot.as_mut() {
                slot.fitted = Some((contents, fit.bounds));
            }
        }
        Ok(ViewportLease {
            _shared: None,
            _exclusive: slot,
        })
    }

    /// Shares the page's window: fits and fixed tabs' operations wait for every holder, other
    /// holders do not. Taken whatever the tab's mode, because a mode change can start while
    /// the holder still works (a capture restores the emulation it saw when it began).
    pub(crate) async fn hold(&self, view: &BrowserView, page: PageId) -> ViewportLease {
        let window = view
            .cached_page(&page)
            .await
            .and_then(|info| info.window_id);
        ViewportLease {
            _shared: self.share_window(view, window).await,
            _exclusive: None,
        }
    }

    /// Tab records and window slots held, for tests of bounded state.
    #[cfg(test)]
    pub(crate) fn tracked(&self) -> (usize, usize) {
        let tabs = self
            .tabs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .len();
        let windows = self
            .windows
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .len();
        (tabs, windows)
    }

    fn tab(&self, page: &PageId) -> Option<Tab> {
        self.tabs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(page)
            .cloned()
    }

    fn record(&self, page: &PageId, fixed: Fixed, applied: Option<SessionId>) {
        self.tabs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(page.clone(), Tab { fixed, applied });
    }

    fn forget(&self, page: &PageId) {
        self.tabs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(page);
    }

    /// Drops tab records and idle window slots the registry no longer knows, so a long-lived
    /// server keeps state only for open tabs and windows. Runs only where state grows.
    async fn forget_closed(&self, view: &BrowserView) {
        let open = view.known_page_ids().await;
        let mut windows = std::collections::BTreeSet::new();
        for page in &open {
            if let Some(window) = view.cached_page(page).await.and_then(|info| info.window_id) {
                windows.insert(window);
            }
        }
        self.tabs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .retain(|page, _| open.contains(page));
        self.windows
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .retain(|window, slot| windows.contains(window) || Arc::strong_count(slot) > 1);
    }

    async fn slot(&self, view: &BrowserView, window: WindowId) -> Arc<RwLock<WindowSlot>> {
        let known = self
            .windows
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .contains_key(&window);
        if !known {
            self.forget_closed(view).await;
        }
        self.windows
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .entry(window)
            .or_default()
            .clone()
    }

    async fn lock_window(
        &self,
        view: &BrowserView,
        window: Option<WindowId>,
    ) -> Option<OwnedRwLockWriteGuard<WindowSlot>> {
        Some(self.slot(view, window?).await.write_owned().await)
    }

    async fn share_window(
        &self,
        view: &BrowserView,
        window: Option<WindowId>,
    ) -> Option<OwnedRwLockReadGuard<WindowSlot>> {
        Some(self.slot(view, window?).await.read_owned().await)
    }

    async fn chrome_version(&self, view: &BrowserView) -> Result<String, CoreError> {
        self.chrome_version
            .get_or_try_init(|| async {
                let reply = view
                    .send_at_capture("Browser.getVersion", json!({}), None)
                    .await?;
                device::chrome_version(&reply).ok_or_else(|| {
                    CoreError::Message("the browser did not report its Chrome version".into())
                })
            })
            .await
            .cloned()
    }
}

fn report(
    page: &PageId,
    request: ViewportRequest,
    viewport: PageMetrics,
    window: Option<WindowFit>,
) -> ViewportReport {
    ViewportReport {
        page: page.0,
        mode: request.mode(),
        zoom: request.zoom(),
        size: request.fixed().map(Fixed::size),
        viewport,
        window,
    }
}
