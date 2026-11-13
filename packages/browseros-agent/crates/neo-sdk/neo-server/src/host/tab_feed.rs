//! The browser link's tab events, told to the host API: the one adapter between the two.

use std::sync::Arc;

use neo_http::Service;
use tokio::sync::broadcast;
use tokio::sync::broadcast::error::RecvError;

use crate::browser::{BrowserLink, TabEvent};

/// Drops what the service keeps about every tab that is not open now: tabs that closed while
/// no socket was watched, or every tab of a browser that restarted, whose old tab numbers can
/// come back. Returns the watch start its listing covers.
///
/// `Service::retain_tabs` ends the state of every tab its listing lacks, unless a host changed
/// the tab after the fence. So the fence comes first and the listing here and now, never one
/// that waited in a queue, which could lack a tab a host locked before the fence. The listing
/// is taken under `tab_watch`'s current start, so every tab that closes after it is reported as a
/// [`TabEvent::Closed`].
pub(crate) async fn reconcile(service: &Service, link: &BrowserLink) -> u64 {
    let fence = service.tab_fence();
    let listing = link.watched_listing().await;
    // retain_tabs keeps a tab a host changed after the fence, though this listing may lack it.
    service.retain_tabs(&listing.open, fence).await;
    listing.watched.generation
}

/// Feeds closes to the host API in order so locks end even for tabs no agent op touched.
/// Reconcile when the watch restarts or the feed falls behind; `reconciled` identifies the
/// watch generation the last listing covered. Opens need no host bookkeeping.
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
            Ok(TabEvent::Opened { .. }) => {}
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
