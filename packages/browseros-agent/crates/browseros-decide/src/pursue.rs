//! One goal, one page, until something ends it.
//!
//! The loop observes, asks once, judges the answer, runs it, and observes again.
//! Whether the page moved is decided by looking rather than by asking the tool
//! that moved it, which is authoritative and needs no agreement with another
//! component about how it words a result.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use crate::client::{Jev, JevError, Question};
use crate::drive::{Driver, Journey, Move};
use crate::gate::{self, Decision, Handback, Verdict};
use crate::kind::Kind;
use crate::offer::Destination;
use crate::questions::{self, Surface};
use crate::run::Known;
use crate::run::{Budget, DECLINE_LIMIT, NAVIGATION_LIMIT, Run, STALL_LIMIT, WAIT_LIMIT};
use crate::space::ActionSpace;
use crate::space::Candidate;
use crate::view::{Moved, PageView};
use crate::vocabulary::{How, Operation};

/// Where a decision comes from.
///
/// A trait so the loop can be driven by a scripted answer in a test, which is
/// what lets every behaviour below be checked without a network or a browser.
pub trait Oracle {
    /// Asks, giving up after `budget`, which covers the whole call.
    fn ask_within(
        &self,
        state: &serde_json::Value,
        questions: &BTreeMap<String, Question>,
        budget: Duration,
    ) -> impl Future<Output = Result<crate::client::Response, JevError>>;
}

impl Oracle for Jev {
    async fn ask_within(
        &self,
        state: &serde_json::Value,
        questions: &BTreeMap<String, Question>,
        budget: Duration,
    ) -> Result<crate::client::Response, JevError> {
        self.ask_within(state, questions, budget).await
    }
}

/// Why a run stopped making progress.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stuck {
    /// Actions ran and left the page as it was.
    NothingChanged,
    /// The gate declined to act, usually because confidence was short.
    NotConfident,
    /// The browser kept reporting something a retry might clear, and it did not
    /// clear.
    BrowserKeptFailing,
    /// The run kept choosing to wait, which makes no observable progress.
    OnlyWaiting,
    /// Actions ran, the page changed every time, and nothing got closer.
    ///
    /// Distinct from [`Stuck::NothingChanged`] because the page changing is
    /// exactly what made this invisible. Measured live: eleven actions, eleven
    /// changes, nothing retired, and the run opened the same search dialog
    /// three times before its budget ran out.
    GoingInCircles,
}

impl Stuck {
    /// What to tell the caller, which has to be true of what happened.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NothingChanged => "nothing_changed",
            Self::NotConfident => "not_confident",
            Self::BrowserKeptFailing => "browser_kept_failing",
            Self::OnlyWaiting => "only_waiting",
            Self::GoingInCircles => "going_in_circles",
        }
    }
}

/// How a run ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    /// The model said every requirement was met, on a page that had not moved
    /// since it looked.
    Satisfied,
    /// Nothing offered could advance the goal.
    Blocked(String),
    /// The run stopped making progress.
    Stalled(Stuck),
    /// The run went back to the caller, with the reason it did.
    NeedsInput(Handback, String),
    /// The step or time budget ran out.
    BudgetSpent,
    /// Something outside the run stopped it: the caller, the session, or the
    /// browser going away.
    ///
    /// Its own ending rather than folded into the budget, because the two need
    /// opposite responses and reading one as the other cost a day. A run that
    /// was cancelled after no steps reported "ran out of steps before the goal
    /// was met", which is not true of anything and should have been impossible
    /// to say.
    Cancelled,
}

impl Status {
    /// A short word a caller can branch on.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Satisfied => "satisfied",
            Self::Blocked(_) => "blocked",
            Self::Stalled(stuck) => stuck.as_str(),
            Self::NeedsInput(handback, _) => handback.as_str(),
            Self::BudgetSpent => "budget_spent",
            Self::Cancelled => "cancelled",
        }
    }
}

/// One decision, as the caller needs to see it.
///
/// Separate from [`Step`] because a step is something that ran and a decision
/// is something that was paid for. A declined decision produces no step, which
/// is why a run could report three decisions and no actions and leave no trace
/// of what it tried three times.
///
/// The gate already computes every field here. Until now it composed the one
/// sentence that explains a decline and the loop discarded it, so three live
/// failures in a row had to be diagnosed by reading source.
#[derive(Debug, Clone, PartialEq)]
pub struct Decided {
    pub operation: &'static str,
    pub how: Option<&'static str>,
    pub target: Option<String>,
    /// What the chosen control is called, which is what a caller can act on.
    pub target_name: Option<String>,
    /// `ran`, `explored` or `handed_back`.
    pub verdict: &'static str,
    /// Which judgement was weakest, in the gate's own words.
    pub weakest: Option<&'static str>,
    pub margin: Option<f64>,
    /// The margin this decision had to clear, which its consequence set.
    ///
    /// Reported beside the margin because a margin alone says nothing: 0.23 is
    /// comfortable for a report and refused for a submit. A share is reported
    /// too and is the least useful of the three, because it is not comparable
    /// between a question offering eleven options and one offering two hundred
    /// and fifty four, which is why the gate reads the margin instead.
    pub needed: Option<f64>,
    pub share: Option<f64>,
    /// The gate's sentence, where it had one.
    pub why: Option<String>,
    /// Which part of the page this decision was asked about, counting from one.
    pub batch: usize,
    pub input_tokens: u64,
}

