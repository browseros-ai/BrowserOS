//! The takeover parts' one door into Chromium, keyed by the browser's tab id.
//!
//! Every part and every host request speaks `TabId`, the key the contract and the trace records
//! use (decision 13); core speaks `PageId`, which only this process knows. This module is the
//! only place in the crate that maps one to the other, and the place later parts add their
//! browser calls: the window state, Ctrl+0, the viewport and the frames. Everything goes over
//! the agent connection's own session, the one the app's browser link owns, never a connection
//! of its own.

use crate::PartError;
use crate::limits::{WINDOW_POLL, WINDOW_SETTLE};
use browseros_cdp::CdpEvent;
use browseros_core::navigation::Navigation;
use browseros_core::pages::PageInfo;
use browseros_core::viewport::{ViewportMode, ViewportRequest, Viewports};
use browseros_core::{
    BrowserSession, BrowserView, CoreError, FrameId, PageId, ProtocolSession, SessionId, TabId,
    TargetId, WindowId,
};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::sync::broadcast;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

/// Chromium as the takeover parts reach it. Cheap to clone. The service builds one from the
/// session and viewports the app hands it and passes it to the parts.
#[derive(Clone)]
pub struct BrowserTabs {
    session: Arc<BrowserSession>,
    viewports: Arc<Viewports>,
}

impl BrowserTabs {
    /// The agent's session and the process's viewport state, both from the app's browser link.
    #[must_use]
    pub fn new(session: Arc<BrowserSession>, viewports: Arc<Viewports>) -> Self {
        Self { session, viewports }
    }

    /// The open tab's page. One `Browser.getTabs` round trip, so it also answers whether the
    /// tab is open.
    ///
    /// # Errors
    /// `TabNotFound` when no open tab has this id; `Browser` when the browser does not answer.
    pub async fn page(&self, tab: TabId) -> Result<PageInfo, PartError> {
        self.pages()
            .await?
            .into_iter()
            .find(|page| page.tab_id == tab)
            .ok_or(PartError::TabNotFound(tab))
    }

    /// Every open tab, in the browser's page order.
    ///
    /// # Errors
    /// `Browser` when the browser does not answer.
    pub async fn open_tabs(&self) -> Result<Vec<TabId>, PartError> {
        Ok(self
            .pages()
            .await?
            .into_iter()
            .map(|page| page.tab_id)
            .collect())
    }

    pub(crate) async fn pages(&self) -> Result<Vec<PageInfo>, PartError> {
        self.session.pages.list().await.map_err(PartError::Browser)
    }

    /// The open tab's page, and the page target's session: the one core attached with the
    /// Page, DOM and Runtime domains on. It is not admitted, so a send costs one round trip; a
    /// bound view's session re-reads the tab around every send.
    ///
    /// # Errors
    /// `TabNotFound` when no open tab has this id; `Browser` when the browser does not answer.
    pub(crate) async fn page_session(
        &self,
        tab: TabId,
    ) -> Result<(PageId, ProtocolSession), PartError> {
        let page = self.page(tab).await?;
        let session = self.session_for(&page).await?;
        Ok((page.page_id, session))
    }

    /// The session of the target that hosts `frame`, an out-of-process frame of `page`. Core's
    /// frame walk finds it (attaching on demand) and checks that the frame still belongs to the
    /// page; the session it hands back is then used raw, because its per-send admission costs
    /// several round trips and refuses `Page.addScriptToEvaluateOnNewDocument`.
    pub(crate) async fn frame_session(
        &self,
        page: PageId,
        frame: FrameId,
    ) -> Result<ProtocolSession, CoreError> {
        let bound = BrowserView::unbound(Arc::clone(&self.session), None)
            .frame_session(page, frame)
            .await?;
        let id = bound
            .session_id()
            .cloned()
            .ok_or_else(|| CoreError::Message("the frame has no session of its own".into()))?;
        Ok(self.session(id))
    }

    /// A session an event named, such as a frame target the browser just attached.
    pub(crate) fn session(&self, id: SessionId) -> ProtocolSession {
        self.session.protocol_session(id)
    }

    /// Every CDP event on the agent connection, from now on.
    pub(crate) fn events(&self) -> broadcast::Receiver<CdpEvent> {
        self.session.cdp_events()
    }
}

/// A browser window's state, as Chromium reports it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WindowState {
    Normal,
    Minimized,
    Maximized,
    Fullscreen,
}

