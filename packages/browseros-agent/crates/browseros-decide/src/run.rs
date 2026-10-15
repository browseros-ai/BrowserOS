//! What one run knows about itself.
//!
//! A value rather than a field on the loop, so an ending can be built from it
//! and so a test can set a run up part-way through without running the steps
//! that would have got it there.

use crate::offer::{Destination, NamedValue, OpenTab};

/// A target holding this much of the answer stops the search of a page.
///
/// Pagination makes every control reachable, which also means a run can pay for
/// every batch of a page it has already answered. On one measured page, 7 of the
/// 8 batches asked were a re-search of a page whose answer was found in the
/// first one.
///
/// A share rather than a margin, because this is not a gate: the question is
/// whether anything further down the page could plausibly beat what is already
/// in hand, and a control holding three quarters of the probability has
/// answered that.
pub const SETTLED_SHARE: f64 = 0.75;

/// How long a run may go on for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Budget {
    pub max_steps: u32,
    pub max_seconds: u64,
}

impl Default for Budget {
    fn default() -> Self {
        Self {
            max_steps: 12,
            max_seconds: 120,
        }
    }
}

/// What the caller put within reach before the run started.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Offered {
    pub destinations: Vec<Destination>,
    pub values: Vec<NamedValue>,
    pub tabs: Vec<OpenTab>,
    /// Moves this browser cannot perform, so they are never offered.
    pub unavailable: Vec<crate::vocabulary::Operation>,
}

/// The best target found for one arrangement of a page.
#[derive(Debug, Clone, PartialEq)]
pub struct Found {
    pub reference: String,
    /// What share of the answer it held, for comparing one batch against
    /// another.
    pub share: f64,
    /// Which batch it was found in, so a report can say where it came from.
    pub batch: usize,
}

/// What this run already learned about the page it is standing on.
///
/// Keyed on the page fingerprint and discarded the moment that changes, because
/// every statement in here is about one arrangement of controls. A page that
/// moved is a different page and has to be searched again.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Known {
    pub fingerprint: u64,
    /// The best target this page offered, across every batch asked so far.
    pub target: Option<Found>,
    /// Every batch of this page was offered and none of them settled it.
    ///
    /// Stops the next decision on the same page from paying for the same search
    /// twice. Note what it does not do: a target found earlier stays in
    /// `target`, so an exhausted page is not an empty one.
    pub exhausted: bool,
    /// Whether this run has already reached for this page's own search.
    ///
    /// Kept beside the fingerprint so it is forgotten the moment the page
    /// changes, which is what stops a run searching the same page twice and
    /// what lets it search the page the search took it to.
    pub searched: bool,
}

impl Known {
    /// Records a target, keeping whichever of the two held more of the answer.
    pub fn found(&mut self, reference: &str, share: f64, batch: usize) {
        if self
            .target
            .as_ref()
            .is_some_and(|already| already.share >= share)
        {
            return;
        }
        self.target = Some(Found {
            reference: reference.to_string(),
            share,
            batch,
        });
    }

    /// Whether what is already in hand is good enough to stop looking.
    #[must_use]
    pub fn settled(&self) -> bool {
        self.target
            .as_ref()
            .is_some_and(|found| found.share >= SETTLED_SHARE)
    }
}

