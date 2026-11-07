//! The CDP link and the one browser session.

use std::future::Future;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use browseros_cdp::{CdpClient, CdpError, ConnectOptions, ReconnectPolicy};
use browseros_core::pages::{PageInfo, PageManagerHooks};
use browseros_core::sensitive_fields::SensitiveValues;
use browseros_core::viewport::Viewports;
use browseros_core::{
    BrowserSession, BrowserSessionHooks, BrowserView, CdpConnection, NativeCapabilities, PageId,
    TabId, page_js,
};
use tokio::sync::{SetOnce, broadcast, watch};
use tokio::time;

use super::{TabEvent, TabListing, TabRegistry, Watched, tab_watch};

/// The first retry after a failed first connect; it doubles up to [`LAST_RETRY`].
const FIRST_RETRY: Duration = Duration::from_secs(1);
/// The longest wait between first-connect attempts, as claw-server's reattach loop.
const LAST_RETRY: Duration = Duration::from_secs(30);
/// The pause before [`BrowserLink::watched_listing`] asks again after the browser did not answer.
const LISTING_RETRY: Duration = Duration::from_secs(1);
/// Tab events a slow reader may fall behind by before it sees `Lagged`, and reconciles instead.
const TAB_EVENTS: usize = 1024;

/// Neo's one link to Chromium.
///
/// Chromium launches the sidecar before its CDP port is guaranteed up, so [`BrowserLink::start`]
/// returns at once and a background loop connects. Health answers meanwhile; readiness flips
/// when the first connection lands. From then on the CDP client reconnects by itself, and this
/// link only reports what the client sees.
pub(crate) struct BrowserLink {
    /// Hosted viewport modes, shared by every run's view and the host API's camera, so leases
    /// are process-wide and a capture never overlaps a viewport change.
    viewports: Arc<Viewports>,
    /// Set once, on the first successful connect. `BrowserSession::new_with_native` spawns
    /// tasks that never end, so a process has exactly one. The host API waits on it
    /// ([`BrowserLink::linked`]): it screenshots on this session, never on one of its own.
    session: SetOnce<Arc<BrowserSession>>,
    /// Why the latest first-connect attempt failed; cleared once linked.
    last_error: Mutex<Option<String>>,
    /// `tab_watch`'s opens and closes, in order. A reader that falls behind gets `Lagged` and
    /// must reconcile against [`BrowserLink::watched_listing`], since it may have missed a close.
    tab_events: broadcast::Sender<TabEvent>,
    /// The watch start `tab_watch` is on: which socket's target events it follows, if any.
    watched: watch::Sender<Option<Watched>>,
    /// `tab_watch`'s latest listing: core's page registry right after it reconciled with the
    /// browser, on the socket it was taken on. Read synchronously by [`BrowserLink::lists_tab`],
    /// on the send path, where core's own registry (behind an async lock) cannot be asked.
    listed: watch::Sender<Option<TabListing>>,
}

/// The browser as one run sees it: an unbound view and the CDP connection epoch it was taken
/// at. An unbound view silently follows a reconnect onto the new socket, so the run keeps the
/// epoch and the dispatcher refuses once it moves (`neo / browser_disconnected`). A plain data
/// bag.
pub(crate) struct RunView {
    /// Every tab, with the process's viewports, and no admission round trips.
    pub(crate) view: BrowserView,
    /// The connection epoch at capture: +1 per socket the client installs.
    pub(crate) epoch: u64,
}

/// What `/system/health` and `/system/ready` report about the browser. A plain data bag.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LinkStatus {
    /// Linked and connected now: runs can start.
    pub(crate) connected: bool,
    /// The current connection epoch; `None` before the first connect.
    pub(crate) epoch: Option<u64>,
    /// Why the latest first-connect attempt failed, until one succeeds.
    pub(crate) last_error: Option<String>,
}

impl BrowserLink {
    /// Starts connecting to Chromium's CDP port and returns at once.
    ///
    /// The first `CdpClient::connect` is not covered by the client's own reconnects (it returns
    /// `Err`), so the loop retries it. After that the client keeps trying on its own: the
    /// default `ReconnectPolicy::Exit(1)` would exit the sidecar after three failed
    /// reconnects, which Chromium would read as a crash.
    pub(crate) fn start(
        cdp_port: u16,
        native: NativeCapabilities,
        tabs: Arc<TabRegistry>,
    ) -> Arc<Self> {
        let options = connect_options(cdp_port);
        Self::start_with(native, tabs, move || {
            let options = options.clone();
            async move {
                let client = CdpClient::connect(options).await?;
                Ok(Arc::new(client) as Arc<dyn CdpConnection>)
            }
        })
    }

