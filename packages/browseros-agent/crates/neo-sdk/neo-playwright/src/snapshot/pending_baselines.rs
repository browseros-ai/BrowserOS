//! One run's whole-page snapshot texts that may become their tabs' baselines when it ends.
//!
//! During the run a text is only pending: nothing compares against it, not even a later delta
//! of the same run, which reads the stored baseline instead. When the run ends its host settles
//! them ([`PendingBaselines::settle`]): if the run ended ok, each tab's last text whose returned
//! string the caller was given (in the run's value or its console) becomes that tab's baseline;
//! every other text is dropped. So each case errs the safe way: the next delta compares against
//! older text the caller does hold, or comes back whole.

use std::mem;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};

use browseros_core::TargetId;

use super::{Baseline, SnapshotBaselines, SnapshotForm};

/// At most this many pending texts per tab in one run; a run that snapshots one tab more often
/// keeps its latest. A text dropped here might have been the one the caller was given, so a tab
/// that dropped one and kept none it gave loses its stored baseline: the bound costs at most one
/// full snapshot, never a delta against text the caller has moved past. 16 of the default
/// 32,000-character texts is half a megabyte.
const PENDING_PER_TAB: usize = 16;

/// One run's pending texts, in the order the run took them. Owned by the run's `RunState`, so
/// whatever the host does not settle goes with the run.
#[derive(Debug, Default)]
pub(crate) struct PendingBaselines {
    inner: Mutex<Inner>,
    /// Where a settled text goes: the `Playwright` facade's store, set by the first op that
    /// records a text.
    store: OnceLock<Arc<SnapshotBaselines>>,
}

#[derive(Debug, Default)]
struct Inner {
    pending: Vec<Pending>,
    /// The tabs that dropped a pending text to stay in bound, each with that text's document.
    evicted: Vec<(TargetId, u32)>,
}

#[derive(Debug)]
struct Pending {
    tab: TargetId,
    /// The tab's document generation the snapshot read before its tree.
    document: u32,
    /// The full v1 text: what becomes the baseline.
    text: String,
    /// How the prelude rendered `text`.
    form: SnapshotForm,
    /// What the call returned when it was not `text` itself: a delta call's delta.
    returned: Option<String>,
}

impl PendingBaselines {
    /// Records one whole-page snapshot of `tab` taken by this run: its full text in `form`, and
    /// what the call returned when that was not the text itself.
    pub(crate) fn record(
        &self,
        store: &Arc<SnapshotBaselines>,
        tab: TargetId,
        document: u32,
        text: String,
        form: SnapshotForm,
        returned: Option<String>,
    ) {
        let _ = self.store.set(Arc::clone(store));
        let mut inner = self.lock();
        let Inner { pending, evicted } = &mut *inner;
        if pending.iter().filter(|entry| entry.tab == tab).count() >= PENDING_PER_TAB
            && let Some(oldest) = pending.iter().position(|entry| entry.tab == tab)
        {
            let dropped = pending.remove(oldest);
            evicted.retain(|(evicted_tab, _)| *evicted_tab != tab);
            evicted.push((tab.clone(), dropped.document));
        }
        pending.push(Pending {
            tab,
            document,
            text,
            form,
            returned,
        });
    }

    /// Ends the run's pending texts. With `ended_ok`, each tab's last text whose returned string
    /// `given` finds in the reply is committed; everything else is dropped. Takes them, so a
    /// host that runs several scripts on one state settles each run's own.
    pub(crate) fn settle(&self, ended_ok: bool, given: &dyn Fn(&str) -> bool) {
        let Inner { pending, evicted } = mem::take(&mut *self.lock());
        let Some(store) = self.store.get() else {
            return;
        };
        if !ended_ok {
            return;
        }
        let mut committed: Vec<TargetId> = Vec::new();
        for entry in pending.into_iter().rev() {
            if committed.contains(&entry.tab) {
                continue;
            }
            if given(entry.returned.as_deref().unwrap_or(&entry.text)) {
                let baseline = Baseline {
                    text: entry.text,
                    form: entry.form,
                };
                store.commit(&entry.tab, entry.document, baseline);
                committed.push(entry.tab);
            }
        }
        for (tab, document) in evicted {
            if !committed.contains(&tab) {
                store.forget(&tab, document);
            }
        }
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use browseros_core::TargetId;

    use super::{PENDING_PER_TAB, PendingBaselines};
    use crate::snapshot::{SnapshotBaselines, SnapshotForm, TextForm};

    /// The form of a default snapshot that kept its text.
    const KEPT: SnapshotForm = SnapshotForm {
        text: TextForm::Kept,
        urls: false,
        max_tokens: 8000.0,
    };

    fn tab(id: &str) -> TargetId {
        TargetId(id.to_owned())
    }

    fn stored(store: &SnapshotBaselines, id: &str) -> Option<String> {
        store
            .stored(&tab(id), store.generation(&tab(id)))
            .map(|baseline| baseline.text)
    }

    /// One whole-page snapshot a test run takes.
    struct Taken<'a> {
        tab: &'a str,
        /// The full v1 text.
        text: &'a str,
        /// What the call returned, when not the text itself.
        returned: Option<&'a str>,
    }

