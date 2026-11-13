//! The host API's composition root. The app hands [`Service::start`] values it already owns
//! and serves [`Service::router`] through [`host_service`](crate::host_service).
//!
//! The service also builds the takeover parts once, and the v1 takeover flow over them, so the
//! app reaches them through it rather than wiring each one: the lock policy and tab cleanup read
//! [`Service::locks`], the app's door and its `page.bringToFront()` read [`Service::front`],
//! and the browser link's tab events go to [`Service::tab_closed`] and [`Service::retain_tabs`], which hand each event to every part
//! that keeps state about tabs: the takeovers, the fits and the locks. A reconcile
//! takes a [`Service::tab_fence`] before its listing, so `retain_tabs` keeps what a host changed on a
//! tab that opened after it, through a host route (`tab_stamps`).

use crate::event_log::{EventKind, EventLog, LogEvent};
use crate::live_tabs::LiveTabs;
use crate::screenshot_taker::Camera;
use crate::settings::Settings;
use crate::tab_stamps::{TabFence, TabStamps};
use crate::{router, wire};
use axum::Router;
use browseros_core::viewport::Viewports;
use browseros_core::{BrowserSession, ProtocolSession, TabId};
use neo_screencast::{CaptureHub, CaptureTransport, CdpPortTransport};
use neo_takeover::{BrowserTabs, Displays, Fits, Front, Locks, TabHold, Takeovers, Xvnc};
use neo_trace::{EpochMillis, ShotJobs, TraceSink, TraceStore};
use std::sync::Arc;
use tokio_util::sync::{CancellationToken, DropGuard};

/// What the app hands the service: values it already owns, no trait to implement.
pub struct ServiceInputs {
    /// Reads for the record routes and the event log.
    pub store: TraceStore,
    /// The trace writer. Its committed events feed `/v1/events` and `wait=`.
    pub sink: TraceSink,
    /// The agent's browser connection (the app's browser link). Screenshots are taken on it,
    /// never on a connection of their own (decision 21), and the takeover parts reach the
    /// browser through it.
    pub browser: Arc<BrowserSession>,
    /// The process's viewport state, so a capture never overlaps a viewport change.
    pub viewports: Arc<Viewports>,
    /// The camera's end of the writer's screenshot queue (`neo_trace::screenshot_queue`;
    /// `spawn_writer` got the other end).
    pub screenshots: ShotJobs,
    /// Neo's screen: Xvnc's address and the size it rests at (the image's `NEO_SCREEN`). The
    /// fits resize it to the owner's viewer and put it back, and `GET /v1/display` shows it.
    pub xvnc: Xvnc,
    /// Chromium's CDP port. Live views capture over a WebSocket of their own to it, never over
    /// the agent's connection: frames are large and frequent, and there they would queue the
    /// agent's commands and its keepalive behind them.
    pub cdp_port: u16,
    /// The host's settings, which `/v1/settings` reads and changes. The app made them before
    /// anything that reads them started, and hands clones to those readers itself.
    pub settings: Settings,
    /// Which of the browser's tabs Neo keeps for itself, for `GET /v1/tabs`'s `owner`: `neo` for
    /// the app's blank tab, `None` for every other tab.
    pub tab_owner: TabOwner,
}

/// Names the tab Neo keeps for itself (`Some("neo")` for its blank tab), or `None`. Asked once per
/// tab of each `GET /v1/tabs`; it must not wait.
pub type TabOwner = Arc<dyn Fn(TabId) -> Option<&'static str> + Send + Sync>;

/// The host API, started. Cheap to clone.
#[derive(Clone)]
pub struct Service {
    state: Arc<ServiceState>,
}