impl WindowState {
    fn from_wire(state: Option<&str>) -> Option<Self> {
        match state {
            // `Bounds.windowState` is optional, and normal when absent.
            None | Some("normal") => Some(Self::Normal),
            Some("minimized") => Some(Self::Minimized),
            Some("maximized") => Some(Self::Maximized),
            Some("fullscreen") => Some(Self::Fullscreen),
            Some(_) => None,
        }
    }

    fn wire(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Minimized => "minimized",
            Self::Maximized => "maximized",
            Self::Fullscreen => "fullscreen",
        }
    }

    /// Whether a window that reads `now` is in this state. Neo's window manager (openbox)
    /// maximizes every normal window, so a normal window may read as maximized.
    fn reached(self, now: Self) -> bool {
        match self {
            Self::Normal => matches!(now, Self::Normal | Self::Maximized),
            state => now == state,
        }
    }
}

/// The fit's calls. The window commands go as raw JSON on the session itself: a scoped
/// `BrowserView` refuses them (`target_scope_unsupported`), and they have no typed bindings.
/// Neo has one window, which every tab shares.
impl BrowserTabs {
    /// Brings the tab to the front of the one screen with BrowserOS's `Browser.activateTab`.
    /// `Page.bringToFront` and `Target.activateTarget` do nothing on a tab a DevTools client is
    /// attached to, and every agent tab has one.
    pub(crate) async fn activate(&self, tab: TabId) -> Result<(), PartError> {
        self.call("Browser.activateTab", json!({ "tabId": tab.0 }))
            .await
            .map(drop)
    }

    /// The window that shows `page`, and its state now.
    pub(crate) async fn window(
        &self,
        page: &PageInfo,
    ) -> Result<(WindowId, WindowState), PartError> {
        let reply: WindowReply = self
            .call_for(
                "Browser.getWindowForTarget",
                json!({ "targetId": page.target_id.as_str() }),
            )
            .await?;
        Ok((WindowId(reply.window_id), reply.bounds.state()?))
    }

    /// Every browser window and its state (BrowserOS's `Browser.getWindows`), for when no tab
    /// names the window, such as at start.
    pub(crate) async fn windows(&self) -> Result<Vec<(WindowId, WindowState)>, PartError> {
        let reply: WindowsReply = self.call_for("Browser.getWindows", json!({})).await?;
        reply
            .windows
            .into_iter()
            .map(|window| Ok((WindowId(window.window_id), window.bounds.state()?)))
            .collect()
    }

    /// Puts `window` in `state`, then reads it back until it shows it, for up to 3 s.
    /// Fullscreen settles in about 5 ms and hides the browser chrome as F11 does. A window
    /// already in `state` is left alone, so a maximized window is never sent `normal`.
    pub(crate) async fn set_window_state(
        &self,
        window: WindowId,
        state: WindowState,
    ) -> Result<(), PartError> {
        let now = self.window_state(window).await?;
        if state.reached(now) {
            return Ok(());
        }
        // Chromium refuses `maximized` on a fullscreen window, so a fullscreen window leaves it
        // as `normal`, which openbox maximizes. The state always goes alone: Chromium refuses
        // a state combined with a position or a size.
        let sent = match (state, now) {
            (WindowState::Maximized, WindowState::Fullscreen) => WindowState::Normal,
            _ => state,
        };
        self.call(
            "Browser.setWindowBounds",
            json!({ "windowId": window.0, "bounds": { "windowState": sent.wire() } }),
        )
        .await?;
        let deadline = Instant::now() + WINDOW_SETTLE;
        loop {
            let now = self.window_state(window).await?;
            if state.reached(now) {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(browser_error(format!(
                    "window {} still reads {now:?}, not {state:?}, {WINDOW_SETTLE:?} after it was set",
                    window.0
                )));
            }
            tokio::time::sleep(WINDOW_POLL).await;
        }
    }

    /// Resets the tab's zoom with Ctrl+0 as a raw key on its page session: CDP has no zoom
    /// command, and Chromium takes Ctrl+0 as a browser accelerator only from a raw key with
    /// its native code (decision 24).
    pub(crate) async fn reset_zoom(&self, tab: TabId) -> Result<(), PartError> {
        let (_, session) = self.page_session(tab).await?;
        for kind in ["rawKeyDown", "keyUp"] {
            session
                .send_value(
                    "Input.dispatchKeyEvent",
                    json!({
                        "type": kind,
                        "modifiers": 2,
                        "key": "0",
                        "code": "Digit0",
                        "windowsVirtualKeyCode": 48,
                        "nativeVirtualKeyCode": 48,
                    }),
                )
                .await
                .map_err(PartError::Browser)?;
        }
        Ok(())
    }