/// One thing the run did.
#[derive(Debug, Clone, PartialEq)]
pub struct Step {
    pub operation: Operation,
    pub how: Option<How>,
    pub target: Option<String>,
    pub value: Option<String>,
    pub confidence: f64,
    pub page_changed: bool,
    pub url_after: String,
    /// What observably moved on the page, as fact rather than as a claim.
    ///
    /// The loop cannot know an action succeeded. It can say that it clicked a
    /// control called "Add to cart" and that a control called "0 items in cart"
    /// is now called "2 items in cart", which is checkable and is what a caller
    /// asking whether the errand worked actually wants.
    pub moved: Vec<Moved>,
    pub input_tokens: u64,
}

/// What a run did, stated so a caller can check it rather than trust it.
#[derive(Debug, Clone, PartialEq)]
pub struct Outcome {
    pub status: Status,
    /// How many controls the last decision could choose from, and how many were
    /// held back by the cap. Reported on every ending, because how much of the
    /// page was visible to a decision is part of judging its answer.
    pub offered: usize,
    pub omitted: usize,
    /// How many controls were retired for changing nothing when acted on.
    pub retired: usize,
    /// Every decision asked for, including those that ended before acting.
    pub decisions: u32,
    /// Questions asked about a part of a page.
    ///
    /// Counted apart from `decisions` because one look at a page larger than
    /// one question costs several of these. A run paying for eight of them to
    /// answer one question should say so rather than report one decision and
    /// eight times the tokens.
    pub batches: u32,
    pub input_tokens: u64,
    pub trail: Vec<Step>,
    /// Every decision, including the ones that never ran.
    pub decided: Vec<Decided>,
    pub url_before: String,
    pub url_after: String,
    pub title: String,
    /// The page the run ended on, as the caller can act on it.
    ///
    /// The one thing the previous attempt did not return, and the reason a
    /// caller could do nothing with a failure but reword its goal and hope. A
    /// caller holding control names has something concrete to change.
    pub ended_on: Option<PageView>,
    /// How likely the last decision thought the goal was already satisfied.
    ///
    /// Reported, never acted on. A completion claim is not evidence.
    pub goal_met: Option<f64>,
    /// Images on the page carrying no accessible name, which this model cannot
    /// read at all.
    pub unlabelled_images: usize,
}

/// Pursues one goal on one page.
pub async fn pursue<O: Oracle, D: Driver>(
    oracle: &O,
    driver: &D,
    goal: &str,
    budget: Budget,
) -> Outcome {
    advance(oracle, driver, Run::new(goal, budget)).await
}