/// One run, from the goal to whatever ended it.
#[derive(Debug, Clone)]
pub struct Run {
    pub goal: String,
    pub budget: Budget,
    pub offered: Offered,
    /// Steps taken, which is not the same as decisions asked for: a declined
    /// decision costs a decision and no step.
    pub spent: u32,
    pub decisions: u32,
    pub input_tokens: u64,
    /// What the run has done, in words the next decision can read.
    pub recent: Vec<String>,
    /// Every decision this run paid for, and what the gate made of each.
    pub decided: Vec<crate::pursue::Decided>,
    /// Controls that were acted on and changed nothing.
    ///
    /// A retry cannot clear whatever swallowed the first attempt, so these are
    /// never offered again. Without it a run spends its whole budget choosing
    /// one dead reference, which is exactly what a live errand did.
    pub retired: Vec<String>,
    /// The control the last action aimed at, so an unmoved page knows what to
    /// retire.
    pub last_acted: Option<String>,
    /// The control the run is about to aim at for a second time running.
    ///
    /// Acting twice in a row on the same control is not progress. A live errand
    /// typed the caller's value into the same search box six times, seeing the
    /// box already hold that value on every look, so the value being visible is
    /// not enough on its own.
    pub repeating: Option<String>,
    /// Consecutive steps that left the page as it was.
    pub unchanged: u32,
    /// Consecutive decisions the gate declined.
    pub declined: u32,
    /// How many times the run has moved between pages.
    pub navigations: u32,
    /// How many times it has chosen to wait.
    pub waits: u32,
    /// Whether the run has moved, and so has somewhere to go back to.
    pub travelled: bool,
    pub url_before: Option<String>,
    /// What the run learned about the page it is on. See [`Known`].
    pub known: Known,
    /// Target questions asked, across every batch of every page.
    ///
    /// Counted separately from `decisions` because one decision can cost
    /// several of these, and a run that pays for eight batches to answer one
    /// question should say so.
    pub batches_asked: u32,
}

/// How many steps in a row may leave the page unchanged before the run stops.
pub const STALL_LIMIT: u32 = 3;
/// How many decisions in a row the gate may decline before the run stops.
pub const DECLINE_LIMIT: u32 = 3;
/// How many times one run may move between pages.
pub const NAVIGATION_LIMIT: u32 = 3;
/// How many times one run may choose to wait.
pub const WAIT_LIMIT: u32 = 3;

impl Run {
    #[must_use]
    pub fn new(goal: impl Into<String>, budget: Budget) -> Self {
        Self {
            goal: goal.into(),
            budget,
            offered: Offered::default(),
            spent: 0,
            decisions: 0,
            input_tokens: 0,
            recent: Vec::new(),
            decided: Vec::new(),
            retired: Vec::new(),
            last_acted: None,
            repeating: None,
            unchanged: 0,
            declined: 0,
            navigations: 0,
            waits: 0,
            travelled: false,
            url_before: None,
            known: Known::default(),
            batches_asked: 0,
        }
    }

    /// Points the run's memory at the page in front of it.
    ///
    /// Forgets everything if the arrangement changed, which is the whole
    /// discipline: memory of a page that has moved is worse than none, because
    /// it names controls by a reference the new page may have given to
    /// something else.
    pub fn now_looking_at(&mut self, fingerprint: u64) {
        if self.known.fingerprint == fingerprint {
            return;
        }
        self.known = Known {
            fingerprint,
            ..Known::default()
        };
    }

    #[must_use]
    pub fn offering(mut self, offered: Offered) -> Self {
        self.offered = offered;
        self
    }

    /// Retires whatever the last action aimed at.
    ///
    /// Returns whether anything was retired, so the loop can tell a dead control
    /// from a page that genuinely does not change when acted on.
    pub fn retire_last(&mut self) -> bool {
        let Some(reference) = self.last_acted.take() else {
            return false;
        };
        if self.retired.contains(&reference) {
            return false;
        }
        self.retired.push(reference);
        true
    }

    /// Whether this is the second time running that the run would aim here.
    ///
    /// Records the aim either way, so the caller can act on the answer.
    pub fn aiming_again_at(&mut self, target: Option<&str>) -> bool {
        let Some(target) = target else {
            self.repeating = None;
            return false;
        };
        let again = self.repeating.as_deref() == Some(target);
        self.repeating = Some(target.to_string());
        again
    }

    /// Records a decision and what the gate made of it.
    ///
    /// Kept on the run rather than threaded through the loop, so every one of
    /// the fifteen ways a run can end carries the record without knowing it
    /// exists.
    pub fn judged(&mut self, decided: crate::pursue::Decided) {
        self.decided.push(decided);
    }

