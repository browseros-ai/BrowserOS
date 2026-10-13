//! What one run knows about itself.
//!
//! A value rather than a field on the loop, so an ending can be built from it
//! and so a test can set a run up part-way through without running the steps
//! that would have got it there.

use crate::offer::{Destination, NamedValue, OpenTab};

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
            retired: Vec::new(),
            last_acted: None,
            repeating: None,
            unchanged: 0,
            declined: 0,
            navigations: 0,
            waits: 0,
            travelled: false,
            url_before: None,
        }
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
}
