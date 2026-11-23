//! Ordered notice of the tabs that open and close, for whatever keeps state by tab.

use std::collections::{BTreeSet, HashMap};
use std::mem;
use std::sync::Arc;
use std::time::Duration;

use browseros_cdp::CdpEvent;
use browseros_core::pages::PageInfo;
use browseros_core::{BrowserSession, CdpConnection, TabId, TargetId};
use serde::Deserialize;
use serde_json::json;
use tokio::sync::broadcast::error::{RecvError, TryRecvError};
use tokio::sync::broadcast::{self, Receiver};
use tokio::sync::watch;
use tokio::time::{self, Instant, MissedTickBehavior};

use super::tab_life::{Gap, ListedTab, Listing, TabTarget};
use super::{PageRef, TabRegistry};

/// How often the watch looks for a replaced socket and retries a listing that failed. The
/// client publishes no reconnect event, and target discovery dies with the old socket, so this
/// bounds how long closes go unseen after a reconnect.
const EPOCH_CHECK: Duration = Duration::from_secs(1);

/// A change to the browser's tabs, in the order `tab_watch` saw it.
///
/// The host API's parts key their state by tab (locks and fits), so they learn each close
/// here rather than by comparing listings, which loses a tab that opened and closed between
/// two of them. A close is final only on its own socket: after a browser restart an old tab id
/// can name a new tab, so each event names the socket epoch it was seen on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TabEvent {
    /// The watch starts afresh: every tab that closes from now on is reported, and what
    /// happened before is not. Each new socket starts one, and so does missing core's events,
    /// which may have held a close; the reader reconciles against a listing taken under it.
    Watching(Watched),
    /// A tab is open, and was not at the listing before.
    Opened {
        /// The socket the listing that showed it came from.
        epoch: u64,
        /// The tab.
        tab: TabId,
    },
    /// A tab closed: its page target was destroyed, and a listing taken after that lacks it.
    Closed {
        /// The socket the target event came from.
        epoch: u64,
        /// The tab.
        tab: TabId,
    },
}

/// One start of the watch: the socket it follows, and which start it is. A plain data bag.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Watched {
    /// The connection epoch of the socket whose target events the watch follows.
    pub(crate) epoch: u64,
    /// Counts the watch's starts in this process. A listing taken under one start covers every
    /// gap before it, so a [`TabEvent::Watching`] no newer than that listing asks for nothing.
    pub(crate) generation: u64,
}

/// The open tabs at one moment, listed on the socket a watch start follows, so every tab that
/// closes later is reported as [`TabEvent::Closed`]. A plain data bag.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TabListing {
    /// The watch start the listing was taken under.
    pub(crate) watched: Watched,
    /// The open tabs. Core leaves out its excluded pages (`chrome-extension://`, `devtools://`).
    pub(crate) open: Vec<TabId>,
}

