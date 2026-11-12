//! The tabs a run opened and the tabs it snapshotted, for the message of a ref that fails.
//!
//! A run with no task starts on a new blank tab, so a ref an agent kept from its last run fails
//! there, and today's advice ("take a new snapshot") points the wrong way: the ref was never on
//! this tab. Only a tab the run itself opened, and has not snapshotted in mode `'ai'`, proves
//! that; a task's tab or a popup may hold refs from earlier, so every other tab keeps the
//! ordinary message (design section 5, "A ref used in a tab this run opened").
//!
//! ```text
//!  context.newPage           opened(tab)        the run's built-in page opens through it too
//!  page/locator.ariaSnapshot snapshotted(tab)   mode 'ai', before it asks the page
//!  resolve::resolver         opened_unsnapshotted(tab) → the "not from this tab" text
//! ```

use std::sync::{Mutex, MutexGuard, PoisonError};

use browseros_core::TabId;

/// One run's opened and snapshotted tabs. Owned by the run's `RunState`, so it ends with the run:
/// the next run's built-in page is a new tab again. Each list holds a tab once and grows only by
/// the tabs one run touches.
#[derive(Debug, Default)]
pub(crate) struct OpenedTabs {
    inner: Mutex<Inner>,
}

#[derive(Debug, Default)]
struct Inner {
    opened: Vec<TabId>,
    snapshotted: Vec<TabId>,
}

impl OpenedTabs {
    /// This run opened `tab`.
    pub(crate) fn opened(&self, tab: TabId) {
        let mut inner = self.lock();
        if !inner.opened.contains(&tab) {
            inner.opened.push(tab);
        }
    }

    /// This run is taking a snapshot of `tab` that mints refs. Noted before the snapshot reads the
    /// page, so one that fails part way still counts and the tab keeps the ordinary message.
    pub(crate) fn snapshotted(&self, tab: TabId) {
        let mut inner = self.lock();
        if !inner.snapshotted.contains(&tab) {
            inner.snapshotted.push(tab);
        }
    }

    /// This run opened `tab` and has taken no snapshot of it that mints refs, so no ref can be
    /// that tab's.
    pub(crate) fn opened_unsnapshotted(&self, tab: TabId) -> bool {
        let inner = self.lock();
        inner.opened.contains(&tab) && !inner.snapshotted.contains(&tab)
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use browseros_core::TabId;

    use super::OpenedTabs;

    #[test]
    fn tests_that_only_a_tab_the_run_opened_and_never_snapshotted_reads_as_such() {
        let tabs = OpenedTabs::default();
        assert!(
            !tabs.opened_unsnapshotted(TabId(1)),
            "a tab the run did not open, such as a task's"
        );
        tabs.opened(TabId(1));
        tabs.opened(TabId(2));
        assert!(tabs.opened_unsnapshotted(TabId(1)));
        tabs.snapshotted(TabId(1));
        assert!(!tabs.opened_unsnapshotted(TabId(1)), "snapshotted since");
        assert!(tabs.opened_unsnapshotted(TabId(2)), "each tab is its own");
        tabs.snapshotted(TabId(3));
        assert!(!tabs.opened_unsnapshotted(TabId(3)));
    }
}
