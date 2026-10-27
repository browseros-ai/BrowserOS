//! Hosted viewport modes for one tab. Mobile and desktop emulate a fixed device inside whatever
//! window the tab has; default leaves the tab natural. The browser window is never read or
//! changed: a caller that wants a window size or fullscreen sets it itself.
//!
//! Hosts enable the feature by attaching one [`Viewports`] to their views; every entry point is
//! on [`BrowserView`], so admission and scope stay where they are.

mod device;
#[cfg(test)]
mod tests;

use crate::{BrowserView, CoreError, PageId, SessionId};
use device::Fixed;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};
use tokio::sync::{OnceCell, OwnedRwLockReadGuard, OwnedRwLockWriteGuard, RwLock};
use tokio_util::sync::CancellationToken;

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

/// What the page actually reports after a change. Nothing here is copied from the request: a
/// value that did not take effect is reported as the page shows it.
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

/// Per-process viewport state. A host creates one and attaches it to every view whose
/// calls may use the feature. PageIds are never reused within a process, so a closed tab's
/// entry can only go stale, never apply to another tab.
#[derive(Default)]
pub struct Viewports {
    tabs: Mutex<HashMap<PageId, Tab>>,
    /// One lock per tab: its mode changes and its fixed operations hold it alone, captures and
    /// default operations share it.
    pages: Mutex<HashMap<PageId, Arc<RwLock<()>>>>,
    chrome_version: OnceCell<String>,
    generations: AtomicU64,
}

/// A tab in a fixed mode. Default tabs have no entry.
#[derive(Debug, Clone)]
struct Tab {
    fixed: Fixed,
    /// The page session on which the mode is wholly in place: emulation applied and the view
    /// pinned. It dies with that session. `None` while a change is pending: whatever failed or
    /// was cut short, the tab's next operation does it again.
    applied: Option<SessionId>,
    /// Moves whenever the tab's emulation may change under us. A change that began under an
    /// older generation never marks the tab applied.
    generation: u64,
}

/// Holds the page for one operation, so its mode cannot change while the operation reads,
/// captures or acts. Dropping it releases the page. A fixed tab's operation holds its page
/// alone; other holders share it.
#[derive(Default)]
pub struct ViewportLease {
    _shared: Option<OwnedRwLockReadGuard<()>>,
    _exclusive: Option<OwnedRwLockWriteGuard<()>>,
}

/// Held across a caller's raw call that replaces what a fixed mode set on the page session.
/// Once dropped, however the call ended, the tab's next operation applies its mode again.
pub(crate) struct Reapply {
    viewports: Arc<Viewports>,
    page: PageId,
}

impl Drop for Reapply {
    fn drop(&mut self) {
        self.viewports.invalidate(&self.page);
    }
}

/// Whether a caller's raw `method` replaces something a fixed mode set on the page session.
pub(crate) fn overrides(method: &str) -> bool {
    device::overrides(method)
}

impl Viewports {
    pub(crate) fn reapply(self: &Arc<Self>, page: PageId) -> Reapply {
        Reapply {
            viewports: Arc::clone(self),
            page,
        }
    }

    pub(crate) fn pointer_scale(&self, page: &PageId) -> f64 {
        self.tab(page).map_or(1.0, |tab| tab.fixed.scale())
    }

    pub(crate) async fn set(
        &self,
        view: &BrowserView,
        page: PageId,
        request: ViewportRequest,
        cancel: &CancellationToken,
    ) -> Result<ViewportReport, CoreError> {
        let dialog_open = || {
            CoreError::Message(format!(
                "a JavaScript dialog is open on page {page}; accept or dismiss it first"
            ))
        };
        let cancelled =
            || CoreError::Message("setViewport was cancelled before it changed anything".into());
        if view.signals().pending_dialog(&page).is_some() {
            return Err(dialog_open());
        }
        self.forget_closed(view).await;
        view.page(page.clone()).await?;
        // A change cancelled while it waits for the tab gives up without touching the page; one
        // that holds the tab no longer looks at the token.
        let _held = tokio::select! {
            biased;
            () = cancel.cancelled() => return Err(cancelled()),
            held = self.lock_page(view, &page) => held,
        };
        if cancel.is_cancelled() {
            return Err(cancelled());
        }
        if view.signals().pending_dialog(&page).is_some() {
            return Err(dialog_open());
        }
        let admitted = view.page_session(page.clone()).await?;
        let session = admitted.session_id.clone();
        let Some(fixed) = request.fixed() else {
            // Any record may have overrides on this session, including a change that failed or
            // was interrupted after sending them. Forgotten only once cleared: a failed clear
            // leaves the tab in its mode.
            if self.tab(&page).is_some() {
                // Pending until cleared: a clear that fails or is cut short midway leaves the
                // tab's next operation to put its whole mode back.
                self.invalidate(&page);
                for (method, params) in device::clear_commands() {
                    view.send_at_capture(method, params, Some(&session)).await?;
                }
            }
            self.forget(&page);
            let metrics = device::measure(&admitted.session).await?;
            view.page(page.clone()).await?;
            return Ok(report(&page, request, metrics));
        };
        // Recorded as pending before anything is sent, and applied only once the view is
        // pinned: a step that fails or is cut short leaves the next operation to redo it.
        let generation = self.begin(&page, fixed);
        let chrome_version = self.chrome_version(view).await?;
        for (method, params) in fixed.commands(&chrome_version) {
            view.send_at_capture(method, params, Some(&session)).await?;
        }
        let metrics = device::settle(&admitted.session, &page, fixed).await?;
        let (method, params) = fixed.pin();
        view.send_at_capture(method, params, Some(&session)).await?;
        self.applied(&page, &session, generation);
        view.page(page.clone()).await?;
        Ok(report(&page, request, metrics))
    }

