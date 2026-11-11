//! The browser link's tab events, told to the host API: the one adapter between the two.

use std::sync::Arc;

use browseros_core::TabId;
use neo_service::Service;
use neo_trace::EpochMillis;
use tokio::sync::broadcast;
use tokio::sync::broadcast::error::RecvError;

use crate::browser::{BrowserLink, TabEvent};

/// Drops what the service keeps about every tab that is not open now: tabs that closed while
/// no socket was watched, flags an earlier Neo left on disk, or every tab of a browser that
/// restarted, whose old tab numbers can come back. Returns the watch start its listing covers.
///
/// `Service::retain_tabs` ends the state of every tab its listing lacks, unless a host changed
/// the tab after the fence. So the fence comes first and the listing here and now, never one
/// that waited in a queue, which could lack a tab a host locked or made private before the
/// fence. The listing is taken under `tab_watch`'s current start, so every tab that closes after
/// it is reported as a [`TabEvent::Closed`].
pub(crate) async fn reconcile(service: &Service, link: &BrowserLink) -> u64 {
    let fence = service.tab_fence();
    let listing = link.watched_listing().await;
    // retain_tabs keeps a tab a host changed after the fence, though this listing may lack it.
    service.retain_tabs(&listing.open, fence).await;
    listing.watched.generation
}

/// Feeds the host API every change `tab_watch` reports, in order, until the link goes away
/// (never, in a running Neo): `tab_closed` for each close and `tab_opened` for each open, and
/// [`reconcile`] when the watch starts afresh or this feed falls behind it. `reconciled` is the
/// watch start the last reconciliation covered.
///
/// Each reconcile runs inline, from its fence to its `retain_tabs`, so no `tab_opened` lands in
/// between. `tab_opened` takes no stamp and relies on that. A reconcile run beside this loop
/// could end the flag of a tab that opened private after its listing.
///
/// The parts behind the service key their state by tab: a closed tab's locks end and its
/// private flag is forgotten, a host hears `tab.closed`, and a tab that opens while a control
/// display is open starts private. `tab_watch` sees every tab, so this covers tabs no op ever
/// touched, such as a tab the owner locked or made private on their own.
pub(crate) async fn tab_feed(
    service: Service,
    link: Arc<BrowserLink>,
    mut events: broadcast::Receiver<TabEvent>,
    mut reconciled: u64,
) {
    loop {
        match events.recv().await {
            // A change seen on a socket since replaced is the next reconciliation's: the
            // browser may have restarted, and the tab id may already name another tab.
            Ok(TabEvent::Closed { epoch, tab }) => {
                if link.epoch() == Some(epoch) {
                    service.tab_closed(tab).await;
                }
            }
            Ok(TabEvent::Opened { epoch, tab }) => {
                if link.epoch() == Some(epoch) {
                    opened(&service, tab).await;
                }
            }
            Ok(TabEvent::Watching(watched)) => {
                if watched.generation > reconciled {
                    reconciled = reconcile(&service, &link).await;
                }
            }
            Err(RecvError::Lagged(missed)) => {
                tracing::warn!(
                    missed,
                    "the tab feed fell behind tab_watch; reconciling the tab state"
                );
                reconciled = reconcile(&service, &link).await;
            }
            Err(RecvError::Closed) => return,
        }
    }
}

async fn opened(service: &Service, tab: TabId) {
    if let Err(error) = service.tab_opened(tab, EpochMillis::now()).await {
        // The flag still holds in memory, so captures stay refused until Neo restarts.
        tracing::warn!(%error, tab = tab.0, "a tab that opened under a control display is private, but not on disk");
    }
}
