//! The task book's seam to whatever must outlive Neo's server: each change goes out as a
//! [`BookChange`], and a restart comes back in as [`SavedTask`]s.
//!
//! ```text
//!  TaskBook ── join / acted / ensure / detach ──▶ BookChange ──▶ BookSink (under the registry's
//!     ▲                                                          lock, so in book order)
//!     └── TabRegistry::restore_book(Vec<SavedTask>) ◀── a saved record, once, at start
//! ```
//!
//! Who holds which tab decides what a task-scoped run may reach, so it must survive a restart of
//! Neo's server while the browser lives on. The book stays in memory; a sink saves its changes,
//! and nothing else reads them.

#[cfg(test)]
use std::mem;
#[cfg(test)]
use std::sync::{Mutex, PoisonError};

use browseros_core::TabId;
use neo_trace::TaskId;

use super::task_book::{Holder, TaskOrigin};

/// One change a restart must see, in book order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum BookChange {
    /// `tab` is now held as `holder` (an open, a popup inherited from its opener, a restore), or
    /// its holder's task moved to [`TaskOrigin::Host`]: then one change per held tab of that task.
    Held {
        /// The tab.
        tab: TabId,
        /// Its holder now.
        holder: Holder,
        /// Where the holder's task came from, now.
        origin: TaskOrigin,
    },
    /// `tab` became its task's page (`TaskRecord::last_used_tab` moved to it). Only on a move,
    /// never per op.
    Page {
        /// The tab.
        tab: TabId,
    },
    /// `tab` left its task: it closed, or Neo's blank tab took it.
    Gone {
        /// The tab.
        tab: TabId,
    },
}

/// Where the book's changes go. Set once ([`TabRegistry::set_book_sink`]); until then they go
/// nowhere.
///
/// It is called under the registry's lock, so changes reach it in book order. It must not block,
/// do I/O or call back into the registry: every tab operation waits on that lock. A writer that
/// saves them sends each one down a channel to its own task.
///
/// [`TabRegistry::set_book_sink`]: super::TabRegistry::set_book_sink
pub(crate) trait BookSink: Send + Sync {
    /// The book just changed.
    fn changed(&self, change: BookChange);
}

/// One task with live tabs, as a saved record rebuilt it for [`TabRegistry::restore_book`].
///
/// [`TabRegistry::restore_book`]: super::TabRegistry::restore_book
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SavedTask {
    /// The task.
    pub(crate) id: TaskId,
    /// Its family's root ([`TaskRecord::family`]).
    ///
    /// [`TaskRecord::family`]: super::TaskRecord::family
    pub(crate) family: TaskId,
    /// Where it came from.
    pub(crate) origin: TaskOrigin,
    /// Its live tabs in their saved order, oldest first.
    pub(crate) tabs: Vec<TabId>,
    /// The tab the record last marked as its page, if it is one of `tabs`; otherwise the newest
    /// is the page, as `TaskRecord::page_tab` already chooses. So after a restart the page is
    /// still one its own script last acted on, never another task's tab.
    pub(crate) page: Option<TabId>,
}

/// Every change a book sent, in order, for tests to read back.
#[cfg(test)]
#[derive(Default)]
pub(super) struct BookLog(Mutex<Vec<BookChange>>);

#[cfg(test)]
impl BookLog {
    /// The changes sent since the last take.
    pub(super) fn take(&self) -> Vec<BookChange> {
        mem::take(&mut *self.0.lock().unwrap_or_else(PoisonError::into_inner))
    }
}

#[cfg(test)]
impl BookSink for BookLog {
    fn changed(&self, change: BookChange) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(change);
    }
}
