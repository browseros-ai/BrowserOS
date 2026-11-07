//! Neo's link to Chromium, and what Neo remembers about its tabs.
//!
//! ```text
//!  BrowserLink ── connect loop: the first connect retried 1 s → 30 s, then the CDP client
//!     │           reconnects by itself (KeepTrying, no retry limit)
//!     │  on the first success, once: the BrowserSession (sensitive values masked, isolated
//!     │  world on), core's detach hook ─▶ TabRegistry::closed, and the tab_watch task
//!     ├─ view() ─▶ RunView: an unbound BrowserView with the viewports, and its CDP epoch
//!     ├─ status() / is_ready() / is_current(epoch) ─▶ health, readiness, the door's gates
//!     ├─ lists_tab(epoch, tab) ─▶ whether core knew a tab at tab_watch's latest listing, with
//!     │  no await: the door asks it for each other tab a main-world script can reach
//!     └─ linked() / viewports() / tab_events() / watched_listing() ─▶ the host API, once the
//!        first connect lands
//!  TabRegistry ── per-tab facts: origin, idle clock, feature slots, each task's last tab
//!  tab_watch ── per epoch: Target.setDiscoverTargets, then each page target's tab, a listing
//!               after every burst of target events, and a TabEvent for each tab that opened
//!               or closed, in order; the listing fires core's detach hook promptly, and is
//!               kept whole for lists_tab
//! ```
//!
//! Views are unbound. On BrowserOS 0.51.1.3 and later a bound capture costs 4 to 6 CDP round
//! trips per run and hides other profiles' tabs, and hosted Neo has one profile. An unbound view
//! silently follows a reconnect, so each run keeps the epoch its view was taken at and the
//! dispatcher refuses once it moves. Handles carry the tab id beside core's page id
//! ([`PageRef`]), because core's page ids restart at 1 with Neo and tab ids do not.

mod browser_link;
mod page_ref;
mod tab_registry;
mod tab_watch;
#[cfg(test)]
mod tests;

pub(crate) use browser_link::{BrowserLink, LinkStatus, RunView};
pub(crate) use page_ref::PageRef;
pub(crate) use tab_registry::TabRegistry;
pub(crate) use tab_watch::{TabEvent, TabListing, Watched};
