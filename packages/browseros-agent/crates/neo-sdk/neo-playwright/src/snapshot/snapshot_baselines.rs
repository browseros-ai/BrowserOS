//! The last whole-page v1 snapshot text each tab's caller was given: what
//! `ariaSnapshot({ delta: true })` diffs against.
//!
//! ```text
//!  page.ariaSnapshot      reads the tab's document generation, then the tree
//!                           → { json, partial, document: <generation> }
//!  prelude                renders the v1 text
//!  page.snapshotBaseline  delta only: the STORED text of that tab and document, never a pending one
//!  page.snapshotPending   the text and the string the call returned, pending on the run's state
//!  run service, run end   ok, and the returned string is in the reply → commit the last such text
//!                         (only while <generation> is still the tab's); otherwise drop them all
//!  watcher                the main frame commits a new document → a new generation, no text
//!                         the tab closes → its entry goes; events were lost → every text goes
//! ```
//!
//! It lives here, on the host, because every `run` is a fresh JS realm, and not in the page's
//! isolated world because the stitched text holds cross-site frames' text, which that frame's
//! world must never hold. The prelude renders and diffs; Rust keeps the text and decides when it
//! stops naming the tab's document. A text is stored only once the run that took it ended with
//! the text in the caller's hands (`PendingBaselines`), so a delta is only ever relative to text
//! the caller holds: a run that threw, or that only inspected the snapshot in code, leaves the
//! older baseline in place.

use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard, PoisonError};

use browseros_core::TargetId;

/// The longest text kept for one tab, in UTF-8 bytes. The v1 text is cut at `maxTokens` (8,000
/// by default: 32,000 characters), so an ordinary snapshot is far below it; a longer one is not
/// kept, and the tab keeps the baseline it had. It is the run result's own cap (neo-server's
/// `Defaults::MAX_RESULT_BYTES`): a longer text could not reach the caller as a run's value
/// anyway.
pub(crate) const MAX_BASELINE_BYTES: usize = 256 * 1024;

/// Every tab's baseline, by the tab's target (a tab's main frame keeps its target's id across
/// navigations). Shared by the `Playwright` facade, whose ops read the generation and the
/// stored text, the runs' pending baselines, which commit to it, and the target watcher, which
/// moves the generation on. The lock is held only to read or change one entry, never across an
/// await.
///
/// What is stored is at most one text per open tab, each at most [`MAX_BASELINE_BYTES`]; an
/// entry goes when its tab closes.
#[derive(Debug, Default)]
pub(crate) struct SnapshotBaselines {
    inner: Mutex<Baselines>,
}

#[derive(Debug, Default)]
struct Baselines {
    tabs: HashMap<TargetId, TabBaseline>,
    /// The last generation handed out. Generations only grow, so a token read before a change
    /// never equals one read after it.
    last: u32,
    /// The generation every tab is at least at: raised when the watcher lost events, which may
    /// have hidden any tab's commit.
    floor: u32,
}

#[derive(Debug, Default)]
struct TabBaseline {
    /// The tab's main document, as counted here: a fresh generation when the entry is made,
    /// and another at each commit seen.
    generation: u32,
    /// The last text of that document the tab's caller was given.
    text: Option<String>,
}

impl SnapshotBaselines {
    /// The tab's current document generation, for a snapshot to read before its tree. Makes the
    /// tab's entry on first use, with a generation of its own: a commit needs that entry, so a
    /// tab that closed (its entry removed) never takes a baseline again.
    pub(crate) fn generation(&self, tab: &TargetId) -> u32 {
        let mut inner = self.lock();
        if !inner.tabs.contains_key(tab) {
            let generation = inner.next();
            inner.tabs.insert(
                tab.clone(),
                TabBaseline {
                    generation,
                    text: None,
                },
            );
        }
        inner.current(tab)
    }

