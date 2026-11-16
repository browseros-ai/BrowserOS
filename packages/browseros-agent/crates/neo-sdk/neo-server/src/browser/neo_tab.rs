//! Neo's one blank tab, which keeps the browser open whatever else closes.

use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use browseros_cdp::browser;
use browseros_core::{ProtocolSession, TabId, TargetId};
use futures_util::future::BoxFuture;
use neo_trace::TraceStore;
use serde_json::json;
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::{Mutex as AsyncMutex, SetOnce};
use tokio::time;
use tokio_util::sync::CancellationToken;

use super::{BrowserLink, TabEvent, TabRegistry};

/// What a blank tab shows: the browser's launch tab and a fresh tab both qualify.
const BLANK_URLS: &[&str] = &["about:blank", "chrome://newtab/", "chrome://new-tab-page/"];
/// One listing; a browser that does not answer is tried again at the next event.
const STEP_BUDGET: Duration = Duration::from_secs(10);
/// How often Neo looks whether the owner browsed in its tab while no script lists tabs.
/// `tab_watch` follows opens and closes, not navigations.
const RECHECK: Duration = Duration::from_secs(30);

/// Whether Neo's tab still shows nothing. A tab that has not committed its first URL reads empty.
fn still_blank(url: &str) -> bool {
    url.is_empty() || BLANK_URLS.contains(&url)
}

/// Tells running scripts a tab was handed back, with its target and address, as of a tab the
/// owner just opened (`Playwright::hand_back`). Set by the facade that runs Playwright.
pub(crate) type HandBack = Box<dyn Fn(TargetId, String) -> BoxFuture<'static, ()> + Send + Sync>;

/// What one [`NeoTab::ensure`] found.
#[derive(Debug, Default)]
struct Ensured {
    /// The tab Neo holds afterwards; `None` when the browser did not answer.
    tab: Option<TabId>,
    /// Every tab the browser listed, Neo's included; empty when it did not list them.
    listed: Vec<TabId>,
    /// A tab the owner browsed in, which Neo handed back.
    handed_back: Option<HandedBack>,
}

/// A tab Neo handed back, as running scripts hear of it.
#[derive(Debug)]
struct HandedBack {
    target: TargetId,
    url: String,
}

/// Whether [`NeoTab::ensure`] may take an existing blank tab for Neo.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Adopt {
    /// At the link's first connect and at each new watch start (a browser restart, a reconnect):
    /// the launch tab, or Neo's own tab restored by the browser, serves.
    Allowed,
    /// While the server runs: a script may have just listed an ordinary blank tab and not used it
    /// yet, and adopting it would refuse that script's next op. Neo opens a new one.
    Never,
}

/// Neo's blank tab (the owner, 2026-11-12: "keep a blank tab open always"). Closing the browser's
/// last tab quits the browser and Neo's server with it, so Neo keeps one
/// blank tab of its own open at all times: the janitor may then close every idle agent tab, a
/// script may close its last one, and a host's tab reset needs no keeper tab of its own.
///
/// ```text
///  link's first connect, a new watch start ─▶ ensure(Allowed): Neo's tab, or adopt a free blank
///                                             tab, or open one
///  Neo's tab closed (the owner, a race)     ─▶ ensure(Never): open a new one
///  page.close (a script, task.close, the janitor) ─▶ ensure(Never), then the close
///  Neo's tab shows a page (the owner browsed, a fit's navigate): seen by a script's listing
///  (`settle`), by any ensure, or within 30 s ─▶ open a new one, then hand the old one back
/// ```
///
/// Exactly one: one lock owns making it, and a blank tab found later stays an ordinary tab. The
/// registry marks it `TabOrigin::Neo`, so no script sees or reaches it (the door refuses it; the
/// engine leaves it out of listings and events, and checks those only through [`NeoTab::settle`],
/// so never in the moment before the mark), no task takes it, and the janitor never closes it. A
/// tab handed back is the owner's (`Found`): scripts list it and hear of it as a new page. Only
/// the sidecar keeps one: the Node-hosted embedding's host launched its own browser.
///
/// What it cannot rule out is the owner closing this tab in the moment the last other tab closes:
/// Neo opens a new one a round trip later, and between the two the browser may end.
pub(crate) struct NeoTab {
    link: Arc<BrowserLink>,
    tabs: Arc<TabRegistry>,
    /// The trace's task links: after a server restart the registry is empty, and a task's own
    /// blank tab must not look free.
    store: TraceStore,
    /// The tab Neo holds now. A std mutex, never held across an await.
    current: Mutex<Option<TabId>>,
    /// Held from an ensure's listing to its mark, so two cannot each open a tab, and a script's
    /// listing or page event is checked only once it is free (`settle`): the browser lists and
    /// announces a tab a moment before Neo can mark it.
    making: AsyncMutex<()>,
    hand_back: SetOnce<HandBack>,
}