/// Tells, in order, which tabs open and close, and keeps core's page registry honest about tabs
/// someone else opened or closed.
///
/// ```text
///  new socket ─ Target.setDiscoverTargets ─▶ Watching ─▶ listing
///  Target.targetCreated (page) ─ its tab ─▶ targets
///  Target.targetDestroyed ─ targets[id] ─▶ closing ─▶ listing ─▶ Closed: closing, not listed
///                                                             └▶ Opened: listed, not before
/// ```
///
/// A destroy names only its target, so the watch keeps each page target's tab, from the
/// `tabId` BrowserOS adds to a target's info and from every listing. A destroyed page target
/// makes its tab a candidate, never a close by itself: a tab can change target and stay open.
/// The close is reported once a listing taken after the destroy lacks the tab. Every destroy is
/// noted before any listing, so a burst of events, or a tab that opened and closed between two
/// listings, still reports its close; a burst costs one listing, and so does a reconnect.
///
/// Each listing also fires core's detach hook for the tabs it lacks, which drops them from
/// `TabRegistry`: core notices a closed tab only on its own close or its next listing.
///
/// A popup's opener goes to the registry twice: at its announcement, for sight only
/// (`TabRegistry::note_opener`), so a task's popup is the task's before any listing; and at the
/// next listing, which registers the popup and lets it join its opener's task
/// (`TabRegistry::popup`).
///
/// It is also the only writer of browser facts to tab telemetry's ledger (`TabLife`): every
/// settled listing, with what the watch may have missed before it (`Gap`), and each page
/// target's `Target.targetCrashed`, idle tab or not, and its `Target.targetInfoChanged`, which
/// after a crash means a page loaded in it again. They ride the listing and the discovery stream
/// the watch already has; only after core's channel dropped events does one `Target.getTargets`
/// rebuild the watch's map of live page targets, which the ledger needs to keep a tab on an
/// excluded page open. A timer settles the closes the ledger holds pending: as
/// the user's two seconds on, or as the shutdown's if the socket dropped meanwhile.
///
/// Runs for the process's life: the session's event channel outlives every reconnect.
pub(crate) async fn tab_watch(
    session: Arc<BrowserSession>,
    tabs: Arc<TabRegistry>,
    events: broadcast::Sender<TabEvent>,
    watched: watch::Sender<Option<Watched>>,
) {
    let connection = session.connection();
    // Subscribed before discovery is asked for, so no target event can slip in between.
    let mut cdp_events = session.cdp_events();
    let mut checks = time::interval(EPOCH_CHECK);
    checks.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut watch = Watch::new(events, watched, tabs);
    loop {
        // A close nobody asked for waits in the tab ledger to be told apart from a browser exit;
        // this wakes the watch when the next one is due.
        let settle_at = watch.tabs.life().next_settle();
        tokio::select! {
            _ = checks.tick() => {
                let epoch = connection.connection_epoch();
                if !connection.is_connected() {
                    continue;
                }
                if watch.follows(epoch) {
                    if watch.owes_listing {
                        watch.list(&session).await;
                    }
                    continue;
                }
                // Queued events came from the old socket, and the new socket's listing
                // supersedes them. Its own target events start only once discovery is on.
                drop_queued(&mut cdp_events);
                // At this epoch, so a socket replaced mid-call fails here and is retried on the
                // next check instead of being marked as watched.
                let discover = connection
                    .send_at_epoch(epoch, "Target.setDiscoverTargets", json!({ "discover": true }), None)
                    .await;
                match discover {
                    Ok(_) => {
                        watch.start(epoch);
                        // Chromium answers discovery by announcing every target that exists,
                        // before its reply: their tabs are known before the first destroy.
                        watch.note_queued(&mut cdp_events);
                        watch.list(&session).await;
                    }
                    Err(error) => tracing::debug!(%error, epoch, "target discovery not enabled yet"),
                }
            }
            event = cdp_events.recv() => match event {
                Ok(event) => {
                    if watch.note(&event) {
                        watch.note_queued(&mut cdp_events);
                        watch.list(&session).await;
                    }
                }
                Err(RecvError::Lagged(_)) => {
                    watch.fell_behind();
                    watch.list(&session).await;
                }
                Err(RecvError::Closed) => return,
            },
            () = time::sleep_until(settle_at.unwrap_or_else(Instant::now)), if settle_at.is_some() => {
                watch.settle(&*connection);
            }
        }
    }
}

/// The open tabs, when `session` lists them on the socket `watched` follows, from end to end.
/// `None` when the browser did not answer or another socket did.
pub(super) async fn watched_listing(
    session: &BrowserSession,
    watched: Watched,
) -> Option<TabListing> {
    let pages = watched_pages(session, watched.epoch).await?;
    let open: Vec<TabId> = pages.iter().map(|page| page.tab_id).collect();
    Some(TabListing { watched, open })
}

/// Core's listing of the open pages, when the socket at `epoch` answered all of it.
async fn watched_pages(session: &BrowserSession, epoch: u64) -> Option<Vec<PageInfo>> {
    let connection = session.connection();
    if connection.connection_epoch() != epoch {
        return None;
    }
    let pages = match session.pages.list().await {
        Ok(pages) => pages,
        Err(error) => {
            tracing::debug!(%error, "tab listing failed");
            return None;
        }
    };
    // Epochs only grow, so the same epoch after the listing means one socket answered it all.
    (connection.connection_epoch() == epoch).then_some(pages)
}