    /// The request the tab's fixed viewport mode was set with, as core keeps it (decision 25);
    /// `None` in the default mode.
    pub(crate) fn viewport(&self, page: &PageInfo) -> Option<ViewportRequest> {
        self.view().current_viewport(&page.page_id)
    }

    /// Sets the tab's viewport; `None` puts it in the default mode. Core waits for every
    /// holder of the tab's viewport lock, then for the page to report the mode, and a page
    /// that never reports it is an error: the plan that runs this bounds it.
    pub(crate) async fn set_viewport(
        &self,
        tab: TabId,
        request: Option<ViewportRequest>,
    ) -> Result<(), PartError> {
        let page = self.page(tab).await?;
        let request = match request {
            Some(request) => request,
            None => ViewportRequest::new(ViewportMode::Default, None).map_err(browser_error)?,
        };
        self.view()
            .set_viewport(page.page_id, request, &CancellationToken::new())
            .await
            .map(drop)
            .map_err(PartError::Browser)
    }

    /// The page's own protocol session: core's attachment, shared with the agent.
    pub(crate) async fn session_for(&self, page: &PageInfo) -> Result<ProtocolSession, PartError> {
        self.session
            .pages
            .get_session(page.page_id.clone())
            .await
            .map(|page| page.session)
            .map_err(PartError::Browser)
    }

    /// The pages `page` opened that are still open (`Target.getTargets`, by `openerId`).
    pub(crate) async fn opened_by(&self, page: &PageInfo) -> Result<Vec<Opened>, PartError> {
        let reply: TargetsReply = self.call_for("Target.getTargets", json!({})).await?;
        Ok(reply
            .target_infos
            .into_iter()
            .filter(|target| {
                target.kind == "page" && target.opener_id.as_ref() == Some(&page.target_id.0)
            })
            .map(|target| Opened {
                target: TargetId(target.target_id),
                url: target.url,
            })
            .collect())
    }

    /// Closes the page's tab (`Browser.closeTab`). A page that asks "Leave site?" stays open.
    pub(crate) async fn close(&self, page: &PageInfo) -> Result<(), PartError> {
        self.session
            .pages
            .close(page.page_id.clone())
            .await
            .map_err(PartError::Browser)
    }

    /// Core's navigation of the page: the same moves, and the same waits for the page to
    /// arrive, as the agent's own `page.goto`, `goBack` and `reload`.
    pub(crate) fn navigation(&self, page: &PageInfo) -> Navigation {
        self.session.nav(page.page_id.clone())
    }

    async fn window_state(&self, window: WindowId) -> Result<WindowState, PartError> {
        let reply: BoundsReply = self
            .call_for("Browser.getWindowBounds", json!({ "windowId": window.0 }))
            .await?;
        reply.bounds.state()
    }

    /// An unbound view with the process's viewport state: no admission, the agent's own tabs.
    fn view(&self) -> BrowserView {
        BrowserView::unbound(Arc::clone(&self.session), None)
            .with_viewports(Arc::clone(&self.viewports))
    }

    async fn call(&self, method: &str, params: Value) -> Result<Value, PartError> {
        self.session
            .cdp(method, params, None)
            .await
            .map_err(PartError::Browser)
    }

    async fn call_for<T: DeserializeOwned>(
        &self,
        method: &str,
        params: Value,
    ) -> Result<T, PartError> {
        serde_json::from_value(self.call(method, params).await?)
            .map_err(|error| browser_error(format!("{method} answered unexpectedly: {error}")))
    }
}

/// A page another page opened.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Opened {
    pub(crate) target: TargetId,
    /// Its URL now, as Chromium's target list reports it.
    pub(crate) url: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TargetsReply {
    target_infos: Vec<TargetReply>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TargetReply {
    target_id: String,
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    url: String,
    #[serde(default)]
    opener_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WindowReply {
    window_id: i64,
    bounds: Bounds,
}

#[derive(Deserialize)]
struct WindowsReply {
    windows: Vec<WindowReply>,
}

#[derive(Deserialize)]
struct BoundsReply {
    bounds: Bounds,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Bounds {
    window_state: Option<String>,
}

impl Bounds {
    fn state(&self) -> Result<WindowState, PartError> {
        WindowState::from_wire(self.window_state.as_deref()).ok_or_else(|| {
            browser_error(format!("an unknown window state {:?}", self.window_state))
        })
    }
}

/// The browser answered, but not as the protocol says it would.
fn browser_error(message: String) -> PartError {
    PartError::Browser(CoreError::Message(message))
}
