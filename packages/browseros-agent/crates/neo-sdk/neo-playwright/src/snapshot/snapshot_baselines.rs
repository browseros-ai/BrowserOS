//! The last whole-page v1 snapshot text of each tab: what `ariaSnapshot({ delta: true })`
//! diffs against.
//!
//! ```text
//!  run N   prelude renders the v1 text ─ page.snapshotBaseline { text } ─▶ swap ─▶ previous
//!                                                                          │
//!  watcher main frame commits a new document, or the tab closes ─▶ forget ─┘
//! ```
//!
//! It lives here, on the host, because every `run` is a fresh JS realm, and not in the page's
//! isolated world because the stitched text holds cross-site frames' text, which that frame's
//! world must never hold. The prelude renders and diffs; Rust only keeps the text and decides
//! when it stops naming the tab's document.

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
/// navigations). Shared by the `Playwright` facade, whose op swaps a tab's text, and the target
/// watcher, which forgets it. The lock is held only to swap or remove one entry.
///
/// An entry ends when the tab's main frame commits a new document or the tab closes, so what
/// is stored is at most one text per open tab, each at most [`MAX_BASELINE_BYTES`].
#[derive(Debug, Default)]
pub(crate) struct SnapshotBaselines {
    tabs: Mutex<HashMap<TargetId, String>>,
}

impl SnapshotBaselines {
    /// Stores `text` as the tab's baseline and returns the one it replaces: `None` when the tab
    /// had none, or its document changed since. A text longer than [`MAX_BASELINE_BYTES`] is
    /// not kept, and the tab is left with none.
    pub(crate) fn swap(&self, tab: &TargetId, text: String) -> Option<String> {
        let mut tabs = self.lock();
        if text.len() > MAX_BASELINE_BYTES {
            return tabs.remove(tab);
        }
        tabs.insert(tab.clone(), text)
    }

    /// The tab's main frame committed a new document, or the tab closed: its baseline names a
    /// document the agent can no longer act on, so its next delta starts over.
    pub(crate) fn forget(&self, tab: &TargetId) {
        self.lock().remove(tab);
    }

    /// The watcher lost events, so a commit or a close may have gone unseen: no baseline is
    /// trusted to still name its tab's document.
    pub(crate) fn forget_all(&self) {
        self.lock().clear();
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<TargetId, String>> {
        self.tabs.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use browseros_core::TargetId;

    use super::{MAX_BASELINE_BYTES, SnapshotBaselines};

    fn tab(id: &str) -> TargetId {
        TargetId(id.to_owned())
    }

    #[test]
    fn tests_that_a_swap_returns_the_tab_s_previous_text() {
        let baselines = SnapshotBaselines::default();
        assert_eq!(baselines.swap(&tab("A"), "one".to_owned()), None);
        assert_eq!(
            baselines.swap(&tab("A"), "two".to_owned()).as_deref(),
            Some("one")
        );
        assert_eq!(
            baselines.swap(&tab("A"), "three".to_owned()).as_deref(),
            Some("two")
        );
    }

    #[test]
    fn tests_that_two_tabs_keep_their_own_baselines() {
        let baselines = SnapshotBaselines::default();
        baselines.swap(&tab("A"), "a".to_owned());
        assert_eq!(baselines.swap(&tab("B"), "b".to_owned()), None);
        assert_eq!(
            baselines.swap(&tab("A"), "a2".to_owned()).as_deref(),
            Some("a")
        );
        assert_eq!(
            baselines.swap(&tab("B"), "b2".to_owned()).as_deref(),
            Some("b")
        );
    }

    #[test]
    fn tests_that_forgetting_a_tab_leaves_the_others() {
        let baselines = SnapshotBaselines::default();
        baselines.swap(&tab("A"), "a".to_owned());
        baselines.swap(&tab("B"), "b".to_owned());
        baselines.forget(&tab("A"));
        assert_eq!(baselines.swap(&tab("A"), "a2".to_owned()), None);
        assert_eq!(
            baselines.swap(&tab("B"), "b2".to_owned()).as_deref(),
            Some("b")
        );
        baselines.forget_all();
        assert_eq!(baselines.swap(&tab("A"), "a3".to_owned()), None);
        assert_eq!(baselines.swap(&tab("B"), "b3".to_owned()), None);
    }

    #[test]
    fn tests_that_a_text_over_the_bound_is_not_kept() {
        let baselines = SnapshotBaselines::default();
        baselines.swap(&tab("A"), "small".to_owned());
        let huge = "x".repeat(MAX_BASELINE_BYTES + 1);
        assert_eq!(
            baselines.swap(&tab("A"), huge).as_deref(),
            Some("small"),
            "the previous text still comes back"
        );
        assert_eq!(
            baselines.swap(&tab("A"), "next".to_owned()),
            None,
            "nothing was kept"
        );
        let exact = "x".repeat(MAX_BASELINE_BYTES);
        baselines.swap(&tab("A"), exact.clone());
        assert_eq!(baselines.swap(&tab("A"), "after".to_owned()), Some(exact));
    }
}
