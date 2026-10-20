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

/// What this run has withheld from the page in front of it.
///
/// Separate from [`Known`] and keyed differently on purpose. `Known` answers
/// questions about the page as the model was shown it, so it is keyed on the
/// full fingerprint and forgotten whenever any of that changes, including the
/// text. This answers "is this the same page I was just looking at", and keying
/// that on the text throws it away whenever a character changes: a commercial
/// page rewrites one every second, and ten snapshots of one home page a second
/// apart produced ten distinct fingerprints.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OnThisPage {
    pub arrangement: u64,
    /// Controls acted on here that changed nothing.
    ///
    /// The rule this replaces said a retry cannot clear whatever swallowed the
    /// first attempt, which is true of a dialog over the control and false of a
    /// page that has since moved on. Measured on the sibling implementation:
    /// the only control that submits a search was offered three times, clicked
    /// once while out of view, and never offered again across the nine
    /// requests that followed.
    pub retired: Vec<String>,
    /// Controls judged not worth acting on while this page has been showing.
    ///
    /// Withheld from the next question, which is what makes the next question
    /// different without moving the page under it.
    pub set_aside: Vec<String>,
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
    /// How many controls were retired across the whole run, for the outcome.
    /// What is withheld from a question lives on `known` and resets with the page.
    pub retired_total: usize,
    /// What has been withheld from the page in front of the run.
    pub here: OnThisPage,
    /// The control the last action aimed at, so an unmoved page knows what to
    /// retire.
    pub last_acted: Option<String>,
    /// The control the run is about to aim at for a second time running.
    ///
    /// Acting twice in a row on the same control is not progress. A live errand
    /// typed the caller's value into the same search box six times, seeing the
    /// box already hold that value on every look, so the value being visible is
    /// not enough on its own.
    /// What the last step aimed at: its reference, and what it was called.
    pub repeating: Option<(String, Option<String>)>,
    /// The last few controls this run aimed at, which it is told about rather
    /// than stopped for.
    pub recent_aims: Vec<(String, Option<String>)>,
    /// Every arrangement of a page this run has stood on.
    pub visited: std::collections::HashSet<u64>,
    /// Consecutive steps that left the page as it was.
    pub unchanged: u32,
    /// Consecutive steps that changed the page and got no closer.
    pub circling: u32,
    /// How many times this run has answered a refusal by searching.
    pub searches: u32,
    /// The best completion score this run has seen, so a step is only credited
    /// for improving on the run's high-water mark rather than on the last step.
    pub closest: f64,
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

/// How many steps may change the page without getting anywhere.
///
/// Higher than [`STALL_LIMIT`] because changing the page is weaker evidence of
/// a loop than failing to: a real errand opens a panel, reads it and closes it,
/// and all three steps are progress. Measured live, a run that was genuinely
/// circling did it three times over eleven steps before its budget ran out.
pub const CIRCLING_LIMIT: u32 = 4;

/// How many times one run may answer a refusal by reaching for a search.
///
/// Counted per run and not per page. The per-page flag lives beside the page
/// fingerprint and is forgotten the moment the page changes, and an active page
/// changes on every action: measured live, a run was refused three times and
/// reached for a search all three, which is not what "once per page" was meant
/// to mean. The flag still stops two attempts on a page standing still; this
/// stops a page that keeps moving re-arming it forever.
pub const SEARCH_LIMIT: u32 = 2;

