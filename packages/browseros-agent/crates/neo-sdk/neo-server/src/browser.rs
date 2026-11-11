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
//!     └─ TaskBook: named tasks and ordered tab ownership, retired by the same close hook
//!  tab_watch ── per epoch: Target.setDiscoverTargets, then each page target's tab, a listing
//!               after every burst of target events, and a TabEvent for each tab that opened
//!               or closed, in order; the listing fires core's detach hook promptly
//! ```
//!
//! Views are unbound. On BrowserOS 0.51.1.3 and later a bound capture costs 4 to 6 CDP round
//! trips per run and hides other profiles' tabs, and hosted Neo has one profile. An unbound view
//! silently follows a reconnect, so each run keeps the epoch its view was taken at and the
//! dispatcher refuses once it moves. Handles carry only the browser tab id; no handle outlives
//! its run, and the run cannot cross a connection epoch.

mod browser_link;
mod page_ref;
mod tab_registry;
mod tab_watch;
mod task_book;
#[cfg(test)]
mod tests;

pub(crate) use browser_link::{BrowserLink, LinkStatus, RunView};
pub(crate) use page_ref::PageRef;
pub(crate) use tab_registry::{Seen, TabRegistry};
pub(crate) use tab_watch::{TabEvent, TabListing, Watched};
pub(crate) use task_book::{TaskError, TaskRecord};
