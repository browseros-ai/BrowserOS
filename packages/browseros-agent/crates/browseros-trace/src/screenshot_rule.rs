//! Which operations get a screenshot (D7). Pure, so the rule is a table test.
//!
//! - When an operation ends: a write that targets a tab, or a failure on a tab that the website
//!   or the script caused. Both are moments a host wants to see the page.
//! - When a run ends: each tab the run touched whose last operation got none, so every tab the
//!   run acted on has a picture of how the run left it. The shot belongs to that operation, so
//!   it never shares a file with an operation's own shot.
//! - Except a tab with one of the run's jobs still out (decision 37). That job is answered only
//!   after the run's end reached the writer, so its picture shows the tab as the run left it,
//!   and a second capture would only repeat it. The run-end shot folds into it.
//!
//! Switching to the `run_end` mode (R5) is a change to this file alone.

use crate::{Effect, ErrorOwner, ErrorRecord, Outcome};
use browseros_core::TabId;
use std::collections::BTreeMap;

/// The tab to capture once an operation ended, or `None`.
pub(crate) fn after_op(
    effect: Effect,
    tab: Option<TabId>,
    outcome: Outcome,
    error: Option<&ErrorRecord>,
) -> Option<TabId> {
    let page_failure = outcome == Outcome::Error
        && error
            .is_some_and(|error| matches!(error.owner, ErrorOwner::Website | ErrorOwner::Script));
    tab.filter(|_| effect == Effect::Write || page_failure)
}

/// The tabs one run touched, each with its last operation (by admission order) and whether that
/// operation got a screenshot job; and the run's jobs still out.
#[derive(Debug, Default)]
pub(crate) struct TouchedTabs {
    last: BTreeMap<TabId, LastOp>,
    /// Each job still out: its operation's seq, and its tab.
    out: BTreeMap<u32, TabId>,
}

#[derive(Clone, Copy, Debug)]
struct LastOp {
    seq: u32,
    shot: bool,
}

impl TouchedTabs {
    /// Operation `seq` on `tab` was admitted: it is the tab's last operation now.
    pub(crate) fn admitted(&mut self, tab: TabId, seq: u32) {
        let last = self.last.entry(tab).or_insert(LastOp { seq, shot: false });
        if seq >= last.seq {
            *last = LastOp { seq, shot: false };
        }
    }

    /// Operation `seq` on `tab` got a screenshot job, which is out until it is answered.
    pub(crate) fn shot(&mut self, tab: TabId, seq: u32) {
        self.out.insert(seq, tab);
        if let Some(last) = self.last.get_mut(&tab)
            && last.seq == seq
        {
            last.shot = true;
        }
    }

    /// Operation `seq`'s job was answered, whatever the answer.
    pub(crate) fn answered(&mut self, seq: u32) {
        self.out.remove(&seq);
    }

    /// The run-end jobs, `(tab, seq)` in tab order: each touched tab whose last operation got
    /// none and that has no job out.
    pub(crate) fn run_end(&self) -> Vec<(TabId, u32)> {
        self.last
            .iter()
            .filter(|(tab, last)| !last.shot && !self.out.values().any(|out| out == *tab))
            .map(|(tab, last)| (*tab, last.seq))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::{TouchedTabs, after_op};
    use crate::{Effect, ErrorOwner, ErrorRecord, Outcome};
    use browseros_core::TabId;

    fn failure(owner: ErrorOwner) -> ErrorRecord {
        ErrorRecord {
            owner,
            code: "x".into(),
            message: "x".into(),
            host_code: None,
            retryable: false,
            operation_id: None,
        }
    }

    #[test]
    fn writes_and_page_failures_on_a_tab_get_a_screenshot() {
        let tab = Some(TabId(3));
        let cases = [
            (Effect::Write, tab, Outcome::Ok, None, true),
            (Effect::Write, tab, Outcome::Cancelled, None, true),
            (
                Effect::Write,
                tab,
                Outcome::Error,
                Some(ErrorOwner::Host),
                true,
            ),
            (Effect::Write, None, Outcome::Ok, None, false),
            (Effect::Read, tab, Outcome::Ok, None, false),
            (
                Effect::Read,
                tab,
                Outcome::Error,
                Some(ErrorOwner::Website),
                true,
            ),
            (
                Effect::Read,
                tab,
                Outcome::Error,
                Some(ErrorOwner::Script),
                true,
            ),
            (
                Effect::Read,
                tab,
                Outcome::Error,
                Some(ErrorOwner::Host),
                false,
            ),
            (
                Effect::Read,
                tab,
                Outcome::Error,
                Some(ErrorOwner::Neo),
                false,
            ),
            (Effect::Read, tab, Outcome::Error, None, false),
            (
                Effect::Read,
                tab,
                Outcome::Cancelled,
                Some(ErrorOwner::Website),
                false,
            ),
            (
                Effect::Read,
                None,
                Outcome::Error,
                Some(ErrorOwner::Website),
                false,
            ),
        ];
        for (effect, tab, outcome, owner, shot) in cases {
            let error = owner.map(failure);
            assert_eq!(
                after_op(effect, tab, outcome, error.as_ref()),
                tab.filter(|_| shot),
                "{effect:?} {tab:?} {outcome:?} {owner:?}"
            );
        }
    }

    #[test]
    fn a_run_ends_with_one_shot_per_touched_tab_whose_last_operation_got_none() {
        let mut tabs = TouchedTabs::default();
        tabs.admitted(TabId(3), 1);
        tabs.shot(TabId(3), 1);
        tabs.admitted(TabId(3), 2);
        tabs.admitted(TabId(4), 3);
        tabs.shot(TabId(4), 3);
        tabs.admitted(TabId(5), 4);
        // An answered job for an operation that is no longer its tab's last does not cover the
        // tab: its picture was taken before the run ended.
        tabs.admitted(TabId(6), 5);
        tabs.admitted(TabId(6), 6);
        tabs.shot(TabId(6), 5);
        for seq in [1, 3, 5] {
            tabs.answered(seq);
        }
        assert_eq!(
            tabs.run_end(),
            vec![(TabId(3), 2), (TabId(5), 4), (TabId(6), 6)]
        );
    }

    #[test]
    fn a_run_end_shot_folds_into_a_job_still_out_on_its_tab() {
        let mut tabs = TouchedTabs::default();
        // A write whose job is out, then a read.
        tabs.admitted(TabId(3), 1);
        tabs.shot(TabId(3), 1);
        tabs.admitted(TabId(3), 2);
        // The same, with the job answered before the run ended.
        tabs.admitted(TabId(4), 3);
        tabs.shot(TabId(4), 3);
        tabs.admitted(TabId(4), 4);
        tabs.answered(3);
        // Two jobs, the older one still out.
        tabs.admitted(TabId(5), 5);
        tabs.shot(TabId(5), 5);
        tabs.admitted(TabId(5), 6);
        tabs.shot(TabId(5), 6);
        tabs.admitted(TabId(5), 7);
        tabs.answered(6);
        // A tab only read, beside the tabs with jobs out.
        tabs.admitted(TabId(6), 8);
        assert_eq!(tabs.run_end(), vec![(TabId(4), 4), (TabId(6), 8)]);
    }

    #[test]
    fn a_late_admission_never_moves_a_tabs_last_operation_back() {
        let mut tabs = TouchedTabs::default();
        tabs.admitted(TabId(3), 5);
        tabs.admitted(TabId(3), 2);
        assert_eq!(tabs.run_end(), vec![(TabId(3), 5)]);
    }
}
