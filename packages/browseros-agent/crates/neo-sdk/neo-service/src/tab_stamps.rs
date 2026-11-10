//! The fence between a reconcile's tab listing and the host's own changes to tabs.
//!
//! When the browser link (re)connects, the app takes a [`TabFence`], lists the open tabs, and
//! calls [`Service::retain_tabs`](crate::Service::retain_tabs), which ends what the parts keep
//! about every tab the listing lacks. A host may lock, make private, fit or take over a tab that
//! opened after the listing. Its request comes over the host socket, so the app cannot order it
//! against its own listing. Without the fence, the reconcile would end that state as if the tab
//! had closed.
//!
//! ```text
//!  app    tab_fence ─▶ listing (no tab 77) ───────────────────▶ retain_tabs(listing, fence)
//!  host                   stamp 77 ─▶ lock 77, make it private      keeps 77 as if listed
//! ```
//!
//! The routes that create state keyed by a tab (a lock, a private flag, a fit, a takeover's
//! start) stamp the tab with the next value of one counter before they act, and a fence is the
//! counter's value when read. `retain_tabs` keeps every tab stamped after its fence as if it
//! were open. The stamp comes before the change, so every change a reconcile can see already
//! has its stamp in the table.
//!
//! A stamp from before the fence is safe to drop. The host named the tab before the listing
//! began, so the tab was open then, and the listing has it unless it closed since. A closed
//! tab's state should end. So a reconcile drops every stamp older than its fence on a tab its
//! listing lacks, and `tab_closed` drops a closed tab's stamp. The table holds the stamps of open
//! tabs a host changed, and those newer than the last fence.
//!
//! The parts take the kept tabs as a list and scan their state later, some after an await. A
//! stamp that landed in between would miss the list, and a part would end its change. So no
//! stamp lands while the parts act on a list. A [`Kept`] holds the table until it drops, and a
//! route that stamps meanwhile waits for it. A route stamps before it takes anything a part
//! holds, so the wait cannot deadlock.
//!
//! A route can wait there as long as the parts take, and its tab can close meanwhile. So a route
//! and a tab's close go in this order:
//!
//! ```text
//!  a route   stamp (may wait) ─▶ tab open? ─▶ change ─▶ tab still open? if not: undo, 404
//!  a close   forget the stamp (may wait) ─▶ end the tab's state
//! ```
//!
//! A route checks its tab after its stamp, since a check from before the wait says nothing
//! about the tab once the route acts. The tab can still close between the check and the change,
//! and its close can end the tab's state before the change lands. That change would outlive its
//! tab: a lock until its TTL, and a private flag, kept on disk, that a reused tab id could
//! inherit after a browser restart. So the lock and private routes check again after the change,
//! and undo it if the browser answers without the tab. A browser that does not answer says
//! nothing about the tab, and the change stands. A close that comes after that check finds the
//! change and ends it. A close forgets the stamp before it ends anything, so it too waits out a
//! reconcile's parts before it touches a tab's state.
//!
//! A control display's first input makes every open tab private inside `Displays`, and no route
//! knows those tabs before it acts. So `Displays` stamps them itself, through the
//! [`TaintStamps`] hook this table implements, after it lists them and before it commits their
//! flags. Unlike a route, the input does not check its tabs again after the change. A tab that
//! closes between the input's listing and its commit can keep its flag until the next reconcile,
//! and the stamp's wait on a running reconcile widens that window. Such a flag over-protects a
//! later tab with that id and never exposes one.
//!
//! A tab that opens while a control display is open starts private in
//! [`Service::tab_opened`](crate::Service::tab_opened), with no stamp. The app's tab feed calls
//! it in the same loop that runs each reconcile, from its fence to its `retain_tabs`, so it never
//! lands in between.

use browseros_core::TabId;
use futures_util::future::BoxFuture;
use neo_takeover::TaintStamps;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::{Mutex, MutexGuard};

/// Where a reconcile's listing begins among the host's changes to tabs. Take it with
/// [`Service::tab_fence`](crate::Service::tab_fence) before listing the tabs, and hand it to
/// [`Service::retain_tabs`](crate::Service::retain_tabs) with that listing.
#[derive(Clone, Copy, Debug)]
pub struct TabFence(u64);