/// How many of a run's recent aims are compared against a new one.
///
/// Four, which is the length of the cycle measured live: click the search
/// button, type in it, click the result, click the search button again.
pub const AIMS_REMEMBERED: usize = 4;
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
            retired_total: 0,
            here: OnThisPage::default(),
            last_acted: None,
            repeating: None,
            recent_aims: Vec::new(),
            visited: std::collections::HashSet::new(),
            unchanged: 0,
            circling: 0,
            searches: 0,
            closest: 0.0,
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

    /// Points what the run has withheld at the arrangement in front of it.
    ///
    /// Keyed coarsely on purpose: a deal countdown ticking once a second is
    /// not a different page, and discarding what was withheld every step is
    /// worse than having withheld nothing.
    pub fn standing_on(&mut self, arrangement: u64) {
        if self.here.arrangement == arrangement {
            return;
        }
        self.here = OnThisPage {
            arrangement,
            ..OnThisPage::default()
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
        if self.here.retired.contains(&reference) {
            return false;
        }
        self.here.retired.push(reference);
        self.retired_total += 1;
        true
    }

    /// Withholds a control the model judged not worth acting on, for as long as
    /// this page is showing.
    pub fn set_aside(&mut self, reference: impl Into<String>) {
        let reference = reference.into();
        if !self.here.set_aside.contains(&reference) {
            self.here.set_aside.push(reference);
        }
    }

    /// Offers again everything that was set aside on this page.
    ///
    /// Called when an action moved the page: a control judged not worth acting
    /// on was judged against a page that no longer exists. What was retired is
    /// left alone, because that is a claim about the control rather than about
    /// the moment.
    pub fn reconsider_set_aside(&mut self) {
        self.here.set_aside.clear();
    }

    /// Everything withheld from the next question about this page.
    #[must_use]
    pub fn withheld(&self) -> Vec<String> {
        let mut all = self.here.retired.clone();
        all.extend(self.here.set_aside.iter().cloned());
        all
    }

    /// Whether this is the second time running that the run would aim here.
    ///
    /// Records the aim either way, so the caller can act on the answer.
    ///
    /// A reference is not a control. A page that re-renders mints a new
    /// reference for the same control, and keying on the reference alone let a
    /// run type the same string into what it believed were three different
    /// search boxes. Measured live on a docs site: `e72`, then `e105`, then
    /// `e220`, all the same box, all "MCP tunnel", eleven actions and no
    /// progress.
    ///
    /// A name is not a control either. A results page carries sixty-five
    /// buttons called "Add to cart" and they are sixty-five different controls,
    /// so matching on the name alone would forbid a run from acting on the
    /// second of them.
    ///
    /// What tells the two apart is whether the control aimed at last time is
    /// still there. Gone, and an identically named one in its place, is a
    /// re-render. Still there, and a different one chosen, is a different
    /// control. `last_still_there` says which.
    pub fn aiming_again_at(
        &mut self,
        target: Option<&str>,
        name: Option<&str>,
        last_still_there: bool,
    ) -> bool {
        let Some(target) = target else {
            self.repeating = None;
            return false;
        };
        let again = self.repeating.as_ref().is_some_and(|(was, was_called)| {
            was == target
                || (!last_still_there
                    && name.is_some_and(|name| {
                        !name.trim().is_empty() && was_called.as_deref() == Some(name)
                    }))
        });
        self.repeating = Some((target.to_string(), name.map(str::to_string)));
        self.recent_aims
            .push((target.to_string(), name.map(str::to_string)));
        if self.recent_aims.len() > AIMS_REMEMBERED {
            self.recent_aims.remove(0);
        }
        again
    }

    /// Whether this run aimed at this control within its last few moves.
    ///
    /// Reported to the next decision rather than acted on, which is the
    /// difference between this and [`Run::aiming_again_at`]. Two moves in a row
    /// at one control is a run stuck on it, and setting the control aside is
    /// right. The same control four moves apart may be a menu opened twice on
    /// purpose, so this only says so, and the novelty of the page is what
    /// decides whether the run is going round in circles.
    #[must_use]
    pub fn aimed_here_recently(&self, target: Option<&str>) -> bool {
        let Some(target) = target else {
            return false;
        };
        // All but the aim just recorded, which is this one.
        let earlier = self.recent_aims.len().saturating_sub(1);
        self.recent_aims[..earlier]
            .iter()
            .any(|(was, _)| was == target)
    }

    /// Whether this run has stood on this arrangement of this page before.
    ///
    /// Records it either way. A step that lands somewhere already visited has
    /// returned rather than arrived, which is the difference a change test
    /// cannot see: on a lap of a loop everything changes every time.
    pub fn standing_somewhere_new(&mut self, fingerprint: u64) -> bool {
        self.visited.insert(fingerprint)
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

    /// The live defect this key exists for: a deal countdown ticking once a
    /// second changed the fingerprint, and with it everything the run had
    /// withheld from the page it was still looking at.
    #[test]
    fn a_ticking_clock_does_not_discard_what_was_withheld() {
        let mut run = Run::new("x", Budget::default());
        run.standing_on(42);
        run.last_acted = Some("e1".to_string());
        assert!(run.retire_last());
        run.set_aside("e2");

        // The same arrangement, a different fingerprint.
        run.now_looking_at(999);
        run.standing_on(42);
        assert_eq!(run.withheld(), vec!["e1".to_string(), "e2".to_string()]);

        // A different arrangement is a different page.
        run.standing_on(43);
        assert!(run.withheld().is_empty());
    }

    /// A control set aside was judged against a page that has since moved. A
    /// run that rejects Search while a box is empty, then fills the box, has
    /// to be able to reach Search again.
    #[test]
    fn a_control_set_aside_is_offered_again_once_the_page_moves() {
        let mut run = Run::new("x", Budget::default());
        run.standing_on(1);
        run.set_aside("e_search");
        run.last_acted = Some("e_box".to_string());
        assert!(run.retire_last());
        assert_eq!(run.withheld().len(), 2);

        run.reconsider_set_aside();
        assert_eq!(
            run.withheld(),
            vec!["e_box".to_string()],
            "what was retired stays, what was set aside is offered again"
        );
    }

    #[test]
    fn retiring_the_same_control_twice_only_counts_once() {
        let mut run = Run::new("x", Budget::default());
        run.last_acted = Some("e1".to_string());
        assert!(run.retire_last());
        run.last_acted = Some("e1".to_string());
        assert!(!run.retire_last(), "already retired");
        assert_eq!(run.here.retired.len(), 1);
        assert_eq!(run.retired_total, 1, "the run counts it once");
    }

    #[test]
    fn retiring_with_nothing_aimed_at_is_not_an_error() {
        let mut run = Run::new("x", Budget::default());
        assert!(!run.retire_last());
        assert!(run.here.retired.is_empty());
    }

    #[test]
    fn aiming_at_the_same_control_twice_running_is_noticed() {
        let mut run = Run::new("x", Budget::default());
        assert!(
            !run.aiming_again_at(Some("e1"), None, true),
            "the first time is fine"
        );
        assert!(
            run.aiming_again_at(Some("e1"), None, true),
            "the second is not"
        );
    }

    #[test]
    fn aiming_somewhere_else_in_between_clears_it() {
        let mut run = Run::new("x", Budget::default());
        run.aiming_again_at(Some("e1"), None, true);
        run.aiming_again_at(Some("e2"), None, true);
        assert!(
            !run.aiming_again_at(Some("e1"), None, true),
            "not twice in a row"
        );
    }

    #[test]
    fn a_move_with_no_target_clears_the_repeat() {
        let mut run = Run::new("x", Budget::default());
        run.aiming_again_at(Some("e1"), None, true);
        assert!(
            !run.aiming_again_at(None, None, true),
            "scrolling is not aiming"
        );
        assert!(!run.aiming_again_at(Some("e1"), None, true));
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

    /// The docs-site case. A page that re-renders mints a new reference for the
    /// same control, and keying on the reference alone let a run type the same
    /// string into what it thought were three different search boxes.
    #[test]
    fn a_control_that_came_back_with_a_new_reference_is_the_same_control() {
        let mut run = Run::new("search the docs", Budget::default());

        assert!(!run.aiming_again_at(Some("e72"), Some("Search"), true));
        // The dialog re-rendered: e72 is gone and an identically named box is
        // here as e105.
        assert!(
            run.aiming_again_at(Some("e105"), Some("Search"), false),
            "the box is the box, whatever it is called this time"
        );
    }

    /// And the other way. A results page carries sixty-five buttons called
    /// "Add to cart" and they are sixty-five different controls, so a run that
    /// acts on the second of them is not repeating itself.
    #[test]
    fn two_controls_that_share_a_name_are_two_controls() {
        let mut run = Run::new("add two things", Budget::default());

        assert!(!run.aiming_again_at(Some("e128"), Some("Add to cart"), true));
        assert!(
            !run.aiming_again_at(Some("e134"), Some("Add to cart"), true),
            "the first one is still on the page, so this is a different button"
        );
    }

    /// An unnamed control cannot be recognised by name, so the reference is all
    /// there is and the old rule stands.
    #[test]
    fn an_unnamed_control_is_matched_by_reference_alone() {
        let mut run = Run::new("click it", Budget::default());

        assert!(!run.aiming_again_at(Some("e1"), Some(""), true));
        assert!(!run.aiming_again_at(Some("e2"), Some(""), false));
        assert!(run.aiming_again_at(Some("e2"), Some(""), false));
    }
}
