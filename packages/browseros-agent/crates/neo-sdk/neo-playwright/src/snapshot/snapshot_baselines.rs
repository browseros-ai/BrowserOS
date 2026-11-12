//! The last whole-page v1 snapshot text of each tab: what `ariaSnapshot({ delta: true })`
//! diffs against.
//!
//! ```text
//!  page.ariaSnapshot      reads the tab's document generation, then the tree
//!                           → { json, partial, document: <generation> }
//!  prelude                renders the v1 text
//!  page.snapshotBaseline  { text, document } → the previous text; stored only while
//!                           <generation> is still the tab's current one
//!  watcher                the main frame commits a new document → a new generation, no text
//!                         the tab closes → its entry goes; events were lost → every text goes
//! ```
//!
//! It lives here, on the host, because every `run` is a fresh JS realm, and not in the page's
//! isolated world because the stitched text holds cross-site frames' text, which that frame's
//! world must never hold. The prelude renders and diffs; Rust only keeps the text and decides
//! when it stops naming the tab's document. The generation closes the gap between the two ops:
//! without it, a document that commits after the tree read and before the swap would be handed
//! the old document's text as its baseline.

use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard, PoisonError};

use browseros_core::TargetId;

/// The longest text kept for one tab, in UTF-8 bytes. The v1 text is cut at `maxTokens` (8,000
/// by default: 32,000 characters), so an ordinary snapshot is far below it; a script that raises
/// `maxTokens` past it keeps no baseline, and its next delta comes back whole. It is the run
/// result's own cap (neo-server's `Defaults::MAX_RESULT_BYTES`): a longer text could not come
/// back as a run's value anyway.
pub(crate) const MAX_BASELINE_BYTES: usize = 256 * 1024;

/// Every tab's baseline, by the tab's target (a tab's main frame keeps its target's id across
/// navigations). Shared by the `Playwright` facade, whose ops read the generation and swap the
/// text, and the target watcher, which moves the generation on. The lock is held only to read
/// or change one entry, never across an await.
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
    /// The tab's main document, as counted here; 0 until its first commit seen.
    generation: u32,
    /// The text of the tab's last whole-page v1 snapshot of that document.
    text: Option<String>,
}

impl SnapshotBaselines {
    /// The tab's current document generation, for a snapshot to read before its tree and hand
    /// back with its text.
    pub(crate) fn generation(&self, tab: &TargetId) -> u32 {
        self.lock().current(tab)
    }

    /// Stores `text` as the tab's baseline and returns the one it replaces: `None` when the tab
    /// had none, or its document changed since. `read` is the generation the snapshot read
    /// before its tree; when the tab has moved on since, the text is of a document that is gone
    /// and is not kept. A text longer than [`MAX_BASELINE_BYTES`] is not kept either, and the tab
    /// is left with none.
    pub(crate) fn swap(&self, tab: &TargetId, read: u32, text: String) -> Option<String> {
        let mut inner = self.lock();
        let current = inner.current(tab);
        let entry = inner.tabs.entry(tab.clone()).or_default();
        if read != current {
            return None;
        }
        if text.len() > MAX_BASELINE_BYTES {
            return entry.text.take();
        }
        entry.text.replace(text)
    }

    /// The tab's main frame committed a new document: its baseline names a document the agent
    /// can no longer act on, so its next delta starts over, and a snapshot of the old one that
    /// is still on its way is not kept.
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

    /// One whole-page snapshot of `id` as the ops make it: the generation, then the swap.
    fn snapshot(baselines: &SnapshotBaselines, id: &str, text: &str) -> Option<String> {
        let read = baselines.generation(&tab(id));
        baselines.swap(&tab(id), read, text.to_owned())
    }

    #[test]
    fn tests_that_a_swap_returns_the_tab_s_previous_text() {
        let baselines = SnapshotBaselines::default();
        assert_eq!(snapshot(&baselines, "A", "one"), None);
        assert_eq!(snapshot(&baselines, "A", "two").as_deref(), Some("one"));
        assert_eq!(snapshot(&baselines, "A", "three").as_deref(), Some("two"));
    }

    #[test]
    fn tests_that_two_tabs_keep_their_own_baselines() {
        let baselines = SnapshotBaselines::default();
        snapshot(&baselines, "A", "a");
        assert_eq!(snapshot(&baselines, "B", "b"), None);
        assert_eq!(snapshot(&baselines, "A", "a2").as_deref(), Some("a"));
        assert_eq!(snapshot(&baselines, "B", "b2").as_deref(), Some("b"));
    }

    #[test]
    fn tests_that_a_new_document_or_a_close_leaves_the_other_tabs() {
        let baselines = SnapshotBaselines::default();
        snapshot(&baselines, "A", "a");
        snapshot(&baselines, "B", "b");
        snapshot(&baselines, "C", "c");
        baselines.document_changed(&tab("A"));
        baselines.tab_closed(&tab("C"));
        assert_eq!(snapshot(&baselines, "A", "a2"), None);
        assert_eq!(snapshot(&baselines, "C", "c2"), None);
        assert_eq!(snapshot(&baselines, "B", "b2").as_deref(), Some("b"));
        baselines.events_lost();
        assert_eq!(snapshot(&baselines, "A", "a3"), None);
        assert_eq!(snapshot(&baselines, "B", "b3"), None);
    }

    #[test]
    fn tests_that_a_snapshot_of_a_document_gone_since_its_read_is_not_kept() {
        let baselines = SnapshotBaselines::default();
        snapshot(&baselines, "A", "first");
        // The tree read happens, then the main frame commits before the swap.
        let read = baselines.generation(&tab("A"));
        baselines.document_changed(&tab("A"));
        assert_eq!(baselines.swap(&tab("A"), read, "old page".to_owned()), None);
        assert_eq!(
            snapshot(&baselines, "A", "new page"),
            None,
            "the new document has no baseline: the old one's text was not kept"
        );
        assert_eq!(
            snapshot(&baselines, "A", "new page").as_deref(),
            Some("new page")
        );
        // The same after lost events, on a tab that never committed before.
        let read = baselines.generation(&tab("B"));
        baselines.events_lost();
        assert_eq!(baselines.swap(&tab("B"), read, "b".to_owned()), None);
        assert_eq!(snapshot(&baselines, "B", "b2"), None);
    }

    #[test]
    fn tests_that_a_text_over_the_bound_is_not_kept() {
        let baselines = SnapshotBaselines::default();
        snapshot(&baselines, "A", "small");
        let huge = "x".repeat(MAX_BASELINE_BYTES + 1);
        assert_eq!(
            snapshot(&baselines, "A", &huge).as_deref(),
            Some("small"),
            "the previous text still comes back"
        );
        assert_eq!(snapshot(&baselines, "A", "next"), None, "nothing was kept");
        let exact = "x".repeat(MAX_BASELINE_BYTES);
        snapshot(&baselines, "A", &exact);
        assert_eq!(snapshot(&baselines, "A", "after"), Some(exact));
    }
}
