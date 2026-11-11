//! The process's one neo-playwright facade.

use std::sync::Arc;

use browseros_core::sensitive_fields::{FieldValues, JournalEntry};
use browseros_core::{TabId, TargetId};

use neo_playwright::{Playwright, PlaywrightHooks, Refusal, TypedBy};
use tokio::sync::OnceCell;

use crate::browser::BrowserLink;
use crate::dispatch::EventLog;

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
    playwright: OnceCell<Arc<Playwright>>,
}

impl PlaywrightFacade {
    /// The facade over `link`'s session, built in the background on its first connect.
    pub(crate) fn start(link: Arc<BrowserLink>) -> Arc<Self> {
        let facade = Arc::new(Self {
            link,
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
                Playwright::new(self.link.linked().await, Arc::new(NoopHooks))
            })
            .await
    }
}

impl EventLog for PlaywrightFacade {
    fn head(&self) -> u64 {
        self.playwright.get().map_or(0, |pw| pw.event_head())
    }

    fn closed(&self, target: &TargetId) -> Option<u64> {
        self.playwright
            .get()
            .and_then(|pw| pw.closed_target(target))
    }
}

/// Consumes the engine's field notifications without retaining values or tab state. The
/// facade owns this stateless sink because the current engine contract requires callbacks.
struct NoopHooks;

impl PlaywrightHooks for NoopHooks {
    fn field_entered(&self, _tab: TabId, _entry: JournalEntry, _by: TypedBy) {}

    fn values_swept(&self, _tab: TabId, _values: &FieldValues) {}

    fn capture_allowed(&self, _tab: TabId) -> Result<(), Refusal> {
        Ok(())
    }
}
