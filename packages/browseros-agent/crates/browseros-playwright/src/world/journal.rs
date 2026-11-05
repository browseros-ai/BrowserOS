//! The seal's journal: values entered into sensitive fields, forwarded to the host's privacy
//! hooks.
//!
//! ```text
//!  page types into a password field ─ seal's input listener (sealed world)
//!    └─ __neoJournal(payload) ─ Runtime.bindingCalled { name, payload } on that frame's session
//!         └─ Journal task: session → page (SessionSetup) → typing window (PageState)
//!              └─ PlaywrightHooks::field_entered(page, entry, TypedBy)
//! ```

use std::sync::Arc;

use browseros_cdp::CdpEvent;
use browseros_core::BrowserSession;
use browseros_core::sensitive_fields::{JOURNAL_BINDING, JournalEntry};
use serde::Deserialize;
use tokio::task::JoinHandle;

use crate::PlaywrightHooks;
use crate::page::PageRegistry;
use crate::world::SessionSetup;

/// The background task that forwards journal entries, for as long as its `Playwright` lives.
///
/// It takes `Runtime.bindingCalled` on the connection's dedicated channel for that method,
/// not the shared event ring: the ring drops events when a reader lags, and a dropped entry is
/// a secret the trace never learns to mask. Every dedicated subscriber of a method gets each of
/// its events, and browseros-takeover's control journal is another: during a takeover it arms
/// the owner's frames with the same binding and reads the same calls. Each journal keeps only
/// the entries from sessions it set up.
#[derive(Debug)]
pub(crate) struct Journal {
    task: JoinHandle<()>,
}

/// The parts of `Runtime.bindingCalled` the journal reads. The payload holds the value, so
/// this never prints.
#[derive(Deserialize)]
struct BindingCalled {
    name: String,
    payload: String,
}

impl Journal {
    /// Starts forwarding. Needs a tokio runtime, as `BrowserSession` itself does.
    pub(crate) fn spawn(
        browser: &BrowserSession,
        sessions: Arc<SessionSetup>,
        pages: Arc<PageRegistry>,
        hooks: Arc<dyn PlaywrightHooks>,
    ) -> Self {
        let mut events = browser.cdp_events_targeted("Runtime.bindingCalled");
        let task = tokio::spawn(async move {
            while let Some(event) = events.recv().await {
                forward(&event, &sessions, &pages, hooks.as_ref());
            }
        });
        Self { task }
    }
}

/// Stops the task with its `Playwright`, so a dropped instance keeps nothing alive.
impl Drop for Journal {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// One binding call. Entries from sessions this instance never set up, or that are not
/// well-formed, are dropped: nothing can attribute them to a page.
fn forward(
    event: &CdpEvent,
    sessions: &SessionSetup,
    pages: &PageRegistry,
    hooks: &dyn PlaywrightHooks,
) {
    let Ok(called) = BindingCalled::deserialize(&event.params) else {
        return;
    };
    if called.name != JOURNAL_BINDING {
        return;
    }
    let Some(page) = event
        .session_id
        .as_ref()
        .and_then(|session| sessions.page_of(session))
    else {
        return;
    };
    let Some(entry) = JournalEntry::parse(&called.payload) else {
        return;
    };
    let typed_by = pages.typed_by(&page);
    hooks.field_entered(page, entry, typed_by);
}