    /// Like [`BrowserLink::start`], over any transport: `connect` is called until it succeeds,
    /// with backoff. Tests pass `test_support::FakeCdp`.
    pub(crate) fn start_with<F, Fut>(
        native: NativeCapabilities,
        tabs: Arc<TabRegistry>,
        connect: F,
    ) -> Arc<Self>
    where
        F: FnMut() -> Fut + Send + 'static,
        Fut: Future<Output = Result<Arc<dyn CdpConnection>, CdpError>> + Send + 'static,
    {
        let link = Arc::new(Self {
            viewports: Arc::new(Viewports::default()),
            session: SetOnce::new(),
            last_error: Mutex::new(None),
            tab_events: broadcast::channel(TAB_EVENTS).0,
            watched: watch::Sender::new(None),
            listed: watch::Sender::new(None),
        });
        tokio::spawn(connect_loop(Arc::clone(&link), native, tabs, connect));
        link
    }

    /// What the link looks like now, from the client's own state; never a CDP call.
    #[must_use]
    pub(crate) fn status(&self) -> LinkStatus {
        LinkStatus {
            connected: self.is_ready(),
            epoch: self.epoch(),
            last_error: self
                .last_error
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone(),
        }
    }

    /// Whether runs can start: linked, and connected now.
    #[must_use]
    pub(crate) fn is_ready(&self) -> bool {
        self.session
            .get()
            .is_some_and(|session| session.is_connected())
    }

    /// A fresh view for one run, with the epoch it was taken at. `None` until connected.
    /// Cheap: no CDP call.
    #[must_use]
    pub(crate) fn view(&self) -> Option<RunView> {
        let session = self
            .session
            .get()
            .filter(|session| session.is_connected())?;
        // Read before the view: if the socket is replaced in between, the run holds the older
        // epoch and is refused, never the reverse.
        let epoch = session.connection().connection_epoch();
        let view = BrowserView::unbound(Arc::clone(session), None)
            .with_viewports(Arc::clone(&self.viewports));
        Some(RunView { view, epoch })
    }

    /// Whether core knew `tab` when `tab_watch` last listed the browser on socket `epoch`, with no
    /// CDP call and no wait: for the door's rules on each tab a main-world script can reach
    /// (`OpCtx::recheck_reach`), which run at a `waitForFunction`'s poll rate. `tab_watch` lists
    /// after every page target that opens or closes, so a tab counts from the listing that first
    /// has it until the first that lacks it. Never for a page core leaves out of its registry (an
    /// extension's, or the developer tools'), and never from another socket's listing: Chromium
    /// numbers its tabs from 1 again after a restart, so an old id may name another tab.
    #[must_use]
    pub(crate) fn lists_tab(&self, epoch: u64, tab: TabId) -> bool {
        self.listed
            .borrow()
            .as_ref()
            .is_some_and(|listing| listing.watched.epoch == epoch && listing.open.contains(&tab))
    }

    /// The current CDP epoch; `None` before the first connect. A run whose epoch differs was
    /// captured on a socket that has since been replaced.
    #[must_use]
    pub(crate) fn epoch(&self) -> Option<u64> {
        self.session
            .get()
            .map(|session| session.connection().connection_epoch())
    }

    /// Whether a run captured at `epoch` still talks to the same socket: linked, connected, and
    /// no reconnect since.
    #[must_use]
    pub(crate) fn is_current(&self, epoch: u64) -> bool {
        self.session.get().is_some_and(|session| {
            session.is_connected() && session.connection().connection_epoch() == epoch
        })
    }

    /// Core's cached facts for a page, without a CDP call. Stale after an unbound navigation
    /// until something refreshes the page; `None` once core saw the tab close.
    pub(crate) async fn cached_page(&self, page: &PageId) -> Option<PageInfo> {
        self.session.get()?.pages.get_info(page.clone()).await
    }

    /// The one session, once the first connect landed; until then this waits. The host API
    /// starts on it, so its camera and its tab checks act on the agent's own connection.
    pub(crate) async fn linked(&self) -> Arc<BrowserSession> {
        Arc::clone(self.session.wait().await)
    }