impl NeoTab {
    /// Neo's blank tab over the app's link, registry and trace store. Nothing opens until
    /// [`NeoTab::follow`] or [`NeoTab::ensure`] runs.
    #[must_use]
    pub(crate) fn new(
        link: Arc<BrowserLink>,
        tabs: Arc<TabRegistry>,
        store: TraceStore,
    ) -> Arc<Self> {
        Arc::new(Self {
            link,
            tabs,
            store,
            current: Mutex::default(),
            making: AsyncMutex::new(()),
            hand_back: SetOnce::new(),
        })
    }

    /// From now on a handed-back tab is announced to running scripts through `hand_back`. A
    /// second call changes nothing.
    pub(crate) fn on_hand_back(&self, hand_back: HandBack) {
        let _ = self.hand_back.set(hand_back);
    }

    /// The tab Neo holds, for the host's tab listing.
    #[must_use]
    pub(crate) fn current(&self) -> Option<TabId> {
        *self.current.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Before a script is shown `listed`, a fresh listing with each tab's address: if Neo's tab
    /// in it shows a page, it is handed back first, so the script lists the owner's page at
    /// once. Otherwise waits until no blank tab of Neo's is being found or opened, so
    /// `TabRegistry::is_neo` knows every tab the browser listed or announced before this call.
    pub(crate) async fn settle(self: &Arc<Self>, listed: &[(TabId, &str)]) {
        let browsed = self.current().is_some_and(|held| {
            self.tabs.is_neo(held)
                && listed
                    .iter()
                    .any(|(tab, url)| *tab == held && !still_blank(url))
        });
        if browsed {
            self.ensure(Adopt::Never).await;
        } else {
            drop(self.making.lock().await);
        }
    }

    /// Makes sure Neo's blank tab is open, and returns it: the one Neo holds if the browser still
    /// lists it blank, else (with `Adopt::Allowed`) a free blank tab, else a new one. A tab the
    /// owner browsed in is handed back once the new one is Neo's. `None` when the browser does
    /// not answer; the next event tries again.
    pub(crate) async fn ensure(self: &Arc<Self>, adopt: Adopt) -> Option<TabId> {
        self.ensured(adopt).await.tab
    }

    /// Before Neo closes `closing` (a script's `page.close`, `task.close`, the janitor): makes sure
    /// of its blank tab first. Whether the close may go ahead: Neo's tab is open, or the browser
    /// listed a tab other than `closing`, so this close cannot quit the browser.
    pub(crate) async fn keep_before_close(self: &Arc<Self>, closing: TabId) -> bool {
        let ensured = self.ensured(Adopt::Never).await;
        ensured.tab.is_some() || ensured.listed.iter().any(|tab| *tab != closing)
    }

    /// [`NeoTab::ensure`], in a task of its own: a caller that stops waiting (its op aborted, its
    /// run ended) must not drop an open the browser already took, which would leave a blank tab
    /// nobody marked. The task is bounded by its listing's budget and CDP's request timeout.
    async fn ensured(self: &Arc<Self>, adopt: Adopt) -> Ensured {
        let this = Arc::clone(self);
        let task = tokio::spawn(async move {
            let ensured = this.ensure_marked(adopt).await;
            // After the lock: scripts' listings need not wait for the watcher to record it.
            if let Some(handed_back) = &ensured.handed_back
                && let Some(hand_back) = this.hand_back.get()
            {
                hand_back(handed_back.target.clone(), handed_back.url.clone()).await;
            }
            ensured
        });
        task.await.unwrap_or_else(|error| {
            tracing::warn!(%error, "Neo's blank tab: making sure of it failed");
            Ensured::default()
        })
    }

    /// [`NeoTab::ensure`] under the lock.
    #[expect(
        clippy::await_holding_invalid_type,
        reason = "one ensure at a time, from its listing to its mark: two cannot each open a tab, \
                  and no script is shown the tab before it is marked"
    )]
    async fn ensure_marked(&self, adopt: Adopt) -> Ensured {
        let _making = self.making.lock().await;
        let session = self.link.linked().await;
        let root = ProtocolSession::root(session.connection());
        let listed: browser::GetTabsResult =
            match time::timeout(STEP_BUDGET, root.send("Browser.getTabs", json!({}))).await {
                Ok(Ok(listed)) => listed,
                Ok(Err(error)) => {
                    tracing::warn!(%error, "Neo's blank tab: the browser did not list its tabs");
                    return Ensured::default();
                }
                Err(_) => {
                    tracing::warn!("Neo's blank tab: the browser's tab listing timed out");
                    return Ensured::default();
                }
            };
        let mut ensured = Ensured {
            listed: listed.tabs.iter().map(|tab| TabId(tab.tab_id)).collect(),
            ..Ensured::default()
        };
        let mut browsed = None;
        if let Some(held) = self.current()
            && self.tabs.is_neo(held)
            && let Some(info) = listed.tabs.iter().find(|tab| tab.tab_id == held.0)
        {
            if still_blank(&info.url) {
                ensured.tab = Some(held);
                return ensured;
            }
            // The owner browsed in it (in a fresh browser Neo's tab is often the one they see),
            // or a fit navigated it: it goes back to them once a new blank tab is Neo's.
            browsed = Some((
                held,
                HandedBack {
                    target: TargetId(info.target_id.clone()),
                    url: info.url.clone(),
                },
            ));
        }
        let adopted = match adopt {
            Adopt::Allowed => self.adoptable(&listed).await,
            Adopt::Never => None,
        };
        let tab = match adopted {
            Some(tab) => {
                tracing::info!(tab = tab.0, "Neo's blank tab: adopted a blank tab");
                Some(tab)
            }
            None => self.open(&root).await,
        };
        let Some(tab) = tab else {
            // Nothing new to hold: a tab the owner browsed in stays Neo's until one opens, so the
            // browser always keeps a tab of Neo's. The next ensure tries again.
            ensured.tab = browsed.map(|(held, _)| held);
            return ensured;
        };
        self.tabs.make_neo(tab);
        *self.current.lock().unwrap_or_else(PoisonError::into_inner) = Some(tab);
        ensured.tab = Some(tab);
        // Then the old one goes back: never a moment without a tab of Neo's.
        if let Some((held, handed_back)) = browsed {
            tracing::info!(
                tab = held.0,
                "Neo's blank tab: the owner browsed in it; handed back"
            );
            self.tabs.release_neo(held);
            ensured.handed_back = Some(handed_back);
        }
        ensured
    }

    /// Keeps the tab open for the process's life: at the link's first connect and at each new
    /// watch start it is found or made (adopting allowed), when it closes a new one opens, and
    /// every [`RECHECK`] a tab the owner browsed in is handed back. Subscribed before the first
    /// connect, so no event is missed.
    pub(crate) async fn follow(self: Arc<Self>, shutdown: CancellationToken) {
        let mut events = self.link.tab_events();
        tokio::select! {
            () = shutdown.cancelled() => return,
            _ = self.link.linked() => {}
        }
        self.ensure(Adopt::Allowed).await;
        let mut recheck = time::interval_at(time::Instant::now() + RECHECK, RECHECK);
        recheck.set_missed_tick_behavior(time::MissedTickBehavior::Delay);
        loop {
            let event = tokio::select! {
                () = shutdown.cancelled() => return,
                _ = recheck.tick() => {
                    self.ensure(Adopt::Never).await;
                    continue;
                }
                event = events.recv() => event,
            };
            match event {
                Ok(TabEvent::Watching(_)) => {
                    self.ensure(Adopt::Allowed).await;
                }
                Ok(TabEvent::Closed { tab, .. }) if Some(tab) == self.current() => {
                    tracing::info!(tab = tab.0, "Neo's blank tab closed; opening another");
                    self.ensure(Adopt::Never).await;
                }
                Ok(TabEvent::Opened { .. } | TabEvent::Closed { .. }) => {}
                // A close may be among the missed events.
                Err(RecvError::Lagged(_)) => {
                    self.ensure(Adopt::Never).await;
                }
                Err(RecvError::Closed) => return,
            }
        }
    }

    /// A blank tab nobody else owns, lowest tab id first: not the agent's, no task's in memory,
    /// and no task's in the trace either.
    async fn adoptable(&self, listed: &browser::GetTabsResult) -> Option<TabId> {
        let mut blank: Vec<TabId> = listed
            .tabs
            .iter()
            .filter(|tab| BLANK_URLS.contains(&tab.url.as_str()))
            .map(|tab| TabId(tab.tab_id))
            .filter(|tab| self.tabs.adoptable(*tab))
            .collect();
        blank.sort_by_key(|tab| tab.0);
        let tasked = match self.store.tasks_for_tabs(&blank).await {
            Ok(tasked) => tasked,
            Err(error) => {
                // Unknown owners: open a tab of Neo's own rather than take a task's.
                tracing::warn!(%error, "Neo's blank tab: the trace's task links did not read");
                return None;
            }
        };
        blank
            .into_iter()
            .find(|tab| tasked.get(tab).is_none_or(Vec::is_empty))
    }

    /// Opens a blank tab in the background, so the owner's front tab stays where it is. No
    /// budget of Neo's own: the browser may open the tab before a late reply, and only the reply
    /// says which tab is Neo's. CDP's request timeout bounds it.
    async fn open(&self, root: &ProtocolSession) -> Option<TabId> {
        let opened: Result<browser::CreateTabResult, _> = root
            .send(
                "Browser.createTab",
                json!({ "url": "about:blank", "background": true }),
            )
            .await;
        match opened {
            Ok(created) => {
                tracing::info!(tab = created.tab.tab_id, "Neo's blank tab: opened");
                Some(TabId(created.tab.tab_id))
            }
            Err(error) => {
                tracing::warn!(%error, "Neo's blank tab: the browser did not open one");
                None
            }
        }
    }
}
