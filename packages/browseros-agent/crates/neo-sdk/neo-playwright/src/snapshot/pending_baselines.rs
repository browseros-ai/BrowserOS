//! One run's whole-page snapshot texts that may become their tabs' baselines when it ends.
//!
//! During the run a text is only pending: nothing compares against it, not even a later delta
//! of the same run, which reads the baseline that was stored when the run started. When the run
//! ends its host settles them ([`PendingBaselines::settle`]): if the run ended ok, each tab's
//! last text whose returned string the caller was given (in the run's value or its console)
//! becomes that tab's baseline; every other text is dropped. So each case errs the safe way:
//! the next delta compares against older text the caller does hold, or comes back whole.

use std::mem;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};

use browseros_core::TargetId;

use super::SnapshotBaselines;

/// At most this many pending texts per tab in one run; a run that snapshots one tab more often
/// keeps its latest. An older text dropped here leaves the stored baseline, which the caller
/// also holds, so the bound only ever costs a larger delta. 16 of the default 32,000-character
/// texts is half a megabyte.
const PENDING_PER_TAB: usize = 16;

/// One run's pending texts, in the order the run took them. Owned by the run's `RunState`, so
/// whatever the host does not settle goes with the run.
#[derive(Debug, Default)]
pub(crate) struct PendingBaselines {
    pending: Mutex<Vec<Pending>>,
    /// Where a settled text goes: the `Playwright` facade's store, set by the first op that
    /// records a text.
    store: OnceLock<Arc<SnapshotBaselines>>,
}

#[derive(Debug)]
struct Pending {
    tab: TargetId,
    /// The tab's document generation the snapshot read before its tree.
    document: u32,
    /// The full v1 text: what becomes the baseline.
    text: String,
    /// What the call returned when it was not `text` itself: a delta call's delta.
    returned: Option<String>,
}

impl PendingBaselines {
    /// Records one whole-page snapshot of `tab` taken by this run.
    pub(crate) fn record(
        &self,
        store: &Arc<SnapshotBaselines>,
        tab: TargetId,
        document: u32,
        text: String,
        returned: Option<String>,
    ) {
        let _ = self.store.set(Arc::clone(store));
        let mut pending = self.lock();
        if pending.iter().filter(|entry| entry.tab == tab).count() >= PENDING_PER_TAB
            && let Some(oldest) = pending.iter().position(|entry| entry.tab == tab)
        {
            pending.remove(oldest);
        }
        pending.push(Pending {
            tab,
            document,
            text,
            returned,
        });
    }

    /// Ends the run's pending texts. With `ended_ok`, each tab's last text whose returned string
    /// `given` finds in the reply is committed; everything else is dropped. Takes them, so a
    /// host that runs several scripts on one state settles each run's own.
    pub(crate) fn settle(&self, ended_ok: bool, given: &dyn Fn(&str) -> bool) {
        let pending = mem::take(&mut *self.lock());
        let Some(store) = self.store.get() else {
            return;
        };
        if !ended_ok {
            return;
        }
        let mut committed: Vec<&TargetId> = Vec::new();
        for entry in pending.iter().rev() {
            if committed.contains(&&entry.tab) {
                continue;
            }
            if given(entry.returned.as_deref().unwrap_or(&entry.text)) {
                store.commit(&entry.tab, entry.document, entry.text.clone());
                committed.push(&entry.tab);
            }
        }
    }

    fn lock(&self) -> MutexGuard<'_, Vec<Pending>> {
        self.pending.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use browseros_core::TargetId;

    use super::{PENDING_PER_TAB, PendingBaselines};
    use crate::snapshot::SnapshotBaselines;

    fn tab(id: &str) -> TargetId {
        TargetId(id.to_owned())
    }

    fn stored(store: &SnapshotBaselines, id: &str) -> Option<String> {
        store.stored(&tab(id), store.generation(&tab(id)))
    }

    /// One run on `store` that takes the snapshots `taken` (tab, full text, what the call
    /// returned when not the text) and hands the caller `reply`.
    fn run(
        store: &Arc<SnapshotBaselines>,
        taken: &[(&str, &str, Option<&str>)],
        ended_ok: bool,
        reply: &str,
    ) {
        let pending = PendingBaselines::default();
        for &(id, text, returned) in taken {
            let document = store.generation(&tab(id));
            pending.record(
                store,
                tab(id),
                document,
                text.to_owned(),
                returned.map(str::to_owned),
            );
        }
        pending.settle(ended_ok, &|returned: &str| reply.contains(returned));
    }

    #[test]
    fn tests_that_a_text_the_caller_was_given_becomes_the_baseline() {
        let store = Arc::new(SnapshotBaselines::default());
        run(&store, &[("A", "page one", None)], true, "\"page one\"");
        assert_eq!(stored(&store, "A").as_deref(), Some("page one"));
        // A delta call: its delta is what the caller got, its full text what is stored.
        run(
            &store,
            &[("A", "page two", Some("+ two"))],
            true,
            "[\"+ two\"]",
        );
        assert_eq!(stored(&store, "A").as_deref(), Some("page two"));
    }

    #[test]
    fn tests_that_a_failed_run_or_a_text_the_caller_never_got_is_dropped() {
        let store = Arc::new(SnapshotBaselines::default());
        run(&store, &[("A", "seen", None)], true, "seen");
        run(&store, &[("A", "thrown", None)], false, "thrown");
        assert_eq!(
            stored(&store, "A").as_deref(),
            Some("seen"),
            "the run failed"
        );
        run(&store, &[("A", "inspected", None)], true, "9");
        assert_eq!(
            stored(&store, "A").as_deref(),
            Some("seen"),
            "only its length came back"
        );
        run(&store, &[("A", "a long page", None)], true, "a long");
        assert_eq!(
            stored(&store, "A").as_deref(),
            Some("seen"),
            "a slice is not the text"
        );
        run(&store, &[("A", "x", Some("+ x"))], true, "x");
        assert_eq!(
            stored(&store, "A").as_deref(),
            Some("seen"),
            "a delta call's own string is what must come back"
        );
    }

    #[test]
    fn tests_that_the_last_text_given_per_tab_wins() {
        let store = Arc::new(SnapshotBaselines::default());
        run(
            &store,
            &[
                ("A", "a1", None),
                ("B", "b1", None),
                ("A", "a2", None),
                ("A", "a3", None),
            ],
            true,
            "a1 a2 b1",
        );
        assert_eq!(
            stored(&store, "A").as_deref(),
            Some("a2"),
            "a3 was not given"
        );
        assert_eq!(stored(&store, "B").as_deref(), Some("b1"));
    }

    #[test]
    fn tests_that_a_document_change_during_the_run_drops_its_pending_text() {
        let store = Arc::new(SnapshotBaselines::default());
        let pending = PendingBaselines::default();
        let document = store.generation(&tab("A"));
        pending.record(&store, tab("A"), document, "old page".to_owned(), None);
        store.document_changed(&tab("A"));
        pending.settle(true, &|_: &str| true);
        assert_eq!(stored(&store, "A"), None);
    }

    #[test]
    fn tests_that_a_tab_keeps_only_its_latest_pending_texts() {
        let store = Arc::new(SnapshotBaselines::default());
        let pending = PendingBaselines::default();
        let document = store.generation(&tab("A"));
        for n in 0..=PENDING_PER_TAB {
            pending.record(&store, tab("A"), document, format!("text {n}"), None);
        }
        // The first was dropped to stay in bound: given alone, it commits nothing.
        pending.settle(true, &|returned: &str| returned == "text 0");
        assert_eq!(stored(&store, "A"), None);
    }
}
