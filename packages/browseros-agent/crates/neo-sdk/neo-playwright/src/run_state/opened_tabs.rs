//! The tabs a run opened and the tabs that issued refs, for the message of a ref that fails.
//!
//! A run with no task starts on a new blank tab, so a ref an agent kept from its last run fails
//! there, and today's advice ("take a new snapshot") points the wrong way: the ref was never on
//! this tab. Only a tab the run itself opened, and that has issued no refs, proves
//! that; a task's tab or a popup may hold refs from earlier, so every other tab keeps the
//! ordinary message (design section 5, "A ref used in a tab this run opened").
//!
//! ```text
//!  context.newPage           opened(tab)        the run's built-in page opens through it too
//!  page/locator.ariaSnapshot refs_issued(tab)   mode 'ai', before it asks the page
//!  locator.ref               refs_issued(tab)   after a successful read
//!  resolve::resolver         opened_without_refs(tab) → the "not from this tab" text
//! ```

use std::sync::{Mutex, MutexGuard, PoisonError};

use browseros_core::TabId;

/// One run's opened tabs and the tabs that may have issued refs. Owned by `RunState`, so it
/// ends with the run and the next run's built-in page is a new tab again. Each list holds a
/// tab once and grows only by the tabs one run touches.
#[derive(Debug, Default)]
pub(crate) struct OpenedTabs {
    inner: Mutex<Inner>,
}

#[derive(Debug, Default)]
struct Inner {
    opened: Vec<TabId>,
    with_refs: Vec<TabId>,
}

impl OpenedTabs {
    /// This run opened `tab`.
    pub(crate) fn opened(&self, tab: TabId) {
        let mut inner = self.lock();
        if !inner.opened.contains(&tab) {
            inner.opened.push(tab);
        }
    }

    /// This tab has issued refs, through a snapshot or `locator.ref()`. A snapshot notes this
    /// before reading the page because a failed snapshot may already have minted some refs.
    /// This affects only failure wording, never snapshot baselines.
    pub(crate) fn refs_issued(&self, tab: TabId) {
        let mut inner = self.lock();
        if !inner.with_refs.contains(&tab) {
            inner.with_refs.push(tab);
        }
    }

    /// This run opened `tab` and has issued no refs for it, so no ref can be that tab's.
    pub(crate) fn opened_without_refs(&self, tab: TabId) -> bool {
        let inner = self.lock();
        inner.opened.contains(&tab) && !inner.with_refs.contains(&tab)
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
    fn tests_that_only_a_tab_the_run_opened_and_never_issued_refs_reads_as_such() {
        let tabs = OpenedTabs::default();
        assert!(
            !tabs.opened_without_refs(TabId(1)),
            "a tab the run did not open, such as a task's"
        );
        tabs.opened(TabId(1));
        tabs.opened(TabId(2));
        assert!(tabs.opened_without_refs(TabId(1)));
        tabs.refs_issued(TabId(1));
        assert!(!tabs.opened_without_refs(TabId(1)), "issued refs since");
        assert!(tabs.opened_without_refs(TabId(2)), "each tab is its own");
        tabs.refs_issued(TabId(3));
        assert!(!tabs.opened_without_refs(TabId(3)));
    }
}