/// Carries a run on, from wherever it is.
pub async fn advance<O: Oracle, D: Driver>(oracle: &O, driver: &D, mut run: Run) -> Outcome {
    let started = Instant::now();
    let mut trail: Vec<Step> = Vec::new();
    let mut last: Option<PageView> = None;
    let mut space = ActionSpace::default();
    let mut goal_met = None;

    loop {
        if driver.stopped() {
            return end(&run, Status::Cancelled, last, &space, trail, goal_met);
        }
        if run.out_of_steps() || started.elapsed().as_secs() >= run.budget.max_seconds {
            return end(&run, Status::BudgetSpent, last, &space, trail, goal_met);
        }

        // The previous iteration read the page after it acted, and nothing has
        // touched it since. Reading again costs a full snapshot to learn
        // nothing. Where a path below changed the page without acting on a
        // decision, it drops the view instead of carrying it forward.
        let view = match last.take() {
            Some(view) => view,
            None => match driver.observe().await {
                Ok(view) => view,
                Err(why) => {
                    return end(
                        &run,
                        Status::NeedsInput(Handback::Unverifiable, why),
                        None,
                        &space,
                        trail,
                        goal_met,
                    );
                }
            },
        };
        if run.url_before.is_none() {
            run.url_before = Some(view.url.clone());
        }
        run.now_looking_at(view.fingerprint);
        run.standing_on(view.arrangement);
        // Every page the run observes is somewhere it has stood, including the
        // one it started on, so coming back to the start counts as a return.
        run.standing_somewhere_new(view.fingerprint);

        let destinations = keyed_destinations(&run);
        let values = keyed_values(&run);
        let surface = Surface {
            destinations: &destinations,
            can_return: run.travelled,
            tabs: &run.offered.tabs,
            values: &values,
            unavailable: &run.offered.unavailable,
        };
        // Once this run has decided to search this page, its search field stays
        // at the front of the question for as long as the page stands still.
        let insist: &[Kind] = if run.known.searched {
            &[Kind::SearchField]
        } else {
            &[]
        };
        let ordered = ActionSpace::ordered(
            &view,
            &run.goal,
            run.last_acted.as_deref(),
            &run.withheld(),
            insist,
        );

        let left = Duration::from_secs(run.budget.max_seconds).saturating_sub(started.elapsed());
        let searched = match search(oracle, &run, &view, &ordered, surface, left).await {
            Ok(searched) => searched,
            Err(why) => {
                return end(
                    &run,
                    Status::NeedsInput(Handback::Unverifiable, why),
                    Some(view),
                    &space,
                    trail,
                    goal_met,
                );
            }
        };
        run.decisions += searched.spent.decisions;
        run.batches_asked += searched.spent.batches;
        run.input_tokens += searched.spent.input_tokens;
        run.known = searched.spent.known;
        space = searched.space;
        let decision = searched.decision;
        goal_met = decision.goal_met;

        let target_name = decision
            .target
            .as_deref()
            .and_then(|reference| view.control(reference))
            .map(|control| control.name.clone());
        let supplied = decision
            .value
            .as_deref()
            .and_then(|key| run.offered.values.iter().find(|value| value.name == key))
            .map(|value| value.text.clone());

        // Whether this move is worth making. Asked only once a control has been
        // settled on, because it names one, and only when the move could
        // actually be carried out: a move with no argument to act with cannot
        // be worth a model call to approve.
        let worth = if decision.needs_target()
            && decision.target.is_some()
            && resolve(&decision, &run, &supplied).is_some()
        {
            let descriptor = decision
                .target
                .as_deref()
                .and_then(|reference| view.control(reference))
                .map(crate::space::describe);
            match descriptor {
                Some(descriptor) => {
                    let how = decision.how.map_or_else(
                        || decision.operation.as_str().to_string(),
                        |how| how.as_str().to_string(),
                    );
                    // What the move would actually do with the control: the
                    // text a fill types, the value a select sets, the key a
                    // press sends.
                    let argument = match decision.how {
                        Some(How::Fill) => supplied.as_deref(),
                        Some(How::Press) => decision.key.as_deref(),
                        _ => decision.value.as_deref(),
                    };
                    let asked = questions::approval(&run.goal, &how, &descriptor, argument);
                    let state =
                        questions::state(&view, &space, &run.recent, run.here.retired.len());
                    let left = Duration::from_secs(run.budget.max_seconds)
                        .saturating_sub(started.elapsed());
                    match oracle.ask_within(&state, &asked, left).await {
                        Ok(response) => {
                            run.decisions += 1;
                            run.input_tokens += response.usage.input_tokens;
                            response
                                .answers
                                .get(questions::WORTH_ACTING_ON)
                                .and_then(|answer| answer.noul)
                        }
                        // An approval that could not be asked is a refusal.
                        // Returning None here skips the check in `verdict`, and
                        // since the target's lead no longer gates, that left a
                        // click on an expensive control judged by the operation
                        // and the verb alone.
                        Err(_) => Some(0.0),
                    }
                }
                None => None,
            }
        } else {
            None
        };

        let judged = gate::verdict(
            &decision,
            target_name.as_deref(),
            supplied.as_deref(),
            worth,
        );
        // Recorded before it is acted on, so a decision that ends the run is in
        // the record that travels back with the ending.
        run.judged(decided_of(
            &decision,
            target_name.as_deref(),
            &judged,
            space.batch + 1,
        ));

        if decision.operation == Operation::Done {
            // A change the run never made is not a change the page holds. Once
            // the page text that travels is chosen for the goal, the goal's own
            // words are on the page, and a goal about adding a toothpaste to a
            // cart reads as met by a results page listing toothpaste: measured
            // three times out of three, one decision, no action, "satisfied".
            //
            // The model is documented as answering the question as written and
            // as losing reliability on multi-hop conditions, and "is this in the
            // cart" from a page that merely names it is two hops. So a
            // completion claim about a goal that asks for a change is only
            // accepted once something has observably changed. It is handed back
            // rather than refused, because the goal may genuinely have been
            // satisfied before the run started, and the caller can see the page.
            if asks_for_a_change(&run.goal) && !trail.iter().any(|step| step.page_changed) {
                return end(
                    &run,
                    Status::NeedsInput(
                        Handback::Unverifiable,
                        "it read this page as already satisfying the goal, and nothing on the \
                         page has changed, so that claim cannot be checked from here. Read the \
                         page, or name the control that would make the change."
                            .to_string(),
                    ),
                    Some(view),
                    &space,
                    trail,
                    goal_met,
                );
            }
            return end(&run, Status::Satisfied, Some(view), &space, trail, goal_met);
        }
        if decision.operation == Operation::Blocked {
            return end(
                &run,
                Status::Blocked("no listed move can make progress on this page".to_string()),
                Some(view),
                &space,
                trail,
                goal_met,
            );
        }

        match judged {
            Verdict::Execute => {}
            Verdict::HandBack(why, said) => {
                // Nothing listed can advance the goal, and this page can search
                // itself. That is not a reason to stop, it is the reason to
                // search: the thing the goal needs is not on this page yet.
                //
                // Once per page, because a search that does not help must not
                // be tried again, and the flag sits beside the fingerprint so
                // the page the search opens is a page that may be searched too.
                if why == Handback::NothingOffered
                    && view.can_search_itself()
                    && !run.known.searched
                    && run.searches < crate::run::SEARCH_LIMIT
                {
                    run.known.searched = true;
                    run.searches += 1;
                    run.remember(
                        "nothing listed could advance the goal, so this page's own search is \
                         offered first",
                    );
                    last = Some(view);
                    continue;
                }
                return end(
                    &run,
                    Status::NeedsInput(why, said),
                    Some(view),
                    &space,
                    trail,
                    goal_met,
                );
            }
            Verdict::Explore(_) => {
                run.declined += 1;
                if run.declined >= DECLINE_LIMIT {
                    return end(
                        &run,
                        Status::Stalled(Stuck::NotConfident),
                        Some(view),
                        &space,
                        trail,
                        goal_met,
                    );
                }
                // Asking the identical question of the identical page returns
                // the identical answer, so something has to change before
                // asking again. Which thing depends on what fell short, and a
                // fixed answer to that is expensive: a scroll issued on every
                // refusal regardless of cause, including refusals about a
                // control sitting on screen, moved the only control that
                // submits a page's search out of the viewport, and the next
                // decision clicked where it used to be.
                if worth.is_some_and(|worth| {
                    worth
                        < decision
                            .operation
                            .consequence(decision.how, target_name.as_deref())
                            .approval()
                }) && let Some(reference) = decision.target.as_deref()
                {
                    // The page is fine. This move is not. Withhold it and ask
                    // again, which changes the question without moving the
                    // page under it.
                    run.set_aside(reference);
                    continue;
                }
                // Nothing here could be settled on, so show the page something
                // else.
                run.spent += 1;
                let scroll = Decision::scroll_down();
                let _ = driver.perform(&Move::Act(Box::new(scroll))).await;
                // Scrolling changed what the page shows, so the view in hand
                // describes a position the run is no longer at. Dropping it
                // makes the next iteration observe, which is the point of
                // having scrolled.
                last = None;
                continue;
            }
        }
        run.declined = 0;

        let Some(next) = resolve(&decision, &run, &supplied) else {
            // A move that cannot be built is not a reason to end the run while
            // the page still holds other controls. Found live: a flight search
            // whose form was already complete aimed at the departure date,
            // `fill_value` correctly answered none_of_these because no supplied
            // value belongs in a date field, and the run stopped there with the
            // only useful move, pressing Search, still on the page.
            if let Some(reference) = decision.target.as_deref()
                && !run.here.set_aside.contains(&reference.to_string())
            {
                run.set_aside(reference);
                continue;
            }
            return end(
                &run,
                Status::NeedsInput(
                    Handback::NoFieldNamed,
                    "the chosen move could not be turned into something to run".to_string(),
                ),
                Some(view),
                &space,
                trail,
                goal_met,
            );
        };

        if matches!(next, Move::Travel(_)) {
            run.navigations += 1;
            if run.navigations > NAVIGATION_LIMIT {
                return end(
                    &run,
                    Status::Blocked(
                        "this run has moved between pages as often as it may".to_string(),
                    ),
                    Some(view),
                    &space,
                    trail,
                    goal_met,
                );
            }
        }
        if matches!(next, Move::Settle) {
            run.waits += 1;
            if run.waits > WAIT_LIMIT {
                return end(
                    &run,
                    Status::Stalled(Stuck::OnlyWaiting),
                    Some(view),
                    &space,
                    trail,
                    goal_met,
                );
            }
        }

        // Acting twice in a row on the same control is not progress, whatever
        // the page says about it. A live errand typed the caller's value into
        // the same search box six times over, and on every look the box already
        // held that value, so the control's own state is not enough to stop it.
        // Whether the control the last step aimed at is still on this page. Gone,
        // with an identically named one in its place, is one control through a
        // re-render rather than two controls.
        let last_still_there = run
            .recent_aims
            .last()
            .is_some_and(|(was, _)| view.control(was).is_some());
        // Aimed at before, further back than the step before. Not a reason to
        // stop, because a menu may be opened twice on purpose, but the next
        // decision should know: measured live, a run walked a four-move cycle
        // three times and every move looked new to a guard that only compared
        // with the move before it.
        let aimed_here_before = run.aimed_here_recently(decision.target.as_deref());
        if decision.aims_at_a_control()
            && run.aiming_again_at(
                decision.target.as_deref(),
                target_name.as_deref(),
                last_still_there,
            )
        {
            // Spends a step even though nothing was run. The decision was paid
            // for, and without this the guard is unbounded: a run whose control
            // is already retired would set it aside again forever.
            run.spent += 1;
            run.last_acted = decision.target.clone();
            run.retire_last();
            run.remember(format!(
                "aimed at {} twice running, so it is set aside",
                decision.target.as_deref().unwrap_or("a control")
            ));
            last = Some(view);
            continue;
        }

        if aimed_here_before && decision.aims_at_a_control() {
            run.remember(format!(
                "aimed at {} again, a few moves after the last time",
                decision.target.as_deref().unwrap_or("a control")
            ));
        }

        run.spent += 1;
        run.last_acted = decision.target.clone();
        let before = view.fingerprint;

        // What the tool said about itself is not read. Whether the page moved
        // is decided by looking again, which is authoritative and needs no
        // agreement with another component about how it words a result.
        match driver.perform(&next).await {
            Ok(_) => {}
            Err(error) => {
                run.remember(format!("tried to act and the browser refused: {error}"));
                run.retire_last();
                if run.spent >= run.budget.max_steps {
                    return end(
                        &run,
                        Status::Stalled(Stuck::BrowserKeptFailing),
                        Some(view),
                        &space,
                        trail,
                        goal_met,
                    );
                }
                last = Some(view);
                continue;
            }
        };

        if matches!(next, Move::Travel(_)) {
            run.travelled = true;
        }

        // Looks, and if nothing moved, keeps looking for a moment: a control
        // whose effect is delayed is indistinguishable from a dead one at the
        // instant the act returns.
        let after = match driver.observe().await {
            Ok(fresh) => Some(look_for_a_change(driver, before, fresh).await),
            Err(_) => None,
        };
        let changed = after
            .as_ref()
            .is_some_and(|view| view.fingerprint != before);
        // What this action observably did, taken by joining the page before it
        // with the page after it on the control reference.
        let moved = after.as_ref().map_or_else(Vec::new, |now| {
            view.what_moved(now, &run.goal, MOVED_REPORTED)
        });
        run.remember(said(&decision, changed, target_name.as_deref(), &moved));
        trail.push(step_of(
            &decision,
            changed,
            after.as_ref().map_or(&view.url, |view| &view.url),
            moved,
            supplied.as_deref(),
        ));

        if changed {
            run.unchanged = 0;
            // A control was set aside because the move was not worth making on
            // the page as it stood. The page has moved, so that judgement is
            // about a page that is gone: a run that rejects Search while a
            // required box is empty, then fills the box, has to be able to
            // reach Search again. Retirement is not cleared with it, because
            // a control that did nothing when acted on is a different claim.
            run.reconsider_set_aside();
            // The page moved. Whether the run got anywhere is a different
            // question, and asking whether anything changed cannot answer it:
            // on a lap of a loop everything changes every time. Measured live,
            // a run walked the same four moves three times, the url and the
            // control count moved on every one of them, and the counter reset
            // on every one of them.
            //
            // What catches a loop is novelty. A page this run has already stood
            // on has been returned to, not arrived at, whatever changed on the
            // way back to it.
            let arrived = after
                .as_ref()
                .is_some_and(|now| run.standing_somewhere_new(now.fingerprint));
            let closer = goal_met.is_some_and(|now| now > run.closest + 0.05);
            if arrived || closer {
                run.circling = 0;
                if let Some(now) = goal_met {
                    run.closest = run.closest.max(now);
                }
            } else {
                run.circling += 1;
                if run.circling >= crate::run::CIRCLING_LIMIT {
                    return end(
                        &run,
                        Status::Stalled(Stuck::GoingInCircles),
                        after.or(Some(view)),
                        &space,
                        trail,
                        goal_met,
                    );
                }
            }
        } else {
            // Whatever that was aimed at did nothing. Retire it rather than let
            // the loop spend its whole budget on one dead reference, and do not
            // count the step as a stall: the next decision has a different set
            // to choose from, so it is not the same question.
            run.unchanged += 1;
            if run.retire_last() {
                run.unchanged = 0;
            } else if run.unchanged >= STALL_LIMIT {
                return end(
                    &run,
                    Status::Stalled(Stuck::NothingChanged),
                    after.or(Some(view)),
                    &space,
                    trail,
                    goal_met,
                );
            }
        }
        last = after.or(Some(view));
    }
}

