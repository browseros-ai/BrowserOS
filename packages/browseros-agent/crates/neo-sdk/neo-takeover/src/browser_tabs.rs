//! The takeover parts' one door into Chromium, keyed by the browser's tab id.
//!
//! Every part and every host request speaks `TabId`, the key the contract and the trace records
//! use (decision 13); core speaks `PageId`, which only this process knows. This module is the
//! only place in the crate that maps one to the other, and the place later parts add their
//! browser calls: the window state, Ctrl+0, the viewport and the frames. Everything goes over
//! the agent connection's own session, the one the app's browser link owns, never a connection
//! of its own.
//!
//! It is also the one funnel for the front of Neo's one screen (decision 45):
//!
//! ```text
//!  follow, decision 41's admission ───┐
//!  a fit claim's Step::Activate ──────┼─▶ front_turn() (FIFO) ─ decide again
//!  the agent's Front::bring_to_front ─┘    ─▶ FrontTurn::activate ─▶ Browser.activateTab
//!                                             └▶ the last FrontMove ─▶ every follower
//!  a control display's admission ─────▶ front_turn(), dropped: waits out a move in flight
//!  a takeover, a control display, a fit ─▶ hold_screen(by) ─▶ a ScreenHoldGuard, counted
//!  the door, the agent's move, follow ───▶ screen_facts() ─▶ the screen rule
//! ```
//!
//! The raw `Browser.activateTab` is private to this module, so a part that activated without
//! the turn would not compile.

use crate::PartError;
use crate::limits::{ACTIVATE_TIMEOUT, FRONT_SHOWN_POLL, WINDOW_POLL, WINDOW_SETTLE};
use crate::screen_rule::{ScreenFacts, ScreenHolder};
use browseros_core::navigation::Navigation;
use browseros_core::pages::PageInfo;
use browseros_core::viewport::{ViewportMode, ViewportRequest, Viewports};
use browseros_core::{
    BrowserSession, BrowserView, CoreError, ProtocolSession, TabId, TargetId, WindowId,
};
use neo_trace::{EpochMillis, TaskId};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;
use tokio::sync::watch;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

/// Chromium as the takeover parts reach it. Cheap to clone, and every clone shares one front:
/// the turn Neo's activations take one at a time, the record of the last one, and the owner's
/// holds on the screen. The service builds one from the session and viewports the app hands it
/// and clones it into every part. A second `new` over the same session is a second front: its
/// activations would not wait for this one's, and its holds would not refuse this one's movers.
#[derive(Clone)]
pub struct BrowserTabs {
    session: Arc<BrowserSession>,
    viewports: Arc<Viewports>,
    front: Arc<FrontState>,
}

/// Neo's one screen as its movers and its owner's holds share it.
struct FrontState {
    /// Held from a mover's last check to its activation's reply, so no other Neo activation
    /// lands between a mover's decision and its send. A tokio mutex: fair, so movers
    /// activate in the order they asked, and held across the CDP round trip and a private
    /// switch's confirm read. A fit takes it inside its claim's `Step::Activate`, under the
    /// surface lock; nothing takes the turn and then waits for the surface, so the two cannot
    /// deadlock.
    turn: tokio::sync::Mutex<()>,
    /// The owner's live holds, counted by kind. A std mutex, never held across an await: the
    /// app's door reads it synchronously. One registry for every part, so no part reads
    /// another's table (D13).
    holds: Mutex<ScreenFacts>,
    /// Neo's last activation, which followers read. `send_replace`, so every move wakes them,
    /// even one to the tab already in front. It never holds the owner's or Chromium's own
    /// moves, which Neo does not see.
    last: watch::Sender<Option<FrontMove>>,
}

/// One activation Neo made, as followers read it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FrontMove {
    /// The tab now in front.
    pub(crate) tab: TabId,
    /// Who moved it there.
    pub(crate) by: Mover,
    /// When the mover sent it, on the clock operation records carry. Stamped just before the
    /// send, so an operation that starts during the round trip is newer than the move. A task
    /// follower ignores an operation that began at or before it (follow.rs): the operation's
    /// start reaches the follower through the trace writer, after the writer's commit, so it
    /// can arrive after a move it predates.
    pub(crate) at: EpochMillis,
}