/// What every route reads.
pub(crate) struct ServiceState {
    pub(crate) store: TraceStore,
    pub(crate) sink: TraceSink,
    pub(crate) events: Arc<EventLog>,
    pub(crate) browser: BrowserTabs,
    /// Read-only native inventory over the app's existing connection. Unlike the page cache,
    /// this includes internal/extension tabs too, so the host sees every open native tab.
    pub(crate) tab_inventory: ProtocolSession,
    pub(crate) locks: Arc<Locks>,
    pub(crate) fits: Arc<Fits>,
    /// Neo's screen over RFB for `GET /v1/display`, in view or control mode.
    pub(crate) displays: Arc<Displays>,
    /// The v1 takeover flow over the parts above (`/v1/takeovers`, and `release=` on a display).
    pub(crate) takeovers: Arc<Takeovers>,
    /// The agent's side of Neo's one screen, over `displays`. The app's door and its
    /// `page.bringToFront()` env read it; no route does.
    pub(crate) front: Front,
    /// The capture hub behind the live views (`live_tabs`).
    pub(crate) live: Arc<CaptureHub>,
    /// The tabs each route changed, stamped before the change, against a reconcile's fence.
    pub(crate) stamps: Arc<TabStamps>,
    /// What the host set (`/v1/settings`).
    pub(crate) settings: Settings,
    /// Which tab Neo keeps for itself (`ServiceInputs::tab_owner`).
    pub(crate) tab_owner: TabOwner,
    /// Cancels the hub's shutdown token when the service is dropped, which ends every live
    /// stream and capture.
    _live_shutdown: DropGuard,
}

impl Service {
    /// Starts the workers: the event log's numbering tasks, which subscribe to the writer, the
    /// locks and the takeovers before this returns, so every event from then on gets a number,
    /// and the camera that answers the writer's screenshot jobs. It also puts Neo's screen and
    /// window home, in the background (decision 30). Live views connect to the browser when the
    /// first one opens.
    ///
    /// # Panics
    /// Outside a Tokio runtime, as `tokio::spawn` does.
    #[must_use]
    pub fn start(inputs: ServiceInputs) -> Self {
        // A transport of this service's own. `CdpPortTransport` disconnects its previous client
        // each time it reconnects, so one shared with another hub would cut that hub's streams.
        let capture = Arc::new(CdpPortTransport::new(inputs.cdp_port));
        Self::start_with_capture(inputs, capture)
    }

    /// [`Service::start`], with live views captured over `capture` instead of a connection to
    /// `inputs.cdp_port`: tests pass `neo_screencast::testing`'s scripted browser.
    ///
    /// # Panics
    /// Outside a Tokio runtime, as `tokio::spawn` does.
    #[doc(hidden)]
    #[must_use]
    pub fn start_with_capture(inputs: ServiceInputs, capture: Arc<dyn CaptureTransport>) -> Self {
        let ServiceInputs {
            store,
            sink,
            browser,
            viewports,
            screenshots,
            xvnc,
            cdp_port: _,
            settings,
            tab_owner,
        } = inputs;
        let events = EventLog::start(store.clone(), sink.subscribe());
        let locks = Locks::new(store.clone(), sink.clone());
        events.follow_locks(locks.subscribe());
        Camera {
            browser: Arc::clone(&browser),
            viewports: Arc::clone(&viewports),
        }
        .spawn(screenshots);
        let tab_inventory = ProtocolSession::root(browser.connection());
        let browser = BrowserTabs::new(browser, viewports);
        let fits = Fits::new(browser.clone(), xvnc);
        let stamps = Arc::new(TabStamps::default());
        let displays = Displays::new(
            xvnc.addr,
            browser.clone(),
            Arc::clone(&locks),
            sink.clone(),
            store.clone(),
        );
        let takeovers = Takeovers::new(Arc::clone(&locks), Arc::clone(&fits), browser.clone());
        events.follow_takeovers(takeovers.subscribe());
        let front = displays.front();
        // A Neo restart in the middle of a fit drops the fit, but would leave Xvnc at the
        // owner's size and the window fullscreen, and nothing else would ever put them back.
        // Best-effort and logged. A fit that takes the surface first wins; its release puts
        // the surface home instead.
        tokio::spawn({
            let fits = Arc::clone(&fits);
            async move { fits.put_surface_home().await }
        });
        let shutdown = CancellationToken::new();
        let live = CaptureHub::new(
            Arc::new(LiveTabs::new(browser.clone())),
            capture,
            shutdown.clone(),
        );
        Self {
            state: Arc::new(ServiceState {
                store,
                sink,
                events,
                browser,
                tab_inventory,
                locks,
                fits,
                displays,
                takeovers,
                front,
                live,
                stamps,
                settings,
                tab_owner,
                _live_shutdown: shutdown.drop_guard(),
            }),
        }
    }

