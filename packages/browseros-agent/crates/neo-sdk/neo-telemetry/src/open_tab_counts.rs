//! The live open-tab counts the `neo_tab_count` gauge reads at each scrape or push.

use std::sync::Arc;

use crate::label::TabOrigin;

/// How many tabs are open right now, by origin. Read from the tab registry's own open map at
/// each collection, never kept as a running sum, so tabs that were open before Neo started and
/// a browser reconnect can never make the gauge drift or go negative.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OpenTabCounts {
    /// Tabs an agent's script opened.
    pub agent: u64,
    /// Tabs Neo found open or someone else opened.
    pub found: u64,
    /// Neo's own tabs.
    pub neo: u64,
}

impl OpenTabCounts {
    /// The count for one `neo.tab.origin` value.
    #[must_use]
    pub const fn of(&self, origin: TabOrigin) -> u64 {
        match origin {
            TabOrigin::Agent => self.agent,
            TabOrigin::Found => self.found,
            TabOrigin::Neo => self.neo,
        }
    }
}

/// Where the gauge reads the counts: a callback the tab registry hands over once
/// ([`crate::Telemetry::set_open_tabs`]). It runs on the collecting thread (a scrape's handler,
/// the periodic exporter's thread), so it must be quick and must never wait on async work: a
/// std lock held for a map read is fine.
pub type OpenTabsSource = Arc<dyn Fn() -> OpenTabCounts + Send + Sync>;