/// Who moved the front.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Mover {
    /// A view's follower, or a follow view's admission (decision 41).
    Follow,
    /// A fit's claim (`Step::Activate`).
    Fit,
    /// The agent's `page.bringToFront()`, for the run's task if the host gave one. A
    /// `followTask` follower adopts its own task's move.
    Agent { task: Option<TaskId> },
}

/// One live hold of the screen by its owner: a takeover, a control display or a fit. Dropping
/// it releases that hold, so a hold cannot outlive its holder or be released twice.
#[must_use = "the owner's hold on the screen ends when this is dropped"]
pub(crate) struct ScreenHoldGuard {
    front: Arc<FrontState>,
    by: ScreenHolder,
}

impl Drop for ScreenHoldGuard {
    fn drop(&mut self) {
        let mut holds = self
            .front
            .holds
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let count = holds.count(self.by);
        *count = count.saturating_sub(1);
    }
}

/// The front turn: while it is held, no other Neo activation runs. Every mover takes it, then
/// decides again, then activates through it.
#[must_use = "the turn orders Neo's activations; dropping it lets the next one run"]
pub(crate) struct FrontTurn<'a> {
    browser: &'a BrowserTabs,
    _turn: tokio::sync::MutexGuard<'a, ()>,
}

impl FrontTurn<'_> {
    /// Brings `tab` to the front, bounded by `ACTIVATE_TIMEOUT`, and on success records the move
    /// for followers. The caller decided to activate after it took this turn, so nothing
    /// Neo does lands between that decision and this send.
    ///
    /// # Errors
    /// `Browser` when the browser refused or the link failed; `TimedOut` with no answer in 5 s.
    pub(crate) async fn activate(&self, tab: TabId, by: Mover) -> Result<(), PartError> {
        let at = EpochMillis::now();
        tokio::time::timeout(ACTIVATE_TIMEOUT, self.browser.send_activate(tab))
            .await
            .unwrap_or(Err(PartError::TimedOut("bringing a tab to the front")))?;
        self.browser
            .front
            .last
            .send_replace(Some(FrontMove { tab, by, at }));
        Ok(())
    }

    /// Whether `tab` reads as in front in `Browser.getTabs` within `within`, read every
    /// `FRONT_SHOWN_POLL`; a failed read counts as not yet. The agent's private switch keeps
    /// views out until this holds, so its guarantee does not rest on whether
    /// `Browser.activateTab` answers before or after the switch.
    pub(crate) async fn shows(&self, tab: TabId, within: Duration) -> bool {
        let shown = async {
            while !self
                .browser
                .front_tabs()
                .await
                .is_ok_and(|front| front.contains(&tab))
            {
                tokio::time::sleep(FRONT_SHOWN_POLL).await;
            }
        };
        tokio::time::timeout(within, shown).await.is_ok()
    }
}

impl BrowserTabs {
    /// The agent's session and the process's viewport state, both from the app's browser link,
    /// with a fresh front: no move yet, and no hold.
    #[must_use]
    pub fn new(session: Arc<BrowserSession>, viewports: Arc<Viewports>) -> Self {
        Self {
            session,
            viewports,
            front: Arc::new(FrontState {
                turn: tokio::sync::Mutex::default(),
                holds: Mutex::default(),
                last: watch::channel(None).0,
            }),
        }
    }

