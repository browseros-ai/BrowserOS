//! The process's one neo-playwright facade.

use std::sync::Arc;

use neo_playwright::{Playwright, PlaywrightHooks};
use tokio::sync::OnceCell;

use crate::browser::BrowserLink;

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
    /// Handed to the facade once, when it is built.
    hooks: Arc<dyn PlaywrightHooks>,
    playwright: OnceCell<Arc<Playwright>>,
}

impl PlaywrightFacade {
    /// The facade over `link`'s session, built in the background on its first connect.
    pub(crate) fn start(link: Arc<BrowserLink>, hooks: Arc<dyn PlaywrightHooks>) -> Arc<Self> {
        let facade = Arc::new(Self {
            link,
            hooks,
            playwright: OnceCell::new(),
        });
        // Waits for a first connect that may never come; nothing waits on this task.
        let early = Arc::clone(&facade);
        tokio::spawn(async move {
            early.get().await;
        });
        facade
    }

    /// The facade, built here if the background start has not finished. An op runs only
    /// inside a run, and a run starts only once the link connected, so this does not wait on
    /// the browser.
    pub(crate) async fn get(&self) -> &Arc<Playwright> {
        self.playwright
            .get_or_init(|| async {
                Playwright::new(self.link.linked().await, Arc::clone(&self.hooks))
            })
            .await
    }
}