/// What the watch knows about the socket it follows, and where it tells what it sees.
struct Watch {
    events: broadcast::Sender<TabEvent>,
    watched: watch::Sender<Option<Watched>>,
    /// The current start; `None` until target discovery is first on.
    current: Option<Watched>,
    /// The followed socket's page targets, by target id, with their tab.
    targets: HashMap<String, TabId>,
    /// CDP reports openerId even when no script subscribed to popup events.
    openers: HashMap<String, String>,
    /// Each popup target's opener tab, resolved as the browser announced the popup, while the
    /// opener was still open: an opener that closes before the next listing leaves `targets`,
    /// and the registry drops its links, yet the open still names it.
    opener_tabs: HashMap<String, TabId>,
    tabs: Arc<TabRegistry>,
    /// Tabs whose page target was destroyed, until a listing says whether each closed or only
    /// changed target.
    closing: BTreeSet<TabId>,
    /// The tabs the last listing had; `None` before the first, whose tabs were open before Neo
    /// started, so none of them counts as opened.
    open: Option<BTreeSet<TabId>>,
    /// A listing is due: target events came since the last one, or it failed.
    owes_listing: bool,
    /// What the next listing must allow for having missed, for the ledger: a new socket, or
    /// events core's channel dropped. Each listing that settles takes it.
    gap: Gap,
    /// `targets` may hold a page target whose destroy was among dropped events, or lack one whose
    /// announcement was: the watch fell behind, and the next listing first rebuilds the map
    /// (`Watch::rebuild_targets`). Until a rebuild succeeds the ledger is not told which unlisted
    /// tabs have a live target, since a dead one would keep a closed tab counted for good.
    targets_stale: bool,
}

/// `Target.targetCreated`, as far as the watch reads it.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TargetCreated {
    target_info: TargetInfo,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TargetInfo {
    target_id: String,
    #[serde(rename = "type")]
    kind: String,
    /// BrowserOS adds the tab of a target that belongs to one; stock Chromium does not.
    #[serde(default)]
    tab_id: Option<i64>,
    #[serde(default)]
    opener_id: Option<String>,
}

/// `Target.getTargets`' reply, as far as the watch reads it.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TargetInfos {
    target_infos: Vec<TargetInfo>,
}

/// `Target.targetDestroyed`, which names only the target.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TargetDestroyed {
    target_id: String,
}

/// `Target.targetCrashed`, as far as the watch reads it: the target whose renderer died.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TargetCrashed {
    target_id: String,
}

/// `Target.targetInfoChanged`, as far as the watch reads it: a target whose page changed, which
/// for a crashed one means a page loaded in it again.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TargetInfoChanged {
    target_info: ChangedTarget,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ChangedTarget {
    target_id: String,
}

impl Watch {
    fn new(
        events: broadcast::Sender<TabEvent>,
        watched: watch::Sender<Option<Watched>>,
        tabs: Arc<TabRegistry>,
    ) -> Self {
        Self {
            events,
            watched,
            current: None,
            targets: HashMap::new(),
            openers: HashMap::new(),
            opener_tabs: HashMap::new(),
            tabs,
            closing: BTreeSet::new(),
            open: None,
            owes_listing: false,
            gap: Gap::Socket,
            targets_stale: false,
        }
    }

    /// Whether the watch already follows socket `epoch`.
    fn follows(&self, epoch: u64) -> bool {
        self.current.is_some_and(|current| current.epoch == epoch)
    }

    /// Discovery is on for socket `epoch`. The old socket's targets and pending closes go:
    /// target ids are that browser's, and its closes are the reader's to reconcile now.
    fn start(&mut self, epoch: u64) {
        self.targets.clear();
        self.openers.clear();
        self.opener_tabs.clear();
        self.closing.clear();
        self.tabs.forget_openers();
        // Discovery announces every live target afresh, so the map is whole again.
        self.gap = Gap::Socket;
        self.targets_stale = false;
        self.begin(epoch);
    }

    /// Core's event channel dropped events the watch never read, which may have held a close:
    /// a new start on the same socket. The known targets stay, so a destroy still to come
    /// finds its tab, until the next listing rebuilds them.
    fn fell_behind(&mut self) {
        if let Some(current) = self.current {
            // A new socket's first listing still asks the ledger whether the browser changed.
            if self.gap != Gap::Socket {
                self.gap = Gap::Lagged;
            }
            self.targets_stale = true;
            self.begin(current.epoch);
        }
    }

    fn begin(&mut self, epoch: u64) {
        let generation = self.current.map_or(1, |current| current.generation + 1);
        let current = Watched { epoch, generation };
        self.current = Some(current);
        self.owes_listing = true;
        // The state first, so a reader woken by the event finds the start it announces.
        self.watched.send_replace(Some(current));
        self.send(TabEvent::Watching(current));
    }