/// How long to keep looking for a change the page has not made yet.
///
/// Only ever spent on the unchanged path, which is the path that otherwise
/// retires a control that worked.
const SLOW_PAGE_PATIENCE: Duration = Duration::from_millis(1_500);
/// How often to look while waiting for that change.
const SLOW_PAGE_POLL: Duration = Duration::from_millis(150);

/// Looks again until the page differs from what it was, or patience runs out.
///
/// The settle inside `observe` answers "has this page stopped changing", which
/// is the wrong question for a control whose effect is delayed. A page that has
/// not begun reacting is perfectly quiet, so a quiet check returns at once and
/// the loop reads the move as having done nothing.
///
/// Measured against a button whose panel appears 400ms after the click: the act
/// tool returns in under 10ms, the page is quiet when the settle looks, and the
/// settle returns immediately. The loop then saw an unchanged page and retired
/// a control that worked perfectly.
///
/// "Has it started" is a question only the loop can ask, because the
/// fingerprint from before the act lives here. So the waiting lives here too,
/// and the driver keeps its two methods.
async fn look_for_a_change<D: Driver>(driver: &D, before: u64, view: PageView) -> PageView {
    if view.fingerprint != before {
        return view;
    }
    let until = Instant::now() + SLOW_PAGE_PATIENCE;
    let mut latest = view;
    while Instant::now() < until {
        tokio::time::sleep(SLOW_PAGE_POLL).await;
        match driver.observe().await {
            Ok(fresh) => {
                let moved = fresh.fingerprint != before;
                latest = fresh;
                if moved {
                    break;
                }
            }
            // A page that cannot be read is not a page that did not change, so
            // the view in hand is the honest answer.
            Err(_) => break,
        }
    }
    latest
}