    /// Records something the run did, for the next decision to read.
    pub fn remember(&mut self, what: impl Into<String>) {
        const KEEP: usize = 6;
        self.recent.push(what.into());
        if self.recent.len() > KEEP {
            self.recent.remove(0);
        }
    }

    #[must_use]
    pub fn out_of_steps(&self) -> bool {
        self.spent >= self.budget.max_steps
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retiring_the_same_control_twice_only_counts_once() {
        let mut run = Run::new("x", Budget::default());
        run.last_acted = Some("e1".to_string());
        assert!(run.retire_last());
        run.last_acted = Some("e1".to_string());
        assert!(!run.retire_last(), "already retired");
        assert_eq!(run.retired.len(), 1);
    }

    #[test]
    fn retiring_with_nothing_aimed_at_is_not_an_error() {
        let mut run = Run::new("x", Budget::default());
        assert!(!run.retire_last());
        assert!(run.retired.is_empty());
    }

    #[test]
    fn aiming_at_the_same_control_twice_running_is_noticed() {
        let mut run = Run::new("x", Budget::default());
        assert!(!run.aiming_again_at(Some("e1")), "the first time is fine");
        assert!(run.aiming_again_at(Some("e1")), "the second is not");
    }

    #[test]
    fn aiming_somewhere_else_in_between_clears_it() {
        let mut run = Run::new("x", Budget::default());
        run.aiming_again_at(Some("e1"));
        run.aiming_again_at(Some("e2"));
        assert!(!run.aiming_again_at(Some("e1")), "not twice in a row");
    }

    #[test]
    fn a_move_with_no_target_clears_the_repeat() {
        let mut run = Run::new("x", Budget::default());
        run.aiming_again_at(Some("e1"));
        assert!(!run.aiming_again_at(None), "scrolling is not aiming");
        assert!(!run.aiming_again_at(Some("e1")));
    }

    #[test]
    fn what_the_run_remembers_is_bounded() {
        let mut run = Run::new("x", Budget::default());
        for index in 0..20 {
            run.remember(format!("did {index}"));
        }
        assert_eq!(run.recent.len(), 6);
        assert_eq!(run.recent.last().unwrap(), "did 19", "the newest is kept");
    }

    #[test]
    fn a_page_that_moved_is_searched_again_from_scratch() {
        let mut run = Run::new("x", Budget::default());
        run.now_looking_at(7);
        run.known.found("e1", 0.90, 0);
        run.known.exhausted = true;

        run.now_looking_at(7);
        assert!(
            run.known.target.is_some(),
            "the same page keeps what it knows"
        );

        run.now_looking_at(8);
        assert_eq!(
            run.known,
            Known {
                fingerprint: 8,
                target: None,
                exhausted: false,
                searched: false,
            }
        );
    }

    /// The better of two batches wins, so searching further never loses what
    /// was already in hand.
    #[test]
    fn the_best_target_across_batches_is_the_one_kept() {
        let mut known = Known::default();
        known.found("e9", 0.41, 0);
        known.found("e300", 0.88, 1);
        known.found("e12", 0.52, 2);
        let found = known.target.expect("something was found");
        assert_eq!(found.reference, "e300");
        assert_eq!(found.batch, 1);
    }

    #[test]
    fn a_target_holding_most_of_the_answer_stops_the_search() {
        let mut known = Known::default();
        known.found("e1", 0.74, 0);
        assert!(!known.settled(), "just under");
        known.found("e2", SETTLED_SHARE, 1);
        assert!(known.settled());
    }

    /// An exhausted page is not an empty one. The search stops; what it found
    /// stays.
    #[test]
    fn exhausting_a_page_does_not_discard_what_it_found() {
        let mut run = Run::new("x", Budget::default());
        run.now_looking_at(3);
        run.known.found("e5", 0.40, 0);
        run.known.exhausted = true;
        assert_eq!(
            run.known.target.as_ref().map(|f| f.reference.as_str()),
            Some("e5")
        );
    }
}