    pub(crate) async fn prepare(
        &self,
        view: &BrowserView,
        page: PageId,
    ) -> Result<ViewportLease, CoreError> {
        // An open dialog blocks the renderer; only the dialog actions can run, and they need
        // no emulation. They must not wait for the tab either: work stalled by the same dialog
        // may hold it. The next operation after the dialog restores the tab.
        if view.signals().pending_dialog(&page).is_some() {
            return Ok(ViewportLease::default());
        }
        if self.tab(&page).is_none() {
            // A default tab changes nothing, but must not act while its mode changes.
            let lease = self.hold(view, page.clone()).await;
            // A setViewport queued ahead of this operation may have made the tab fixed; the
            // shared lease is released here and the fixed path below holds the tab alone.
            if self.tab(&page).is_none() {
                return Ok(lease);
            }
        }
        let lease = ViewportLease {
            _shared: None,
            _exclusive: Some(self.lock_page(view, &page).await),
        };
        // The dialog may have opened while this operation waited for the tab.
        if self.tab(&page).is_none() || view.signals().pending_dialog(&page).is_some() {
            return Ok(lease);
        }
        let admitted = view.page_session(page.clone()).await?;
        let session = admitted.session_id.clone();
        // Read after the session: a caller's raw override may have landed meanwhile.
        let Some(tab) = self.tab(&page) else {
            return Ok(lease);
        };
        if tab.applied.as_ref() == Some(&session) {
            return Ok(lease);
        }
        // Lost with its session or replaced by a caller's raw override. Pending until pinned
        // again: a step that fails or is cut short leaves the next operation to redo it.
        let generation = self.invalidate(&page);
        let chrome_version = self.chrome_version(view).await?;
        for (method, params) in tab.fixed.commands(&chrome_version) {
            view.send_at_capture(method, params, Some(&session)).await?;
        }
        device::settle(&admitted.session, &page, tab.fixed).await?;
        let (method, params) = tab.fixed.pin();
        view.send_at_capture(method, params, Some(&session)).await?;
        self.applied(&page, &session, generation);
        Ok(lease)
    }

    /// Shares the page: its mode changes and its fixed operations wait for every holder, other
    /// holders do not. Taken whatever the tab's mode, because a mode change can start while
    /// the holder still works (a capture restores the emulation it saw when it began).
    pub(crate) async fn hold(&self, view: &BrowserView, page: PageId) -> ViewportLease {
        ViewportLease {
            _shared: Some(self.share_page(view, &page).await),
            _exclusive: None,
        }
    }

    /// Tab records and page locks held, for tests of bounded state.
    #[cfg(test)]
    pub(crate) fn tracked(&self) -> (usize, usize) {
        let tabs = self
            .tabs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .len();
        let pages = self
            .pages
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .len();
        (tabs, pages)
    }

    fn tab(&self, page: &PageId) -> Option<Tab> {
        self.tabs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(page)
            .cloned()
    }

    /// Records `fixed` as the tab's mode, pending, and returns the generation its change runs
    /// under.
    fn begin(&self, page: &PageId, fixed: Fixed) -> u64 {
        let generation = self.next_generation();
        self.tabs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(
                page.clone(),
                Tab {
                    fixed,
                    applied: None,
                    generation,
                },
            );
        generation
    }

    /// The mode is wholly in place on `session`, unless something touched the tab's emulation
    /// after the change began.
    fn applied(&self, page: &PageId, session: &SessionId, generation: u64) {
        if let Some(tab) = self
            .tabs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get_mut(page)
            && tab.generation == generation
        {
            tab.applied = Some(session.clone());
        }
    }

    /// The tab's emulation may have changed under us: its next operation applies the mode again,
    /// and no change already under way marks it applied. Returns the new generation.
    fn invalidate(&self, page: &PageId) -> u64 {
        let generation = self.next_generation();
        if let Some(tab) = self
            .tabs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get_mut(page)
        {
            tab.applied = None;
            tab.generation = generation;
        }
        generation
    }

    fn next_generation(&self) -> u64 {
        self.generations.fetch_add(1, Ordering::Relaxed) + 1
    }

    fn forget(&self, page: &PageId) {
        self.tabs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(page);
    }

    /// Drops tab records and idle page locks the registry no longer knows, so a long-lived
    /// server keeps state only for open tabs. Runs only where state grows.
    async fn forget_closed(&self, view: &BrowserView) {
        let open = view.known_page_ids().await;
        self.tabs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .retain(|page, _| open.contains(page));
        self.pages
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .retain(|page, lock| open.contains(page) || Arc::strong_count(lock) > 1);
    }

    async fn page_lock(&self, view: &BrowserView, page: &PageId) -> Arc<RwLock<()>> {
        let known = self
            .pages
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .contains_key(page);
        if !known {
            self.forget_closed(view).await;
        }
        self.pages
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .entry(page.clone())
            .or_default()
            .clone()
    }

    async fn lock_page(&self, view: &BrowserView, page: &PageId) -> OwnedRwLockWriteGuard<()> {
        self.page_lock(view, page).await.write_owned().await
    }

    async fn share_page(&self, view: &BrowserView, page: &PageId) -> OwnedRwLockReadGuard<()> {
        self.page_lock(view, page).await.read_owned().await
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

fn report(page: &PageId, request: ViewportRequest, viewport: PageMetrics) -> ViewportReport {
    ViewportReport {
        page: page.0,
        mode: request.mode(),
        zoom: request.zoom(),
        size: request.fixed().map(Fixed::size),
        viewport,
    }
}