    /// The process's viewports, which every run's view already shares.
    #[must_use]
    pub(crate) fn viewports(&self) -> Arc<Viewports> {
        Arc::clone(&self.viewports)
    }

    /// `tab_watch`'s opens and closes from now on, in the order it saw them.
    #[must_use]
    pub(crate) fn tab_events(&self) -> broadcast::Receiver<TabEvent> {
        self.tab_events.subscribe()
    }

    /// The open tabs, listed now on the socket `tab_watch` follows, so every tab that closes
    /// after the listing reaches [`BrowserLink::tab_events`] as a close. Waits for the first
    /// connect and for the watch to follow the current socket, and asks again while the
    /// browser does not answer. As new as one `Browser.getTabs` round trip allows.
    pub(crate) async fn watched_listing(&self) -> TabListing {
        let session = self.linked().await;
        let mut watched = self.watched.subscribe();
        loop {
            let current = *watched.borrow_and_update();
            if let Some(current) = current
                && let Some(listing) = tab_watch::watched_listing(&session, current).await
            {
                return listing;
            }
            // A new start (discovery on a new socket), or a pause before asking again.
            let _ = time::timeout(LISTING_RETRY, watched.changed()).await;
        }
    }

    /// The one session, for tests that check how it was built.
    #[cfg(test)]
    pub(crate) fn session(&self) -> Option<&Arc<BrowserSession>> {
        self.session.get()
    }

    /// Builds the session on the first successful connect. Called once: the connect loop ends
    /// right after.
    fn link(
        &self,
        connection: Arc<dyn CdpConnection>,
        native: NativeCapabilities,
        tabs: Arc<TabRegistry>,
    ) {
        // Core's DOM scripts run in BrowserOS's isolated world, never where the page can see
        // them (skeleton D4). Process-wide, and set before any page is touched.
        page_js::set_isolated_world_enabled(true);
        let on_page_detached = Arc::new(move |page: PageId| tabs.closed(&page));
        let hooks = BrowserSessionHooks {
            page_manager: PageManagerHooks {
                on_page_detached: Some(on_page_detached),
                ..PageManagerHooks::default()
            },
        };
        let session = BrowserSession::new_with_native(connection, hooks, native);
        // sdk-privacy: snapshots show password, card and one-time-code values masked.
        session.set_sensitive_values(SensitiveValues::Mask);
        tokio::spawn(tab_watch::tab_watch(
            Arc::clone(&session),
            self.tab_events.clone(),
            self.watched.clone(),
            self.listed.clone(),
        ));
        // Cannot fail: this is the loop's only success, and nothing else sets the session.
        let _ = self.session.set(session);
        *self
            .last_error
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = None;
    }
}

/// Retries the first connect from [`FIRST_RETRY`] up to [`LAST_RETRY`], then links once.
async fn connect_loop<F, Fut>(
    link: Arc<BrowserLink>,
    native: NativeCapabilities,
    tabs: Arc<TabRegistry>,
    mut connect: F,
) where
    F: FnMut() -> Fut + Send,
    Fut: Future<Output = Result<Arc<dyn CdpConnection>, CdpError>> + Send,
{
    let mut backoff = FIRST_RETRY;
    let connection = loop {
        match connect().await {
            Ok(connection) => break connection,
            Err(error) => {
                tracing::warn!(%error, retry_secs = backoff.as_secs(), "cannot reach the browser over CDP yet");
                *link
                    .last_error
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner) = Some(error.to_string());
                time::sleep(backoff).await;
                backoff = (backoff * 2).min(LAST_RETRY);
            }
        }
    };
    tracing::info!(
        epoch = connection.connection_epoch(),
        "connected to the browser over CDP"
    );
    link.link(connection, native, tabs);
}

/// claw-server's options: a short connect timeout and one try per attempt (the loop above
/// retries), then unlimited reconnects.
fn connect_options(cdp_port: u16) -> ConnectOptions {
    ConnectOptions {
        connect_timeout: Duration::from_secs(2),
        connect_max_retries: 1,
        reconnect_policy: ReconnectPolicy::KeepTrying,
        reconnect_delay: Duration::from_secs(1),
        reconnect_max_retries: usize::MAX,
        ..ConnectOptions::new(cdp_port)
    }
}