    /// The tab's stored baseline, if it is of the document generation `document` names.
    pub(crate) fn stored(&self, tab: &TargetId, document: u32) -> Option<String> {
        let inner = self.lock();
        if inner.current(tab) != document {
            return None;
        }
        inner.tabs.get(tab).and_then(|entry| entry.text.clone())
    }

    /// Whether [`SnapshotBaselines::stored`] would answer a text, without copying it.
    pub(crate) fn has_stored(&self, tab: &TargetId, document: u32) -> bool {
        let inner = self.lock();
        inner.current(tab) == document
            && inner
                .tabs
                .get(tab)
                .is_some_and(|entry| entry.text.is_some())
    }

    /// `text` becomes the tab's baseline: its run ended with it in the caller's hands. Kept only
    /// while the tab is still on the document `document` names, and only up to
    /// [`MAX_BASELINE_BYTES`]; otherwise the tab keeps what it had, which the caller also holds.
    pub(crate) fn commit(&self, tab: &TargetId, document: u32, text: String) {
        let mut inner = self.lock();
        if inner.current(tab) != document || text.len() > MAX_BASELINE_BYTES {
            return;
        }
        if let Some(entry) = inner.tabs.get_mut(tab) {
            entry.text = Some(text);
        }
    }

    /// The tab's caller may hold a newer text than the stored one, which the run could not keep
    /// (`PendingBaselines` dropped it to stay in bound): no baseline is safe to compare against,
    /// so the next delta comes back whole. Only while the tab is on the document `document`
    /// names; a newer document has no baseline anyway.
    pub(crate) fn forget(&self, tab: &TargetId, document: u32) {
        let mut inner = self.lock();
        if inner.current(tab) != document {
            return;
        }
        if let Some(entry) = inner.tabs.get_mut(tab) {
            entry.text = None;
        }
    }

    /// The tab's main frame committed a new document: its baseline names a document the agent
    /// can no longer act on, so its next delta starts over, and a pending text of the old one is
    /// not committed.
    pub(crate) fn document_changed(&self, tab: &TargetId) {
        let mut inner = self.lock();
        let generation = inner.next();
        let entry = inner.tabs.entry(tab.clone()).or_default();
        entry.generation = generation;
        entry.text = None;
    }

    /// The tab closed.
    pub(crate) fn tab_closed(&self, tab: &TargetId) {
        self.lock().tabs.remove(tab);
    }

    /// The watcher lost events, so a commit or a close may have gone unseen: no baseline is
    /// trusted to still name its tab's document, nor any generation read before now.
    pub(crate) fn events_lost(&self) {
        let mut inner = self.lock();
        inner.floor = inner.next();
        for entry in inner.tabs.values_mut() {
            entry.text = None;
        }
    }

    fn lock(&self) -> MutexGuard<'_, Baselines> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl Baselines {
    fn current(&self, tab: &TargetId) -> u32 {
        let own = self.tabs.get(tab).map_or(0, |entry| entry.generation);
        own.max(self.floor)
    }

    /// A generation no one has read yet. Four billion commits wrap it, after which a token from
    /// before the wrap could match again; a process does not live that long.
    fn next(&mut self) -> u32 {
        self.last = self.last.wrapping_add(1);
        self.last
    }
}

#[cfg(test)]
mod tests {
    use browseros_core::TargetId;

    use super::{MAX_BASELINE_BYTES, SnapshotBaselines};

    fn tab(id: &str) -> TargetId {
        TargetId(id.to_owned())
    }

    /// A snapshot of `id` whose run ended with its text given to the caller.
    fn given(baselines: &SnapshotBaselines, id: &str, text: &str) {
        let read = baselines.generation(&tab(id));
        baselines.commit(&tab(id), read, text.to_owned());
    }

    fn stored(baselines: &SnapshotBaselines, id: &str) -> Option<String> {
        baselines.stored(&tab(id), baselines.generation(&tab(id)))
    }