/// One decision, the batch of the page it was decided against, and what
/// reaching it cost.
///
/// The cost is reported rather than applied, because a search reads the run and
/// cannot also hold it mutably while the question surface borrows from it.
/// Reporting is the better shape anyway: a search that states its own bill is
/// easier to test than one that reaches into the ledger.
struct Searched {
    decision: Decision,
    space: ActionSpace,
    spent: Spent,
}

/// What one search of a page cost.
#[derive(Debug, Default)]
struct Spent {
    decisions: u32,
    batches: u32,
    input_tokens: u64,
    /// The run's memory of this page, updated by what the search learned.
    known: Known,
}

/// Asks about a page, batch by batch, until something settles or it runs out.
///
/// A question may offer 254 controls and a page may have more, so one look at a
/// page can cost several questions. Three things keep that from being wasteful.
///
/// A batch that settles stops the search, because nothing further down the page
/// is going to beat a control holding three quarters of the answer. On one
/// measured page, 7 of the 8 batches asked were a re-search of a page already
/// answered by the first.
///
/// A page already searched to the end is not searched again while it stays
/// still, which is what [`Run::known`] is for.
///
/// And an answer that needs no control at all ends the search immediately:
/// there is nothing further down the page that could change what READ or DONE
/// means.
async fn search<O: Oracle>(
    oracle: &O,
    run: &Run,
    view: &PageView,
    ordered: &[Candidate],
    surface: Surface<'_>,
    budget: Duration,
) -> Result<Searched, String> {
    let mut spent = Spent {
        known: run.known.clone(),
        ..Spent::default()
    };
    // A page searched to the end already, still standing still, gets one
    // question rather than all of them again.
    let batches = if spent.known.exhausted {
        1
    } else {
        ActionSpace::batches(ordered).max(1)
    };
    let mut best: Option<(Decision, ActionSpace)> = None;

    for batch in 0..batches {
        let space = ActionSpace::batch(ordered, batch);
        let moves = questions::available(&space, view, surface);
        let ways = questions::ways(&space, view);
        let asked = questions::build(&run.goal, &space, &moves, &ways, surface);
        let state = questions::state(view, &space, &run.recent, run.here.retired.len());

        let response = oracle
            .ask_within(&state, &asked, budget)
            .await
            .map_err(|error| error.to_string())?;
        spent.decisions += 1;
        spent.batches += 1;
        spent.input_tokens += response.usage.input_tokens;

        let decision = gate::read(&response).map_err(|error| error.to_string())?;

        // A move that aims at nothing is not a question about this batch, so
        // nothing further down the page could improve on it.
        if !decision.needs_target() {
            return Ok(Searched {
                decision,
                space,
                spent,
            });
        }

        let share = decision.target_lead.map_or(0.0, |lead| lead.share);
        if let Some(reference) = decision.target.as_deref() {
            spent.known.found(reference, share, batch);
        }
        let better = best
            .as_ref()
            .is_none_or(|(held, _)| share > held.target_lead.map_or(0.0, |lead| lead.share));
        if better {
            best = Some((decision, space));
        }
        if share >= crate::run::SETTLED_SHARE {
            break;
        }
    }

    // Every batch was offered and none of them settled it, so the next decision
    // on this same page should not pay to find that out again. What was found
    // stays: an exhausted page is not an empty one.
    if !spent.known.settled() {
        spent.known.exhausted = true;
    }

    best.map(|(decision, space)| Searched {
        decision,
        space,
        spent,
    })
    .ok_or_else(|| "this page offered nothing to decide between".to_string())
}