    /// Waits for the front turn, behind the movers that asked first.
    pub(crate) async fn front_turn(&self) -> FrontTurn<'_> {
        FrontTurn {
            browser: self,
            _turn: self.front.turn.lock().await,
        }
    }

    /// Takes a hold of the screen for its owner. Sync: the count rises before this returns, so
    /// the door's next check sees it.
    pub(crate) fn hold_screen(&self, by: ScreenHolder) -> ScreenHoldGuard {
        *self.holds().count(by) += 1;
        ScreenHoldGuard {
            front: Arc::clone(&self.front),
            by,
        }
    }

    /// The owner's holds now, for the screen rule. Sync, one short lock, no I/O.
    pub(crate) fn screen_facts(&self) -> ScreenFacts {
        *self.holds()
    }

    /// Neo's front moves from now on, for a follower. Subscribe before reading what the
    /// follower starts from, so a move between the two is not missed.
    pub(crate) fn front_moves(&self) -> watch::Receiver<Option<FrontMove>> {
        self.front.last.subscribe()
    }

    fn holds(&self) -> std::sync::MutexGuard<'_, ScreenFacts> {
        self.front
            .holds
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
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

    /// The tabs in front: each window's active tab, from a fresh `Browser.getTabs`. Neo has one
    /// window, so this is the tab its one screen shows.
    pub(crate) async fn front_tabs(&self) -> Result<Vec<TabId>, PartError> {
        Ok(self
            .pages()
            .await?
            .into_iter()
            .filter(|page| page.is_active)
            .map(|page| page.tab_id)
            .collect())
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
    /// attached to, and every agent tab has one. Private: only [`FrontTurn::activate`] reaches
    /// it.
    async fn send_activate(&self, tab: TabId) -> Result<(), PartError> {
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
        let page = self.page(tab).await?;
        let session = self.session_for(&page).await?;
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

#[cfg(test)]
mod tests {
    use super::{BrowserTabs, FrontMove, Mover};
    use crate::screen_rule::{ScreenFacts, ScreenHolder};
    use browseros_core::testing::TestConnection;
    use browseros_core::viewport::Viewports;
    use browseros_core::{BrowserSession, BrowserSessionHooks, TabId};
    use neo_trace::EpochMillis;
    use serde_json::json;
    use std::sync::Arc;
    use std::time::Duration;

    type BoxError = Box<dyn std::error::Error + Send + Sync>;
    type TestResult = Result<(), BoxError>;

    fn browser(connection: &Arc<TestConnection>) -> BrowserTabs {
        BrowserTabs::new(
            BrowserSession::new(connection.clone(), BrowserSessionHooks::default()),
            Arc::new(Viewports::default()),
        )
    }

    /// Each tab the browser was asked to bring to the front, in order.
    fn activated(connection: &TestConnection) -> Result<Vec<i64>, BoxError> {
        Ok(connection
            .calls()?
            .iter()
            .filter(|call| call.method == "Browser.activateTab")
            .filter_map(|call| call.params["tabId"].as_i64())
            .collect())
    }

    #[tokio::test(start_paused = true)]
    async fn an_activation_waits_for_the_front_turn() -> TestResult {
        let connection = TestConnection::new([]);
        connection.always("Browser.activateTab", json!({}))?;
        let browser = browser(&connection);
        let mut moves = browser.front_moves();
        let held = browser.front_turn().await;
        let waiting = tokio::spawn({
            let browser = browser.clone();
            async move {
                browser
                    .front_turn()
                    .await
                    .activate(TabId(4), Mover::Fit)
                    .await
            }
        });
        tokio::time::sleep(Duration::from_secs(1)).await;
        assert_eq!(
            activated(&connection)?,
            [0; 0],
            "another mover holds the turn, so nothing is sent"
        );
        assert!(!moves.has_changed()?, "and no move is recorded");

        let before = EpochMillis::now();
        drop(held);
        waiting.await??;
        assert_eq!(activated(&connection)?, [4], "sent once the turn is free");
        let FrontMove { tab, by, at } = moves
            .borrow_and_update()
            .clone()
            .ok_or("no move was recorded")?;
        assert_eq!(
            (tab, by),
            (TabId(4), Mover::Fit),
            "followers read who moved the front, and to which tab"
        );
        assert!(
            (before..=EpochMillis::now()).contains(&at),
            "and when it was sent"
        );
        Ok(())
    }

    #[tokio::test]
    async fn every_clone_reads_the_holds_until_their_guards_drop() -> TestResult {
        let connection = TestConnection::new([]);
        let browser = browser(&connection);
        let fit = browser.hold_screen(ScreenHolder::Fit);
        let control = browser.hold_screen(ScreenHolder::ControlDisplay);
        let part = browser.clone();
        assert_eq!(
            part.screen_facts(),
            ScreenFacts {
                takeover: 0,
                control: 1,
                fit: 1
            },
            "a part's clone reads the holds another part took"
        );
        drop(fit);
        assert_eq!(
            part.screen_facts(),
            ScreenFacts {
                control: 1,
                ..ScreenFacts::default()
            }
        );
        drop(control);
        assert_eq!(part.screen_facts(), ScreenFacts::default());
        Ok(())
    }
}