    /// Every `/v1` route, each response stamped with a request id. Serve it through
    /// [`host_service`](crate::host_service), which puts the host auth in front.
    pub fn router(&self) -> Router {
        router::router(Arc::clone(&self.state))
    }

    /// The tab locks the routes take. The app's lock policy asks [`Locks::check`] about every
    /// agent operation, and its tab cleanup never closes a tab [`Locks::is_locked`] names.
    #[must_use]
    pub fn locks(&self) -> &Arc<Locks> {
        &self.state.locks
    }

    /// The agent's side of Neo's one screen. The app's door asks [`Front::check`] about every op
    /// that moves the front, at admission and at every recheck, and its `page.bringToFront()`
    /// moves it through [`Front::bring_to_front`]. Arm it together with [`Service::locks`]: both
    /// are the owner's claims, the tab and the screen.
    #[must_use]
    pub fn front(&self) -> &Front {
        &self.state.front
    }

    /// Whether a lock or a fit holds `tab` now, or when the last of them let go of it within the
    /// hour. The app's idle-tab janitor never closes a held tab and restarts its idle clock
    /// from the let-go: a lock is how a host keeps the agent off, and a fit is how it shows the
    /// tab to the owner, so the owner may be using it either way. Never waits.
    #[must_use]
    pub fn tab_hold(&self, tab: TabId) -> Option<TabHold> {
        TabHold::latest_of(self.state.locks.hold(tab), self.state.fits.hold(tab))
    }

    /// The browser link saw a tab close. Its stamp is forgotten first, so the close waits out a
    /// reconcile's parts before it touches the tab's state, as a route does (`tab_stamps`). Then
    /// its takeover is released, while its lock still holds, so the release lifts the lock itself
    /// and gives back what it took; then its fit is restored (so the screen does not stay at the
    /// owner's size), its locks end, and `tab.closed` is
    /// published, so a host that reads it finds the tab's state already gone.
    pub async fn tab_closed(&self, tab: TabId) {
        self.state.stamps.forget(tab).await;
        self.state.takeovers.tab_closed(tab).await;
        self.state.fits.tab_closed(tab).await;
        self.state.locks.tab_closed(tab);
        self.state.events.publish(LogEvent::new(
            EventKind::TabClosed,
            None,
            Some(tab),
            &wire::tab_closed_event(tab, EpochMillis::now()),
        ));
    }

    /// Where a reconcile's listing begins among the host's changes to tabs. Take it before
    /// listing the tabs, and hand it to [`Service::retain_tabs`] with that listing.
    #[must_use]
    pub fn tab_fence(&self) -> TabFence {
        self.state.stamps.fence()
    }

    /// The browser link (re)connected with `open` tabs, listed after `fence` was taken. Whatever
    /// the parts keep about any other tab ends, as if it closed: it closed while Neo was not
    /// watching, or the browser restarted and may reuse a tab number. A tab a host changed
    /// after `fence` is kept as if it were open, since it may have opened after the listing.
    /// Host changes to tabs wait until every part is done (`tab_stamps`).
    pub async fn retain_tabs(&self, open: &[TabId], fence: TabFence) {
        let kept = self.state.stamps.kept(open, fence).await;
        self.state.takeovers.retain_tabs(&kept.tabs).await;
        self.state.fits.retain_tabs(&kept.tabs).await;
        self.state.locks.retain_tabs(&kept.tabs);
    }
}