/// Turns a judged decision into something the driver can run.
fn resolve(decision: &Decision, run: &Run, supplied: &Option<String>) -> Option<Move> {
    match decision.operation {
        Operation::Act => {
            let how = decision.how?;
            if how == How::Fill {
                // The model chose the value by its name, because a name is
                // something it can choose between and a string is something it
                // cannot write. The name is not what belongs in the field. Only
                // the caller has the text, so the key is swapped for it here.
                //
                // Without this swap the key itself is typed: a run given
                // {"search term": "32GB DDR5"} put the words "search term" in
                // the search box. Measured by reading the box afterwards.
                let text = supplied.clone()?;
                let mut filled = decision.clone();
                filled.value = Some(text);
                return Some(Move::Act(Box::new(filled)));
            }
            if how == How::Submit {
                // The field is the one the previous step typed into, which this
                // run recorded. Whatever the model named for the target head is
                // discarded on purpose: that head was not read for a submit and
                // so was never judged, and acting on an ungated judgement is
                // what the gate exists to prevent.
                //
                // No field recorded leaves the reference unset, which the driver
                // sends as a bare key press at whatever holds the focus. That is
                // the behaviour of the implementation this was measured against,
                // so it is a weaker fallback rather than a new risk.
                let mut submitting = decision.clone();
                submitting.target = run.last_acted.clone();
                submitting.key = Some("Enter".to_string());
                return Some(Move::Act(Box::new(submitting)));
            }
            // A select is not the same case. Its value came from the control's
            // own options, flattened into the question as one option per
            // control and value, so it is already the literal thing the page
            // expects rather than a name standing in for it.
            Some(Move::Act(Box::new(decision.clone())))
        }
        Operation::Navigate => {
            let key = decision.target.as_deref()?;
            let destination = run
                .offered
                .destinations
                .iter()
                .find(|destination| key_for(&destination.label) == key)?;
            Some(Move::Travel(Journey::To(destination.clone())))
        }
        Operation::Back => Some(Move::Travel(Journey::Back)),
        Operation::Reload => Some(Move::Travel(Journey::Again)),
        Operation::Tabs => {
            let page = decision.target.as_deref()?.parse().ok()?;
            Some(Move::Travel(Journey::Tab(page)))
        }
        Operation::Wait => Some(Move::Settle),

        Operation::Done | Operation::Blocked => None,
    }
}

