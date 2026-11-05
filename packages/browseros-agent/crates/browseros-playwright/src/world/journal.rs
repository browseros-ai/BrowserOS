//! The seal's journal: values entered into sensitive fields, forwarded to the host's privacy
//! hooks, and the seal's reports of closed shadow roots that no scan found yet.
//!
//! ```text
//!  page types into a password field ─ seal's input listener (sealed world)
//!    └─ __neoJournal(payload) ─ Runtime.bindingCalled { name, payload, executionContextId }
//!         │                     on that frame's session
//!         └─ Journal task: session → page (SessionSetup) → typing window (PageState)
//!              └─ PlaywrightHooks::field_entered(page, entry, TypedBy)
//!  page types into a closed root no one watches ─ the input reaches the seal at its host
//!    └─ __neoJournal(HIDDEN_ROOT_REPORT) ─ the same binding call
//!         └─ Journal task: session and context → the page's world (FrameWorlds::by_context)
//!              └─ a sweep beside the loop, within FRAME_BUDGET (evaluate_guard::sweep_reported)
//!                   └─ PlaywrightHooks::values_swept(page, values)
//! ```

use std::sync::Arc;

use browseros_cdp::CdpEvent;
use browseros_core::sensitive_fields::{HIDDEN_ROOT_REPORT, JOURNAL_BINDING, JournalEntry};
use browseros_core::{BrowserSession, PageId};
use serde::Deserialize;
use tokio::task::{JoinHandle, JoinSet};
use tokio::time;

use crate::PlaywrightHooks;
use crate::page::{self, FRAME_BUDGET, PageRegistry};
use crate::world::{ContextId, SealedWorld, SessionSetup};

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
#[serde(rename_all = "camelCase")]
struct BindingCalled {
    name: String,
    payload: String,
    #[serde(default)]
    execution_context_id: Option<i64>,
}

/// A world whose seal reported a hidden root, to be swept beside the journal's loop.
struct Reported {
    page: PageId,
    world: SealedWorld,
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
            // A report's sweep makes CDP calls, so it runs beside the loop and entries keep
            // flowing meanwhile. The set goes with the task, and its sweeps with it.
            let mut sweeps = JoinSet::new();
            loop {
                tokio::select! {
                    event = events.recv() => {
                        let Some(event) = event else { break };
                        if let Some(reported) = forward(&event, &sessions, &pages, hooks.as_ref()) {
                            sweeps.spawn(sweep(reported, Arc::clone(&hooks)));
                        }
                    }
                    Some(_done) = sweeps.join_next(), if !sweeps.is_empty() => {}
                }
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

/// One binding call: an entry goes to the host now, and a hidden-root report comes back as the
/// world to sweep, unless a sweep of that world is queued already. Calls from sessions this
/// instance never set up, of a world it never installed, or that are not well-formed, are
/// dropped: nothing can attribute them to a page.
fn forward(
    event: &CdpEvent,
    sessions: &SessionSetup,
    pages: &PageRegistry,
    hooks: &dyn PlaywrightHooks,
) -> Option<Reported> {
    let called = BindingCalled::deserialize(&event.params).ok()?;
    if called.name != JOURNAL_BINDING {
        return None;
    }
    let session = event.session_id.as_ref()?;
    let page = sessions.page_of(session)?;
    if called.payload == HIDDEN_ROOT_REPORT {
        let context = ContextId(called.execution_context_id?);
        let world = pages.get(&page)?.frames().by_context(session, context)?;
        return world.queue_report().then_some(Reported { page, world });
    }
    let entry = JournalEntry::parse(&called.payload)?;
    let typed_by = pages.typed_by(&page);
    hooks.field_entered(page, entry, typed_by);
    None
}

/// The sweep a hidden-root report asked for; what it finds goes to the host. A sweep that fails
/// or misses its budget tells the host nothing: the next op's sweep takes the reported host
/// itself, so the op's own result is still masked.
async fn sweep(reported: Reported, hooks: Arc<dyn PlaywrightHooks>) {
    let Reported { page, world } = reported;
    match time::timeout(FRAME_BUDGET, page::sweep_reported(&world)).await {
        Ok(Ok(values)) if !values.is_empty() => hooks.values_swept(page, &values),
        Ok(_) => {}
        // It may have given up waiting for another watch, before its own could unqueue it.
        Err(_late) => world.unqueue_report(),
    }
}
