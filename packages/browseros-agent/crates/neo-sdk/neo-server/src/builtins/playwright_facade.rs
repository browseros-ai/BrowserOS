//! The process's one neo-playwright facade.

use std::sync::{Arc, Weak};
use std::time::Duration;

use browseros_core::{TabId, TargetId};

use futures_util::future::BoxFuture;
use neo_http::PictureVerdict;
use neo_playwright::{EventFence, FileAccess, PictureCheck, Playwright};
use tokio::sync::{OnceCell, SetOnce};
use tokio::time;

use crate::browser::{BrowserLink, NeoTab};
use crate::dispatch::{EventLog, WorkerFact};
use crate::host::{AgentFront, FilledTabs, LiveTab};

/// neo-playwright's `Playwright` for this process. The crate wants one per browser
/// connection, and Neo has exactly one: the link's session, which exists from the first
/// connect on and follows every reconnect.
///
/// [`PlaywrightFacade::start`] builds it as soon as that connect lands, so its page event log
/// (popups, dialogs, downloads, closes) runs ahead of any op, and an op's `waitForEvent` cannot
/// miss an event that came between the script's call and the first op of the process. An op
/// that comes first builds it itself, through the same cell, so there is still exactly one.
pub(crate) struct PlaywrightFacade {
    link: Arc<BrowserLink>,
    /// Frozen startup mode, passed through each op's environment. Only hosted BrowserOS
    /// suppresses console/page-error events; ordinary desktop connections keep their waits.
    hosted: bool,
    /// Brings an agent tab forward before its input or capture (`OpEnv::bring_forward`).
    front: Arc<AgentFront>,
    /// Required by the SDK constructor so neither startup mode can omit filesystem policy.
    file_access: Arc<FileAccess>,
    playwright: OnceCell<Arc<Playwright>>,
    /// Neo's blank tab, made sure of before every close; unset in the Node-hosted embedding,
    /// whose host launched the browser itself.
    blank_tab: SetOnce<Arc<NeoTab>>,
    /// The tabs that took a host's private fill before a restart, read back into the entry's own
    /// set when the facade is built, so no op runs before it holds them (ruling R45). `None` in
    /// the embedding, which serves no host API and so no fill.
    filled: Option<Arc<FilledTabs>>,
}

impl PlaywrightFacade {
    /// The facade over `link`'s session, built in the background on its first connect.
    pub(crate) fn start(
        link: Arc<BrowserLink>,
        hosted: bool,
        front: Arc<AgentFront>,
        file_access: Arc<FileAccess>,
        filled: Option<Arc<FilledTabs>>,
    ) -> Arc<Self> {
        let facade = Arc::new(Self {
            link,
            hosted,
            front,
            file_access,
            playwright: OnceCell::new(),
            blank_tab: SetOnce::new(),
            filled,
        });
        // Waits for a first connect that may never come; nothing waits on this task.
        let early = Arc::clone(&facade);
        tokio::spawn(async move {
            early.get().await;
        });
        facade
    }

    /// The browser's configured startup mode.
    /// From now on every close first makes sure of Neo's blank tab, every op's listings and
    /// page events skip it (`OpCtxEnv`), and a tab Neo hands back reaches running scripts as a
    /// new page. Called once by the sidecar; a second call changes nothing.
    pub(crate) fn keep(self: &Arc<Self>, blank_tab: Arc<NeoTab>) {
        // Weak: the facade holds the blank tab, which must not keep the facade alive.
        let facade = Arc::downgrade(self);
        blank_tab.on_hand_back(Box::new(move |target, url| {
            let facade = Weak::clone(&facade);
            Box::pin(async move {
                // A Playwright not started yet has no run to tell.
                let playwright = facade
                    .upgrade()
                    .and_then(|facade| facade.playwright.get().cloned());
                if let Some(playwright) = playwright {
                    playwright.hand_back(target, url).await;
                }
            })
        }));
        let _ = self.blank_tab.set(blank_tab);
    }

    /// Neo's blank tab, when the sidecar keeps one.
    pub(crate) fn blank_tab(&self) -> Option<&Arc<NeoTab>> {
        self.blank_tab.get()
    }

    /// Makes sure Neo's blank tab is open before `closing` closes, without adopting another tab
    /// (a script may be about to use it). Whether the close may go ahead
    /// (`NeoTab::keep_before_close`); always, when the sidecar keeps no blank tab.
    pub(crate) async fn keep_blank_tab(&self, closing: TabId) -> bool {
        match self.blank_tab.get() {
            Some(blank_tab) => blank_tab.keep_before_close(closing).await,
            None => true,
        }
    }

    pub(crate) fn hosted_mode(&self) -> bool {
        self.hosted
    }