/// What to search this page for, chosen by how well it narrows this page.
///
/// The old rule took the goal's longest word, which is a property of the goal
/// and says nothing about the page. Measured live: the goal named a Sensodyne
/// toothpaste on a page of toothpaste search results, the longest word was
/// "Toothpaste", and the match was 55,718 characters of nearly every line.
///
/// So the candidates are scored against what is actually on the page, and the
/// most selective one that still matches something wins. `None` means no word
/// What to search the page for, derived from the goal in code.
///
/// The model cannot write a pattern, so this is the only way a search can be
/// aimed at anything. The longest distinctive word is a better needle than the
/// The key a destination is chosen by, which is its label made safe.
#[must_use]
pub fn key_for(label: &str) -> String {
    let key: String = label
        .chars()
        .map(|character| {
            if character.is_alphanumeric() {
                character.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect();
    key.trim_matches('_').to_string()
}

fn keyed_destinations(run: &Run) -> Vec<(String, &Destination)> {
    run.offered
        .destinations
        .iter()
        .map(|destination| (key_for(&destination.label), destination))
        .collect()
}

fn keyed_values(run: &Run) -> Vec<(String, &crate::offer::NamedValue)> {
    run.offered
        .values
        .iter()
        .map(|value| (value.name.clone(), value))
        .collect()
}

fn step_of(
    decision: &Decision,
    changed: bool,
    url: &str,
    moved: Vec<Moved>,
    typed: Option<&str>,
) -> Step {
    Step {
        operation: decision.operation,
        how: decision.how,
        target: decision.target.clone(),
        // What was actually typed, not the name the caller filed it under. The
        // model chooses a value by its name because it cannot write a string,
        // and the substitution happens on the decision the driver is handed.
        // Recording the name here told a caller it had typed "search term"
        // when it had typed "MCP tunnel".
        value: typed.map(str::to_string).or_else(|| decision.value.clone()),
        confidence: decision.confidence(),
        page_changed: changed,
        url_after: url.to_string(),
        moved,
        input_tokens: decision.input_tokens,
    }
}

/// At most this many moved controls are reported per step.
///
/// A live page has a countdown on it, so a step that moved everything should
/// report the few things the goal is about rather than the clock.
const MOVED_REPORTED: usize = 3;

/// Whether the goal asks for the page to end up different than it started.
///
/// Deliberately a word list. The alternative is asking the model, and a
/// question about whether to trust the model's answer cannot be settled by
/// another of its answers. Being wrong in the permissive direction costs a
/// completion claim that is handed back for the caller to check, which is the
/// safer of the two mistakes.
fn asks_for_a_change(goal: &str) -> bool {
    const CHANGES: &[&str] = &[
        "add", "set", "tick", "untick", "check", "uncheck", "toggle", "select", "choose", "pick",
        "fill", "type", "enter", "write", "submit", "apply", "search", "sort", "filter", "narrow",
        "buy", "order", "book", "pay", "remove", "delete", "clear", "sign in", "log in", "upload",
        "download", "click", "press", "open", "go to", "navigate",
    ];
    let goal = goal.to_lowercase();
    CHANGES.iter().any(|verb| goal.contains(verb))
}

fn said(decision: &Decision, changed: bool, target_name: Option<&str>, moved: &[Moved]) -> String {
    let what = decision.operation.as_str();
    let how = decision.how.map(How::as_str).unwrap_or("");
    // By name, because a ref means nothing to a model that cannot remember
    // minting it. "clicked Add to cart" is a fact about the goal; "ACT CLICK on
    // e119" is a fact about our own bookkeeping.
    let on = target_name
        .map(|name| format!("{name:?}"))
        .or_else(|| decision.target.clone())
        .unwrap_or_else(|| "the page".to_string());
    if !changed {
        return format!("{what} {how} on {on}, and the page did not change");
    }
    if moved.is_empty() {
        return format!("{what} {how} on {on}, and the page changed");
    }
    // The effects, in the same words the caller is told them in. This is the
    // whole of the run's memory of having succeeded: the model is documented as
    // holding nothing between calls, so a cart that went from empty to full is
    // evidence it can only have from here. Without it a goal that is already met
    // reads exactly like one that has not been started, and the next decision
    // does the same thing again.
    let effects = moved
        .iter()
        .map(|change| format!("{:?} became {:?}", change.was, change.now))
        .collect::<Vec<_>>()
        .join(", and ");
    format!("{what} {how} on {on}, and {effects}")
}

#[allow(clippy::too_many_arguments)]
/// One decision turned into the record a caller reads.
///
/// The gate has already worked all of this out: which judgement was weakest,
/// by how much it missed and in what words. This only keeps it.
fn decided_of(
    decision: &Decision,
    target_name: Option<&str>,
    judged: &Verdict,
    batch: usize,
) -> Decided {
    let consequence = decision.operation.consequence(decision.how, target_name);
    let short = decision.short_of(consequence);
    let (verdict, why) = match judged {
        Verdict::Execute => ("ran", None),
        Verdict::Explore(said) => ("explored", Some(said.clone())),
        Verdict::HandBack(_, said) => ("handed_back", Some(said.clone())),
    };
    Decided {
        operation: decision.operation.as_str(),
        how: decision.how.map(How::as_str),
        target: decision.target.clone(),
        target_name: target_name.map(str::to_string),
        verdict,
        weakest: short.map(|(which, _)| which.as_str()),
        margin: short.map(|(_, lead)| lead.margin),
        needed: short.map(|(_, lead)| {
            if lead.distributed {
                consequence.margin()
            } else {
                consequence.share_bar()
            }
        }),
        share: short.map(|(_, lead)| lead.share),
        why,
        batch,
        input_tokens: decision.input_tokens,
    }
}

fn end(
    run: &Run,
    status: Status,
    view: Option<PageView>,
    space: &ActionSpace,
    trail: Vec<Step>,
    goal_met: Option<f64>,
) -> Outcome {
    Outcome {
        status,
        offered: space.offered.len(),
        omitted: space.omitted,
        retired: run.retired_total,
        decisions: run.decisions,
        batches: run.batches_asked,
        input_tokens: run.input_tokens,
        trail,
        decided: run.decided.clone(),
        url_before: run.url_before.clone().unwrap_or_default(),
        url_after: view
            .as_ref()
            .map(|view| view.url.clone())
            .unwrap_or_default(),
        title: view
            .as_ref()
            .map(|view| view.title.clone())
            .unwrap_or_default(),
        goal_met,
        unlabelled_images: view.as_ref().map_or(0, |view| view.unlabelled_images),
        ended_on: view,
    }
}

#[cfg(test)]
mod memory_tests {
    use super::*;
    use crate::gate::Lead;

    fn clicked() -> Decision {
        Decision {
            operation: Operation::Act,
            operation_lead: Lead::default(),
            how: Some(How::Click),
            how_lead: None,
            target: Some("e119".to_string()),
            target_lead: None,
            value: None,
            value_lead: None,
            key: None,
            key_lead: None,
            direction: None,
            none_of_these: false,
            goal_met: None,
            input_tokens: 0,
        }
    }

    /// The whole of the run's memory of having succeeded. Without the effects,
    /// a goal that is already met reads like one that was never started.
    #[test]
    fn the_memory_carries_what_the_action_did_to_the_page() {
        let moved = vec![Moved {
            reference: "e57".to_string(),
            was: "0 items in cart".to_string(),
            now: "1 item in cart".to_string(),
        }];
        let remembered = said(&clicked(), true, Some("Add to cart"), &moved);
        assert!(remembered.contains("Add to cart"), "{remembered}");
        assert!(remembered.contains("0 items in cart"), "{remembered}");
        assert!(remembered.contains("1 item in cart"), "{remembered}");
    }

    /// A ref is our own bookkeeping and means nothing to a model that cannot
    /// remember minting it.
    #[test]
    fn the_memory_names_the_control_rather_than_its_reference() {
        let remembered = said(&clicked(), true, Some("Add to cart"), &[]);
        assert!(remembered.contains("Add to cart"), "{remembered}");
        assert!(!remembered.contains("e119"), "{remembered}");
    }

    #[test]
    fn an_action_that_moved_nothing_still_says_so() {
        let remembered = said(&clicked(), false, Some("Add to cart"), &[]);
        assert!(remembered.contains("did not change"), "{remembered}");
    }
}
