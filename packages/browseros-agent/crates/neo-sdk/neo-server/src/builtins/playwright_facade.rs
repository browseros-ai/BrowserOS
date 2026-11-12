//! The process's one neo-playwright facade.

use std::sync::Arc;

use browseros_core::TargetId;

use futures_util::future::BoxFuture;
use neo_playwright::Playwright;
use tokio::sync::OnceCell;

use crate::browser::BrowserLink;
use crate::dispatch::{EventLog, WorkerFact};
use crate::host::AgentFront;

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
    playwright: OnceCell<Arc<Playwright>>,
}

impl PlaywrightFacade {
    /// The facade over `link`'s session, built in the background on its first connect.
    pub(crate) fn start(link: Arc<BrowserLink>, hosted: bool, front: Arc<AgentFront>) -> Arc<Self> {
        let facade = Arc::new(Self {
            link,
            hosted,
            front,
            playwright: OnceCell::new(),
        });
        // Waits for a first connect that may never come; nothing waits on this task.
        let early = Arc::clone(&facade);
        tokio::spawn(async move {
            early.get().await;
        });
        facade
    }

    /// The browser's configured startup mode.
    pub(crate) fn hosted_mode(&self) -> bool {
        self.hosted
    }

    /// What brings an op's tab forward before its input or capture.
    pub(crate) fn front(&self) -> &Arc<AgentFront> {
        &self.front
    }

    /// The facade, built here if the background start has not finished. An op runs only
    /// inside a run, and a run starts only once the link connected, so this does not wait on
    /// the browser.
    pub(crate) async fn get(&self) -> &Arc<Playwright> {
        self.playwright
            .get_or_init(|| async { Playwright::new(self.link.linked().await) })
            .await
    }
}

impl EventLog for PlaywrightFacade {
    fn synchronize(&self) -> BoxFuture<'_, ()> {
        Box::pin(async {
            if let Some(pw) = self.playwright.get() {
                pw.synchronize_events().await;
            }
        })
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
}