    /// What brings an op's tab forward before its input or capture.
    pub(crate) fn front(&self) -> &Arc<AgentFront> {
        &self.front
    }

    /// The shared file policy every operation must carry to the library's IO gateways.
    pub(crate) fn file_access(&self) -> &Arc<FileAccess> {
        &self.file_access
    }

    /// Whether `tab` took a host's private typing, for the camera, which must not wait: `false`
    /// before the facade is built, when no entry can have run.
    pub(crate) fn took_private_text(&self, tab: TabId) -> bool {
        self.playwright
            .get()
            .is_some_and(|playwright| playwright.took_private_text(tab))
    }

    /// What `tab` shows now, for the camera (ruling R45): whether any frame holds a box masking
    /// treats as private, not empty and painted in clear. Private before the facade is built
    /// and whenever a frame cannot be read, so a picture is never stored on a guess.
    pub(crate) async fn picture_verdict(&self, tab: TabId) -> PictureVerdict {
        let Some(playwright) = self.playwright.get() else {
            return PictureVerdict::Private;
        };
        match playwright.picture_check(tab).await {
            PictureCheck::Clear { reading } => PictureVerdict::Clear { reading },
            PictureCheck::Private | PictureCheck::Unknown => PictureVerdict::Private,
        }
    }

    /// The facade, built here if the background start has not finished. An op runs only
    /// inside a run, and a run starts only once the link connected, so this does not wait on
    /// the browser.
    pub(crate) async fn get(&self) -> &Arc<Playwright> {
        self.playwright
            .get_or_init(|| async {
                let playwright = Playwright::new(self.link.linked().await);
                if let Some(filled) = &self.filled {
                    remember_filled(filled, &self.link, &playwright).await;
                }
                playwright
            })
            .await
    }
}

/// The tabs the record names that the browser still has, marked in the entry's own set before
/// the facade serves its first op: Neo's camera and a script's screenshot then refuse them as
/// before the restart. The browser's own tab list is read unfiltered, so a tab on an extension
/// page is not dropped. A listing that keeps failing fails closed.
async fn remember_filled(filled: &FilledTabs, link: &BrowserLink, playwright: &Playwright) {
    if filled.is_empty() {
        return;
    }
    for _attempt in 0..LISTING_ATTEMPTS {
        if let Some(tabs) = link.every_tab().await {
            let live: Vec<LiveTab> = tabs
                .into_iter()
                .map(|(target, tab)| LiveTab { target, tab })
                .collect();
            for tab in filled.live(&live).await {
                playwright.keep_private_tab(tab);
            }
            return;
        }
        time::sleep(LISTING_RETRY).await;
    }
    // Fail closed: which of the browser's tabs the record names cannot be told, so every tab is
    // treated as having taken a fill for this process's life.
    tracing::error!(
        "cannot list the browser's tabs at start; every tab is treated as having taken a private fill"
    );
    playwright.keep_every_tab();
}

/// How often the start asks the browser for its tabs before it fails closed, and how long it
/// waits between asks.
const LISTING_ATTEMPTS: u32 = 3;
const LISTING_RETRY: Duration = Duration::from_millis(500);

impl EventLog for PlaywrightFacade {
    fn fence(&self) -> EventFence {
        self.playwright
            .get()
            .map_or_else(EventFence::default, |pw| pw.event_fence())
    }
    fn synchronize(&self, fence: EventFence) -> BoxFuture<'_, bool> {
        Box::pin(async move {
            match self.playwright.get() {
                Some(pw) => pw.synchronize_events(fence).await,
                // No facade yet: no watcher, so no event it could still owe.
                None => true,
            }
        })
    }
    fn handled_through(&self, fence: EventFence) -> bool {
        self.playwright
            .get()
            .is_none_or(|pw| pw.events_handled_through(fence))
    }
    fn workers(&self, target: &TargetId) -> Option<Vec<WorkerFact>> {
        self.playwright.get().map(|pw| {
            pw.workers(target)
                .into_iter()
                .map(|worker| WorkerFact {
                    id: worker.id,
                    url: worker.url,
                })
                .collect()
        })
    }
    fn head(&self) -> u64 {
        self.playwright.get().map_or(0, |pw| pw.event_head())
    }

    fn closed(&self, target: &TargetId) -> Option<u64> {
        self.playwright
            .get()
            .and_then(|pw| pw.closed_target(target))
    }

    fn navigated(&self, target: &TargetId) -> Option<u64> {
        self.playwright
            .get()
            .and_then(|pw| pw.navigated_target(target))
    }

    fn frame_versions(&self) -> Vec<(TabId, u64)> {
        self.playwright
            .get()
            .map_or_else(Vec::new, |pw| pw.frame_versions())
    }
}
