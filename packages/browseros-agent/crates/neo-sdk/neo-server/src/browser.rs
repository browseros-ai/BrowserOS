//! Neo's link to Chromium, and what Neo remembers about its tabs.
//!
//! ```text
//!  BrowserLink ── connect loop: the first connect retried 1 s → 30 s, then the CDP client
//!     │           reconnects by itself (KeepTrying, no retry limit)
//!     │  on the first success, once: the BrowserSession (isolated
//!     │  world on), core's detach hook ─▶ TabRegistry::closed, and the tab_watch task
//!     ├─ view() ─▶ RunView: an unbound BrowserView with the viewports, and its CDP epoch
//!     ├─ status() / is_ready() / is_current(epoch) ─▶ health, readiness, the door's gates
//!     └─ linked() / viewports() / tab_events() / watched_listing() ─▶ the host API, once the
//!        first connect lands
//!  TabRegistry ── per-tab facts: origin, idle clock, feature slots, each host task's last tab
//!     ├─ TaskBook: named tasks, their families and ordered tab holders, retired by the same
//!     │           close hook; each change goes out as a BookChange, a restart comes back in
//!     │           as SavedTasks
//!     └─ sight(RunScope, tab) ─▶ Sight: whether a run, bound to a task family or not, reaches
//!        a tab: its family holds it, or it is a popup of one that no task holds yet
//!  tab_watch ── per epoch: Target.setDiscoverTargets, then each page target's tab, a listing
//!               after every burst of target events, and a TabEvent for each tab that opened
//!               or closed, in order; the listing fires core's detach hook promptly; a popup's
//!               opener is noted at its announcement, before any listing
//!  NeoTab ── Neo's one blank tab (TabOrigin::Neo), kept open on every watch start and after
//!            each close of it, so no other close ends the browser (the sidecar only)
//! ```
//!
//! Views are unbound. On BrowserOS 0.51.1.3 and later a bound capture costs 4 to 6 CDP round
//! trips per run and hides other profiles' tabs, and hosted Neo has one profile. An unbound view
//! silently follows a reconnect, so each run keeps the epoch its view was taken at and the
//! dispatcher refuses once it moves. Handles carry only the browser tab id; no handle outlives
//! its run, and the run cannot cross a connection epoch.

mod book_change;
mod browser_link;
mod neo_tab;
mod page_ref;
mod run_scope;
mod tab_registry;
mod tab_watch;
mod task_book;
#[cfg(test)]
mod tests;

pub(crate) use book_change::{BookChange, BookSink, SavedTask};
pub(crate) use browser_link::{BrowserLink, LinkStatus, RunView};
#[cfg(test)]
pub(crate) use neo_tab::Adopt;
pub(crate) use neo_tab::NeoTab;
pub(crate) use page_ref::PageRef;
pub(crate) use run_scope::{RunScope, Sight, TabsScope};
pub(crate) use tab_registry::{Seen, TabRegistry, TaskPage};
pub(crate) use tab_watch::{TabEvent, TabListing, Watched};
pub(crate) use task_book::{Holder, TaskError, TaskOrigin, TaskRecord};
