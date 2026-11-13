//! The host's claims on tabs, as the idle-tab janitor asks them.

use std::sync::Arc;

use browseros_core::TabId;
use neo_takeover::TabHold;
use tokio::sync::SetOnce;

/// Where a tab's claims are read: the host API's `Service::tab_hold` once it starts.
type HoldSource = Arc<dyn Fn(TabId) -> Option<TabHold> + Send + Sync>;

/// Whether the host holds a tab with a lock or a fit, now or until when (the settings design,
/// section 2, "What inactivity means"). The janitor never closes a held tab, and restarts its
/// idle clock when the last claim lets go: a lock keeps the agent off a tab the owner is
/// using, and a fit is how a host shows the tab to the owner, without a lock in zuzu's
/// handoff.
///
/// The locks and fits belong to neo-service, which starts only once the browser link's first
/// connect lands. Until the host API arms this, the host holds nothing, which is exact: a lock or
/// a fit is taken only through the host API's routes, served only after arming. `default()` is
/// unarmed, as it stays in the Node-hosted embedding, which has no host API.
#[derive(Default)]
pub(crate) struct HostHolds {
    source: SetOnce<HoldSource>,
}

impl HostHolds {
    /// The host API started: from now on `source` answers. Called once per process; a second
    /// call changes nothing. Tests arm it with a closure over their own locks and fits.
    pub(crate) fn arm(&self, source: impl Fn(TabId) -> Option<TabHold> + Send + Sync + 'static) {
        let _ = self.source.set(Arc::new(source));
    }

    /// The host's claim on `tab`, or `None` when it has none.
    #[must_use]
    pub(crate) fn hold(&self, tab: TabId) -> Option<TabHold> {
        self.source.get().and_then(|source| source(tab))
    }
}
