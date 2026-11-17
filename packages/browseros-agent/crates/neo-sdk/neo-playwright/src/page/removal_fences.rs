//! The tabs whose init-script removals are still in flight, for the facade's whole life.

use std::collections::HashMap;
use std::fmt;
use std::future::Future;
use std::pin::pin;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use browseros_core::{TabId, TargetId};
use futures_util::future::{self, Either};
use tokio::sync::Notify;

use crate::target_crashes::TargetCrashes;

/// Every tab with an init-script or binding removal in flight (`page/init_script.rs`), by any
/// run. The run's end does not wait for its removals: one busy tab would add up to the
/// teardown grace to the reply of every run that registered there. The wait moves here, to the
/// tab: the next op admitted on that tab, by any run of any task, waits for the tab's removals
/// first ([`OpScope::admit`](crate::op_scope::OpScope::admit)), so a later run never loads an
/// earlier run's init script or calls its binding. An op on another tab never waits.
///
/// ```text
///  run A ends ─ each registration dropped ─ TabFence::hold, before A's reply
///             └▶ a task of its own: the removal, then the hold released
///  run B: an op on that tab ─ OpScope::admit ─ settled(tab) ─ observe_page ─ …
/// ```
///
/// A hold ends when its removal answers or fails, which a closed tab's does at once (the CDP
/// client fails a detached session's requests with `SessionGone`), or when the tab's renderer
/// crashes, which answers nothing until the CDP timeout. So no tab waits forever. The residual:
/// an owner's own navigation on that tab before the browser applies the already-sent removal can
/// run the script once.
pub(crate) struct RemovalFences {
    crashes: Arc<TargetCrashes>,
    /// Removals in flight per tab; a tab with none has no entry. A std mutex, held for a count
    /// update and never across an await.
    held: Mutex<HashMap<TabId, usize>>,
    /// Woken at every release, so each waiter rechecks its own tab.
    released: Notify,
}

// A hand-written `Debug`: the crash facts are not worth printing.
impl fmt::Debug for RemovalFences {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RemovalFences")
            .field("held", &*self.lock())
            .finish_non_exhaustive()
    }
}

impl RemovalFences {
    /// The table for one facade, whose crash facts end the hold of a crashed tab.
    pub(crate) fn new(crashes: Arc<TargetCrashes>) -> Self {
        Self {
            crashes,
            held: Mutex::default(),
            released: Notify::new(),
        }
    }

    /// Whether a removal on `tab` is in flight now.
    pub(crate) fn holds(&self, tab: TabId) -> bool {
        self.lock().contains_key(&tab)
    }

    /// Resolves once no removal on `tab` is in flight, at once if none is now. The caller bounds
    /// it with its op's own deadline and abort.
    pub(crate) async fn settled(&self, tab: TabId) {
        loop {
            // Enabled before the check, so a release between the two still wakes it.
            let mut released = pin!(self.released.notified());
            released.as_mut().enable();
            if !self.holds(tab) {
                return;
            }
            released.await;
        }
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<TabId, usize>> {
        self.held.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Where a registration's removal will hold a fence: the facade's table, the tab, and the page
/// target whose crash ends the hold. Inert: a registration keeps it from its add, and only its
/// drop turns it into a [`RemovalHold`], so the run's own ops never wait on a live script.
#[derive(Debug)]
pub(crate) struct TabFence {
    fences: Arc<RemovalFences>,
    tab: TabId,
    target: TargetId,
}

impl TabFence {
    pub(crate) fn new(fences: &Arc<RemovalFences>, tab: TabId, target: TargetId) -> Self {
        Self {
            fences: Arc::clone(fences),
            tab,
            target,
        }
    }

    /// Holds the tab's fence from now on. Synchronous, so a removal started before a reply is
    /// already held when the reply leaves.
    pub(crate) fn hold(self) -> RemovalHold {
        *self.fences.lock().entry(self.tab).or_default() += 1;
        RemovalHold { fence: self }
    }
}

/// One removal in flight on a tab. Dropping it releases the tab's fence and wakes its waiters.
#[derive(Debug)]
#[must_use]
pub(crate) struct RemovalHold {
    fence: TabFence,
}

impl RemovalHold {
    /// Runs `removal` to its end and holds the fence until then, or until the tab's renderer is
    /// recorded crashed, whichever comes first. After a crash the removal still runs to its end in
    /// the background, but the tab's next op goes on now and meets the crash at its admission
    /// instead of the CDP timeout.
    pub(crate) async fn during(self, removal: impl Future<Output = ()>) {
        let crashes = Arc::clone(&self.fence.fences.crashes);
        let target = self.fence.target.clone();
        let removal = pin!(removal);
        let crashed = pin!(crashes.crashed(&target));
        match future::select(removal, crashed).await {
            Either::Left(((), _crashed)) => drop(self),
            Either::Right(((), removal)) => {
                drop(self);
                removal.await;
            }
        }
    }
}

impl Drop for RemovalHold {
    fn drop(&mut self) {
        let fence = &self.fence;
        {
            let mut held = fence.fences.lock();
            if let Some(count) = held.get_mut(&fence.tab) {
                *count = count.saturating_sub(1);
                if *count == 0 {
                    held.remove(&fence.tab);
                }
            }
        }
        fence.fences.released.notify_waiters();
    }
}