/// The stamps of the host's changes to tabs, and the counter that numbers them.
#[derive(Default)]
pub(crate) struct TabStamps {
    /// The last stamp given out. A fence reads it without the table's lock, so taking a fence
    /// never waits on a reconcile.
    clock: AtomicU64,
    /// Each tab's newest stamp. An async mutex, because a [`Kept`] holds it across the parts'
    /// awaits.
    stamped: Mutex<HashMap<TabId, u64>>,
}

/// The tabs a reconcile keeps: the ones it listed, and every tab a host changed after its
/// fence. No tab can be stamped while it lives.
#[must_use = "routes stamp again once it drops, so keep it until the last part has acted"]
pub(crate) struct Kept<'a> {
    pub(crate) tabs: Vec<TabId>,
    _stamps: MutexGuard<'a, HashMap<TabId, u64>>,
}

impl TabStamps {
    /// The fence for a listing about to be taken.
    pub(crate) fn fence(&self) -> TabFence {
        TabFence(self.clock.load(Ordering::SeqCst))
    }

    /// Stamps `tab` newer than every fence taken so far. A route, or a control display's first
    /// input, calls it before it creates the tab's state, and waits while a reconcile's [`Kept`]
    /// lives.
    pub(crate) async fn stamp(&self, tab: TabId) {
        let mut stamped = self.stamped.lock().await;
        let stamp = self.clock.fetch_add(1, Ordering::SeqCst) + 1;
        stamped.insert(tab, stamp);
    }

    /// Drops the stamp of a tab that closed.
    pub(crate) async fn forget(&self, tab: TabId) {
        self.stamped.lock().await.remove(&tab);
    }

    /// The tabs a reconcile whose listing is `open` keeps. Drops every stamp older than `fence`
    /// on a tab `open` lacks, since that tab closed after the host named it.
    pub(crate) async fn kept(&self, open: &[TabId], fence: TabFence) -> Kept<'_> {
        let mut stamped = self.stamped.lock().await;
        stamped.retain(|tab, stamp| *stamp > fence.0 || open.contains(tab));
        let mut tabs = open.to_vec();
        tabs.extend(
            stamped
                .iter()
                .filter(|(tab, stamp)| **stamp > fence.0 && !open.contains(tab))
                .map(|(tab, _)| *tab),
        );
        Kept {
            tabs,
            _stamps: stamped,
        }
    }
}

/// A control display's first input stamps each tab it is about to make private, as a route stamps
/// the tab it is about to change.
impl TaintStamps for TabStamps {
    fn stamp_all<'a>(&'a self, tabs: &'a [TabId]) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            for &tab in tabs {
                self.stamp(tab).await;
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::TabStamps;
    use browseros_core::TabId;
    use std::error::Error;

    async fn stamped_tabs(stamps: &TabStamps) -> Vec<TabId> {
        let mut tabs: Vec<TabId> = stamps.stamped.lock().await.keys().copied().collect();
        tabs.sort();
        tabs
    }

    #[tokio::test]
    async fn the_table_keeps_only_the_stamps_a_reconcile_may_still_need()
    -> Result<(), Box<dyn Error>> {
        let stamps = TabStamps::default();
        stamps.stamp(TabId(3)).await;
        stamps.stamp(TabId(5)).await;
        let fence = stamps.fence();
        stamps.stamp(TabId(77)).await;
        let mut kept = stamps.kept(&[TabId(5)], fence).await.tabs;
        kept.sort();
        assert_eq!(kept, [TabId(5), TabId(77)]);
        assert_eq!(
            stamped_tabs(&stamps).await,
            [TabId(5), TabId(77)],
            "3 is unlisted and older than the fence, so its stamp is gone; 5 is open"
        );

        let next = stamps.fence();
        assert_eq!(stamps.kept(&[TabId(5)], next).await.tabs, [TabId(5)]);
        assert_eq!(
            stamped_tabs(&stamps).await,
            [TabId(5)],
            "77 is unlisted and older than this fence"
        );

        stamps.forget(TabId(5)).await;
        assert!(stamped_tabs(&stamps).await.is_empty(), "5 closed");
        Ok(())
    }
}
