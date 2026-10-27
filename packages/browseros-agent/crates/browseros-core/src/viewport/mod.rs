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
use tokio::sync::{Mutex as AsyncMutex, OnceCell, OwnedMutexGuard};

pub const MIN_ZOOM: f64 = 25.0;
pub const MAX_ZOOM: f64 = 400.0;
pub const DEFAULT_ZOOM: f64 = 100.0;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ViewportMode {
    #[default]
    Default,
    Mobile,
    Desktop,
}

/// A validated request. Zoom is a percentage of the mobile preview and exists only for mobile;
/// leaving it out means 100, never the tab's previous zoom.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ViewportRequest {
    mode: ViewportMode,
    zoom: Option<f64>,
}

impl ViewportRequest {
    pub fn new(mode: ViewportMode, zoom: Option<f64>) -> Result<Self, String> {
        match (mode, zoom) {
            (ViewportMode::Mobile, None) => Ok(Self {
                mode,
                zoom: Some(DEFAULT_ZOOM),
            }),
            (ViewportMode::Mobile, Some(zoom))
                if zoom.is_finite() && (MIN_ZOOM..=MAX_ZOOM).contains(&zoom) =>
            {
                Ok(Self {
                    mode,
                    zoom: Some(zoom),
                })
            }
            (ViewportMode::Mobile, Some(_)) => Err(format!(
                "zoom must be a percentage from {MIN_ZOOM} to {MAX_ZOOM}"
            )),
            (_, Some(_)) => Err("zoom applies only to mobile".to_string()),
            (_, None) => Ok(Self { mode, zoom: None }),
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

    fn fixed(self) -> Option<Fixed> {
        match (self.mode, self.zoom) {
            (ViewportMode::Mobile, zoom) => Some(Fixed::Mobile {
                zoom: zoom.unwrap_or(DEFAULT_ZOOM),
            }),
            (ViewportMode::Desktop, _) => Some(Fixed::Desktop),
            (ViewportMode::Default, _) => None,
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
    windows: Mutex<HashMap<WindowId, Arc<AsyncMutex<WindowSlot>>>>,
    chrome_version: OnceCell<String>,
}

/// A tab in a fixed mode. Default tabs have no entry.
#[derive(Debug, Clone)]
struct Tab {
    fixed: Fixed,
    /// The page session the emulation lives on; it dies with that session.
    applied: Option<SessionId>,
    window: Option<WindowId>,
}

#[derive(Default)]
struct WindowSlot {
    /// Bounds this window had after its last fit, and the contents it was fitted for.
    fitted: Option<(Size, WindowBounds)>,
}

/// Holds the page's window for one operation, so no other tab's fit can change the window
/// while the operation reads, captures or acts. Dropping it releases the window.
#[derive(Default)]
pub struct ViewportLease {
    _window: Option<OwnedMutexGuard<WindowSlot>>,
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
        let info = view.page(page.clone()).await?;
        let mut slot = self.lock_window(info.window_id).await;
        if view.signals().pending_dialog(&page).is_some() {
            return Err(CoreError::Message(format!(
                "a JavaScript dialog is open on page {page}; accept or dismiss it first"
            )));
        }
        let session = view.page_session(page.clone()).await?.session_id;
        let Some(fixed) = request.fixed() else {
            let owned = self.forget(&page);
            if owned.and_then(|tab| tab.applied) == Some(session.clone()) {
                for (method, params) in device::clear_commands() {
                    view.send_at_capture(method, params, Some(&session)).await?;
                }
            }
            let metrics = device::measure(view, &session).await?;
            view.page(page.clone()).await?;
            return Ok(report(&page, request, metrics, None));
        };
        // Recorded before applying: a failure leaves the tab pending, so the next operation
        // retries instead of acting on a half-applied device.
        self.record(&page, fixed, None, info.window_id);
        let chrome_version = self.chrome_version(view).await?;
        for (method, params) in fixed.commands(&chrome_version) {
            view.send_at_capture(method, params, Some(&session)).await?;
        }
        let metrics = device::settle(view, &session, &page, fixed).await?;
        self.record(&page, fixed, Some(session.clone()), info.window_id);
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
        if self.tab(&page).is_none() {
            // A default tab changes nothing, but must not act while a fixed tab's operation
            // owns the window it shares.
            let window = view
                .cached_page(&page)
                .await
                .and_then(|info| info.window_id);
            let shared = window.filter(|window| self.holds_fixed_tab(*window));
            return Ok(ViewportLease {
                _window: self.lock_window(shared).await,
            });
        }
        let info = view.page(page.clone()).await?;
        let mut slot = self.lock_window(info.window_id).await;
        // Re-read under the lock: a setViewport may have changed the tab while we waited.
        let Some(tab) = self.tab(&page) else {
            return Ok(ViewportLease { _window: slot });
        };
        // An open dialog blocks the renderer; only the dialog actions can run, and they need
        // no geometry. The next operation after the dialog restores it.
        if view.signals().pending_dialog(&page).is_some() {
            return Ok(ViewportLease { _window: slot });
        }
        let session = view.page_session(page.clone()).await?.session_id;
        if tab.applied.as_ref() != Some(&session) {
            let chrome_version = self.chrome_version(view).await?;
            for (method, params) in tab.fixed.commands(&chrome_version) {
                view.send_at_capture(method, params, Some(&session)).await?;
            }
            device::settle(view, &session, &page, tab.fixed).await?;
            self.record(&page, tab.fixed, Some(session.clone()), info.window_id);
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
        Ok(ViewportLease { _window: slot })
    }

    fn holds_fixed_tab(&self, window: WindowId) -> bool {
        self.tabs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .values()
            .any(|tab| tab.window == Some(window))
    }

    fn tab(&self, page: &PageId) -> Option<Tab> {
        self.tabs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(page)
            .cloned()
    }

    fn record(
        &self,
        page: &PageId,
        fixed: Fixed,
        applied: Option<SessionId>,
        window: Option<WindowId>,
    ) {
        self.tabs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(
                page.clone(),
                Tab {
                    fixed,
                    applied,
                    window,
                },
            );
    }

    fn forget(&self, page: &PageId) -> Option<Tab> {
        self.tabs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(page)
    }

    async fn lock_window(&self, window: Option<WindowId>) -> Option<OwnedMutexGuard<WindowSlot>> {
        let window = window?;
        let slot = self
            .windows
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .entry(window)
            .or_default()
            .clone();
        Some(slot.lock_owned().await)
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
        viewport,
        window,
    }
}