    #[test]
    fn tests_that_a_committed_text_is_the_tab_s_baseline_until_the_next() {
        let baselines = SnapshotBaselines::default();
        assert_eq!(stored(&baselines, "A"), None);
        given(&baselines, "A", "one");
        assert_eq!(stored(&baselines, "A").as_deref(), Some("one"));
        assert!(baselines.has_stored(&tab("A"), baselines.generation(&tab("A"))));
        given(&baselines, "A", "two");
        assert_eq!(stored(&baselines, "A").as_deref(), Some("two"));
    }

    #[test]
    fn tests_that_two_tabs_keep_their_own_baselines() {
        let baselines = SnapshotBaselines::default();
        given(&baselines, "A", "a");
        assert_eq!(stored(&baselines, "B"), None);
        given(&baselines, "B", "b");
        assert_eq!(stored(&baselines, "A").as_deref(), Some("a"));
        assert_eq!(stored(&baselines, "B").as_deref(), Some("b"));
    }

    #[test]
    fn tests_that_a_new_document_or_a_close_leaves_the_other_tabs() {
        let baselines = SnapshotBaselines::default();
        given(&baselines, "A", "a");
        given(&baselines, "B", "b");
        given(&baselines, "C", "c");
        baselines.document_changed(&tab("A"));
        baselines.tab_closed(&tab("C"));
        assert_eq!(stored(&baselines, "A"), None);
        assert_eq!(stored(&baselines, "C"), None);
        assert_eq!(stored(&baselines, "B").as_deref(), Some("b"));
        baselines.events_lost();
        assert_eq!(stored(&baselines, "B"), None);
    }

    #[test]
    fn tests_that_a_text_of_a_document_gone_since_its_read_is_not_committed() {
        let baselines = SnapshotBaselines::default();
        given(&baselines, "A", "first");
        let read = baselines.generation(&tab("A"));
        assert_eq!(baselines.stored(&tab("A"), read).as_deref(), Some("first"));
        // The tree read happens, then the main frame commits before the run ends.
        baselines.document_changed(&tab("A"));
        baselines.commit(&tab("A"), read, "old page".to_owned());
        assert_eq!(stored(&baselines, "A"), None, "the new document has none");
        assert_eq!(
            baselines.stored(&tab("A"), read),
            None,
            "nor does the old generation answer"
        );
        // The same after lost events, on a tab that never committed before.
        let read = baselines.generation(&tab("B"));
        baselines.events_lost();
        baselines.commit(&tab("B"), read, "b".to_owned());
        assert_eq!(stored(&baselines, "B"), None);
    }

    #[test]
    fn tests_that_a_closed_tab_takes_no_baseline_from_a_run_that_ends_after() {
        let baselines = SnapshotBaselines::default();
        let read = baselines.generation(&tab("A"));
        baselines.tab_closed(&tab("A"));
        baselines.commit(&tab("A"), read, "of a closed tab".to_owned());
        assert_eq!(baselines.lock().tabs.len(), 0, "no entry was made again");
    }

    #[test]
    fn tests_that_forgetting_drops_only_the_named_document_s_text() {
        let baselines = SnapshotBaselines::default();
        given(&baselines, "A", "a");
        let read = baselines.generation(&tab("A"));
        baselines.forget(&tab("A"), read.wrapping_sub(1));
        assert_eq!(stored(&baselines, "A").as_deref(), Some("a"));
        baselines.forget(&tab("A"), read);
        assert_eq!(stored(&baselines, "A"), None);
    }

    #[test]
    fn tests_that_a_text_over_the_bound_leaves_the_baseline_it_had() {
        let baselines = SnapshotBaselines::default();
        given(&baselines, "A", "small");
        given(&baselines, "A", &"x".repeat(MAX_BASELINE_BYTES + 1));
        assert_eq!(stored(&baselines, "A").as_deref(), Some("small"));
        let exact = "x".repeat(MAX_BASELINE_BYTES);
        given(&baselines, "A", &exact);
        assert_eq!(stored(&baselines, "A"), Some(exact));
    }
}