    /// Records what `event` says about page targets. `true` when it may mean a tab opened or
    /// closed, so a listing is due. A destroyed target carries no type, so every destroy
    /// counts; a created one only as a page, since frames and workers come and go with every
    /// page load.
    fn note(&mut self, event: &CdpEvent) -> bool {
        match event.method.as_str() {
            "Target.targetCreated" => {
                let Ok(TargetCreated { target_info }) = TargetCreated::deserialize(&event.params)
                else {
                    return false;
                };
                if target_info.kind != "page" {
                    return false;
                }
                let opener_tab = target_info
                    .opener_id
                    .as_ref()
                    .and_then(|opener| self.targets.get(opener))
                    .copied();
                if let Some(opener) = opener_tab {
                    if let Some(tab) = target_info.tab_id {
                        self.tabs.note_opener(TabId(tab), opener);
                    }
                    self.opener_tabs
                        .insert(target_info.target_id.clone(), opener);
                }
                if let Some(opener) = target_info.opener_id {
                    self.openers.insert(target_info.target_id.clone(), opener);
                }
                if let Some(tab) = target_info.tab_id {
                    self.targets.insert(target_info.target_id, TabId(tab));
                }
            }
            "Target.targetDestroyed" => {
                if let Ok(TargetDestroyed { target_id }) =
                    TargetDestroyed::deserialize(&event.params)
                    && let Some(tab) = self.targets.remove(&target_id)
                {
                    self.closing.insert(tab);
                }
            }
            // Counted here and nowhere else (not from ops that fail on the dead tab), so an idle
            // tab's crash counts as an active one's does. A crash opens and closes nothing.
            "Target.targetCrashed" => {
                if let Ok(TargetCrashed { target_id }) = TargetCrashed::deserialize(&event.params)
                    && let Some(tab) = self.targets.get(&target_id).copied()
                {
                    self.tabs.life().crashed(tab, &TargetId(target_id));
                }
                return false;
            }
            "Target.targetInfoChanged" => {
                // The ledger knows which renderers crashed; for any other target this is no-op.
                if let Ok(TargetInfoChanged { target_info }) =
                    TargetInfoChanged::deserialize(&event.params)
                    && let Some(tab) = self.targets.get(&target_info.target_id).copied()
                {
                    self.tabs
                        .life()
                        .recovered(tab, &TargetId(target_info.target_id));
                }
                return false;
            }
            _ => return false,
        }
        self.owes_listing = true;
        true
    }

    /// Notes every event already queued, so a burst costs one listing. None is dropped: a
    /// destroy among them still makes its tab a candidate before the listing.
    fn note_queued(&mut self, cdp_events: &mut Receiver<CdpEvent>) {
        loop {
            match cdp_events.try_recv() {
                Ok(event) => {
                    self.note(&event);
                }
                Err(TryRecvError::Lagged(_)) => self.fell_behind(),
                Err(TryRecvError::Empty | TryRecvError::Closed) => return,
            }
        }
    }

    /// Lists the tabs, which reconciles core's registry, then reports what the listing settles:
    /// the closes it confirms and the tabs it shows newly open. A listing that failed, or that
    /// another socket answered, settles nothing and stays due.
    async fn list(&mut self, session: &BrowserSession) {
        let Some(current) = self.current else {
            // Before discovery is on nothing is reported, but core still learns of closes.
            if let Err(error) = session.pages.list().await {
                tracing::debug!(%error, "tab listing failed");
            }
            return;
        };
        if self.targets_stale {
            self.rebuild_targets(session, current.epoch).await;
        }
        let seen = self.tabs.seen();
        let Some(pages) = watched_pages(session, current.epoch).await else {
            return;
        };
        self.owes_listing = false;
        for page in &pages {
            self.targets
                .insert(page.target_id.as_str().to_owned(), page.tab_id);
        }
        for page in &pages {
            if let Some(opener) = self
                .openers
                .get(page.target_id.as_str())
                .and_then(|target| self.targets.get(target))
            {
                self.tabs.popup(&PageRef::from(page), *opener, seen);
            }
        }
        self.openers
            .retain(|target, _| pages.iter().any(|page| page.target_id.as_str() == target));
        self.opener_tabs
            .retain(|target, _| pages.iter().any(|page| page.target_id.as_str() == target));
        self.tell_ledger(&pages);
        let open: BTreeSet<TabId> = pages.iter().map(|page| page.tab_id).collect();
        // Opens go out before closes, each first to the registry's open sink, synchronously: a
        // takeover whose tab closed in this listing is released by its close, and a tab the
        // owner opened in the same moment must be in its notes first. The sink, not the event,
        // carries opens to the takeovers, since the event's reader waits out each close's
        // release before it reads on.
        if let Some(before) = &self.open {
            for &tab in open.difference(before) {
                let opener = pages
                    .iter()
                    .find(|page| page.tab_id == tab)
                    .and_then(|page| self.opener_tabs.get(page.target_id.as_str()))
                    .copied();
                self.tabs.opened(tab, opener);
                self.send(TabEvent::Opened {
                    epoch: current.epoch,
                    tab,
                });
            }
        }
        for tab in mem::take(&mut self.closing) {
            if !open.contains(&tab) {
                self.tabs.forget_announced(tab);
                self.send(TabEvent::Closed {
                    epoch: current.epoch,
                    tab,
                });
            }
        }
        self.open = Some(open);
    }

