//! Which operations get a screenshot (D7). Pure, so the rule is a table test.
//!
//! - When an operation ends: a write that targets a tab, or a failure on a tab that the website
//!   or the script caused. Both are moments a host wants to see the page.
//! - When a run ends: each tab the run touched whose last operation got none, so every tab the
//!   run acted on has a picture of how the run left it. The shot belongs to that operation, so
//!   it never shares a file with an operation's own shot.
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
/// operation got a screenshot job.
#[derive(Debug, Default)]
pub(crate) struct TouchedTabs(BTreeMap<TabId, LastOp>);

#[derive(Clone, Copy, Debug)]
struct LastOp {
    seq: u32,
    shot: bool,
}

impl TouchedTabs {
    /// Operation `seq` on `tab` was admitted: it is the tab's last operation now.
    pub(crate) fn admitted(&mut self, tab: TabId, seq: u32) {
        let last = self.0.entry(tab).or_insert(LastOp { seq, shot: false });
        if seq >= last.seq {
            *last = LastOp { seq, shot: false };
        }
    }

    /// Operation `seq` on `tab` got a screenshot job.
    pub(crate) fn shot(&mut self, tab: TabId, seq: u32) {
        if let Some(last) = self.0.get_mut(&tab)
            && last.seq == seq
        {
            last.shot = true;
        }
    }

    /// The run-end jobs, `(tab, seq)` in tab order: each touched tab whose last operation got
    /// none.
    pub(crate) fn run_end(&self) -> Vec<(TabId, u32)> {
        self.0
            .iter()
            .filter(|(_, last)| !last.shot)
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
        // A job for an operation that is no longer its tab's last does not cover the tab.
        tabs.admitted(TabId(6), 5);
        tabs.admitted(TabId(6), 6);
        tabs.shot(TabId(6), 5);
        assert_eq!(
            tabs.run_end(),
            vec![(TabId(3), 2), (TabId(5), 4), (TabId(6), 6)]
        );
    }

    #[test]
    fn a_late_admission_never_moves_a_tabs_last_operation_back() {
        let mut tabs = TouchedTabs::default();
        tabs.admitted(TabId(3), 5);
        tabs.admitted(TabId(3), 2);
        assert_eq!(tabs.run_end(), vec![(TabId(3), 5)]);
    }
}