    fn full<'a>(tab: &'a str, text: &'a str) -> Taken<'a> {
        Taken {
            tab,
            text,
            returned: None,
        }
    }

    /// One run on `store` that takes `taken` and hands the caller `reply`.
    fn run(store: &Arc<SnapshotBaselines>, taken: &[Taken<'_>], ended_ok: bool, reply: &str) {
        let pending = PendingBaselines::default();
        for snapshot in taken {
            let document = store.generation(&tab(snapshot.tab));
            pending.record(
                store,
                tab(snapshot.tab),
                document,
                snapshot.text.to_owned(),
                KEPT,
                snapshot.returned.map(str::to_owned),
            );
        }
        pending.settle(ended_ok, &|returned: &str| reply.contains(returned));
    }

    #[test]
    fn tests_that_a_text_the_caller_was_given_becomes_the_baseline() {
        let store = Arc::new(SnapshotBaselines::default());
        run(&store, &[full("A", "page one")], true, "\"page one\"");
        assert_eq!(stored(&store, "A").as_deref(), Some("page one"));
        // A delta call: its delta is what the caller got, its full text what is stored.
        let delta = Taken {
            tab: "A",
            text: "page two",
            returned: Some("+ two"),
        };
        run(&store, &[delta], true, "[\"+ two\"]");
        assert_eq!(stored(&store, "A").as_deref(), Some("page two"));
    }

    #[test]
    fn tests_that_a_failed_run_or_a_text_the_caller_never_got_is_dropped() {
        let store = Arc::new(SnapshotBaselines::default());
        run(&store, &[full("A", "seen")], true, "seen");
        run(&store, &[full("A", "thrown")], false, "thrown");
        assert_eq!(
            stored(&store, "A").as_deref(),
            Some("seen"),
            "the run failed"
        );
        run(&store, &[full("A", "inspected")], true, "9");
        assert_eq!(
            stored(&store, "A").as_deref(),
            Some("seen"),
            "only its length came back"
        );
        run(&store, &[full("A", "a long page")], true, "a long");
        assert_eq!(
            stored(&store, "A").as_deref(),
            Some("seen"),
            "a slice is not the text"
        );
        let delta = Taken {
            tab: "A",
            text: "x",
            returned: Some("+ x"),
        };
        run(&store, &[delta], true, "x");
        assert_eq!(
            stored(&store, "A").as_deref(),
            Some("seen"),
            "a delta call's own string is what must come back"
        );
    }

    #[test]
    fn tests_that_the_last_text_given_per_tab_wins() {
        let store = Arc::new(SnapshotBaselines::default());
        let taken = [
            full("A", "a1"),
            full("B", "b1"),
            full("A", "a2"),
            full("A", "a3"),
        ];
        run(&store, &taken, true, "a1 a2 b1");
        assert_eq!(
            stored(&store, "A").as_deref(),
            Some("a2"),
            "a3 was not given"
        );
        assert_eq!(stored(&store, "B").as_deref(), Some("b1"));
    }

    #[test]
    fn tests_that_the_committed_baseline_keeps_the_form_its_text_was_recorded_in() {
        let store = Arc::new(SnapshotBaselines::default());
        let never = SnapshotForm {
            text: TextForm::Never,
            ..KEPT
        };
        for form in [never, KEPT] {
            let pending = PendingBaselines::default();
            let document = store.generation(&tab("A"));
            pending.record(&store, tab("A"), document, "page".to_owned(), form, None);
            pending.settle(true, &|returned: &str| returned == "page");
            let stored = store.stored(&tab("A"), document);
            assert_eq!(stored.map(|baseline| baseline.form), Some(form));
        }
    }

    #[test]
    fn tests_that_a_document_change_during_the_run_drops_its_pending_text() {
        let store = Arc::new(SnapshotBaselines::default());
        let pending = PendingBaselines::default();
        let document = store.generation(&tab("A"));
        pending.record(
            &store,
            tab("A"),
            document,
            "old page".to_owned(),
            KEPT,
            None,
        );
        store.document_changed(&tab("A"));
        pending.settle(true, &|_: &str| true);
        assert_eq!(stored(&store, "A"), None);
    }

    #[test]
    fn tests_that_a_tab_past_the_bound_never_keeps_a_baseline_older_than_what_it_gave() {
        let store = Arc::new(SnapshotBaselines::default());
        run(&store, &[full("A", "older")], true, "older");
        let texts: Vec<String> = (0..=PENDING_PER_TAB).map(|n| format!("text {n}")).collect();
        let taken: Vec<Taken<'_>> = texts.iter().map(|text| full("A", text)).collect();
        // Only the first, which the bound dropped, was given: "older" must not stay.
        run(&store, &taken, true, "text 0 ");
        assert_eq!(stored(&store, "A"), None);
        // A later text that was given still wins.
        run(&store, &[full("A", "older")], true, "older");
        run(&store, &taken, true, "text 16");
        assert_eq!(stored(&store, "A").as_deref(), Some("text 16"));
    }
}