    /// Settles the tab ledger's pending closes: if the socket the watch follows
    /// dropped or was replaced since, they were the browser exiting and count as `shutdown`;
    /// otherwise those whose wait is over count as the user's. Asked of the connection now, so a
    /// drop after a close's wait ended does not reach back to it.
    fn settle(&self, connection: &dyn CdpConnection) {
        let life = self.tabs.life();
        let followed = self
            .current
            .is_some_and(|current| current.epoch == connection.connection_epoch());
        if !followed || !connection.is_connected() {
            life.socket_dropped();
        }
        life.settle(Instant::now());
    }

    /// After core's channel dropped events, rebuilds the map of live page targets from one
    /// `Target.getTargets` on socket `epoch`, before the listing: a destroy among the dropped
    /// events would leave a dead target in it, and an announcement a missing one. Asked first, so
    /// a tab that closes between the two is unlisted but still mapped, and its destroy, read
    /// later, finds it. On failure the map stays stale, and the next listing asks again.
    async fn rebuild_targets(&mut self, session: &BrowserSession, epoch: u64) {
        let reply = session
            .connection()
            .send_at_epoch(epoch, "Target.getTargets", json!({}), None)
            .await;
        let infos = match reply.map(|value| TargetInfos::deserialize(&value)) {
            Ok(Ok(TargetInfos { target_infos })) => target_infos,
            Ok(Err(error)) => {
                tracing::debug!(%error, "target listing unreadable; live targets stay unknown");
                return;
            }
            Err(error) => {
                tracing::debug!(%error, "target listing failed; live targets stay unknown");
                return;
            }
        };
        self.targets.clear();
        for info in infos.into_iter().filter(|info| info.kind == "page") {
            if let Some(opener) = info.opener_id {
                self.openers.insert(info.target_id.clone(), opener);
            }
            if let Some(tab) = info.tab_id {
                self.targets.insert(info.target_id, TabId(tab));
            }
        }
        self.targets_stale = false;
    }

    /// Hands this listing to tab telemetry's ledger, with the close candidates and the gap since
    /// the last one, before the candidates are settled below. The ledger keeps its own counts,
    /// so nothing here changes what the watch reports.
    fn tell_ledger(&mut self, pages: &[PageInfo]) {
        let listed: Vec<ListedTab> = pages
            .iter()
            .map(|page| ListedTab {
                tab: page.tab_id,
                target: page.target_id.clone(),
                opener: self.opener_of(page.target_id.as_str()),
            })
            .collect();
        let destroyed: Vec<TabId> = self.closing.iter().copied().collect();
        let targeted: Vec<TabTarget> = if self.targets_stale {
            Vec::new()
        } else {
            self.targets
                .iter()
                .map(|(target, tab)| TabTarget {
                    tab: *tab,
                    target: TargetId(target.clone()),
                })
                .collect()
        };
        self.tabs.life().listed(Listing {
            tabs: &listed,
            destroyed: &destroyed,
            targeted: &targeted,
            gap: mem::replace(&mut self.gap, Gap::None),
            now: Instant::now(),
        });
    }

    /// The tab that opened page target `target`: as the browser announced it, else through its
    /// announced opener target, which a new socket's discovery may announce after the popup.
    fn opener_of(&self, target: &str) -> Option<TabId> {
        self.opener_tabs.get(target).copied().or_else(|| {
            self.openers
                .get(target)
                .and_then(|opener| self.targets.get(opener))
                .copied()
        })
    }

    fn send(&self, event: TabEvent) {
        // No reader is fine: the host API subscribes once the link is up.
        let _ = self.events.send(event);
    }
}

/// Drops every queued event: they came from a socket the watch no longer follows.
fn drop_queued(cdp_events: &mut Receiver<CdpEvent>) {
    loop {
        match cdp_events.try_recv() {
            Ok(_) | Err(TryRecvError::Lagged(_)) => {}
            Err(TryRecvError::Empty | TryRecvError::Closed) => return,
        }
    }
}
