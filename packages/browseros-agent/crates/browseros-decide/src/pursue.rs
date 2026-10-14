//! One goal, one page, until something ends it.
//!
//! The loop observes, asks once, judges the answer, runs it, and observes again.
//! Whether the page moved is decided by looking rather than by asking the tool
//! that moved it, which is authoritative and needs no agreement with another
//! component about how it words a result.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use crate::client::{Jev, JevError, Question};
use crate::drive::{Answer, Ask, Driver, Journey, Move, Performed};
use crate::gate::{self, Decision, Handback, Verdict};
use crate::offer::Destination;
use crate::questions::{self, Surface};
use crate::run::{
    Budget, DECLINE_LIMIT, NAVIGATION_LIMIT, Run, STALL_LIMIT, WAIT_LIMIT,
};
use crate::run::Known;
use crate::space::Candidate;
use crate::space::ActionSpace;
use crate::view::PageView;
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
    /// The run answered a question about the page and stopped, which is what a
    /// report is for.
    Answered,
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
            Self::Answered => "answered",
        }
    }
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
    pub url_before: String,
    pub url_after: String,
    pub title: String,
    /// The page the run ended on, as the caller can act on it.
    ///
    /// The one thing the previous attempt did not return, and the reason a
    /// caller could do nothing with a failure but reword its goal and hope. A
    /// caller holding control names has something concrete to change.
    pub ended_on: Option<PageView>,
    /// What a report handed back, when one ended the run.
    pub answer: Option<Answer>,
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
            return end(&run, Status::Cancelled, last, &space, trail, None, goal_met);
        }
        if run.out_of_steps() || started.elapsed().as_secs() >= run.budget.max_seconds {
            return end(&run, Status::BudgetSpent, last, &space, trail, None, goal_met);
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
                        None,
                        goal_met,
                    );
                }
            },
        };
        if run.url_before.is_none() {
            run.url_before = Some(view.url.clone());
        }
        run.now_looking_at(view.fingerprint);

        let destinations = keyed_destinations(&run);
        let values = keyed_values(&run);
        let surface = Surface {
            destinations: &destinations,
            can_return: run.travelled,
            tabs: &run.offered.tabs,
            values: &values,
            unavailable: &run.offered.unavailable,
        };
        let ordered = ActionSpace::ordered(&view, &run.goal, run.last_acted.as_deref(), &run.retired);

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
                    None,
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

        if decision.operation == Operation::Done {
            return end(&run, Status::Satisfied, Some(view), &space, trail, None, goal_met);
        }
        if decision.operation == Operation::Blocked {
            return end(
                &run,
                Status::Blocked("no listed move can make progress on this page".to_string()),
                Some(view),
                &space,
                trail,
                None,
                goal_met,
            );
        }

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

        match gate::verdict(&decision, target_name.as_deref(), supplied.as_deref()) {
            Verdict::Execute => {}
            Verdict::HandBack(why, said) => {
                return end(
                    &run,
                    Status::NeedsInput(why, said),
                    Some(view),
                    &space,
                    trail,
                    None,
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
                        None,
                        goal_met,
                    );
                }
                // Asking the identical question of the identical page returns
                // the identical answer, so the honest response is to change what
                // the next decision can see.
                run.spent += 1;
                let scroll = Decision::scroll_down();
                let _ = driver.perform(&Move::Act(scroll)).await;
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
            return end(
                &run,
                Status::NeedsInput(
                    Handback::NoFieldNamed,
                    "the chosen move could not be turned into something to run".to_string(),
                ),
                Some(view),
                &space,
                trail,
                None,
                goal_met,
            );
        };

        if matches!(next, Move::Travel(_)) {
            run.navigations += 1;
            if run.navigations > NAVIGATION_LIMIT {
                return end(
                    &run,
                    Status::Blocked("this run has moved between pages as often as it may".to_string()),
                    Some(view),
                    &space,
                    trail,
                    None,
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
                    None,
                    goal_met,
                );
            }
        }

        // Acting twice in a row on the same control is not progress, whatever
        // the page says about it. A live errand typed the caller's value into
        // the same search box six times over, and on every look the box already
        // held that value, so the control's own state is not enough to stop it.
        if decision.aims_at_a_control() && run.aiming_again_at(decision.target.as_deref()) {
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

        run.spent += 1;
        run.last_acted = decision.target.clone();
        let before = view.fingerprint;

        let performed = match driver.perform(&next).await {
            Ok(performed) => performed,
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
                        None,
                        goal_met,
                    );
                }
                last = Some(view);
                continue;
            }
        };

        if let Performed::Answer(answer) = performed {
            trail.push(step_of(&decision, false, &view.url));
            return end(
                &run,
                Status::Answered,
                Some(view),
                &space,
                trail,
                Some(answer),
                goal_met,
            );
        }

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
        trail.push(step_of(
            &decision,
            changed,
            after.as_ref().map_or(&view.url, |view| &view.url),
        ));
        run.remember(said(&decision, changed));

        if changed {
            run.unchanged = 0;
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
                    None,
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
        let state = questions::state(view, &space, &run.recent, run.retired.len());

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
            return Ok(Searched { decision, space, spent });
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

    best.map(|(decision, space)| Searched { decision, space, spent })
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
                return Some(Move::Act(filled));
            }
            // A select is not the same case. Its value came from the control's
            // own options, flattened into the question as one option per
            // control and value, so it is already the literal thing the page
            // expects rather than a name standing in for it.
            Some(Move::Act(decision.clone()))
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
        Operation::Snapshot => Some(Move::Resnapshot),
        Operation::Wait => Some(Move::Settle),
        Operation::Read => Some(Move::Report(Ask::Read)),
        Operation::Grep => Some(Move::Report(Ask::Grep(pattern_for(&run.goal)))),
        Operation::Count => Some(Move::Report(Ask::Count(pattern_for(&run.goal)))),
        Operation::CheckState => Some(Move::Report(Ask::State {
            reference: decision.target.clone()?,
            state: decision.state.clone().unwrap_or_else(|| "visible".to_string()),
        })),
        Operation::Screenshot => Some(Move::Report(Ask::Picture)),
        Operation::Evaluate => Some(Move::Report(Ask::Evaluate(pattern_for(&run.goal)))),
        Operation::Done | Operation::Blocked => None,
    }
}

/// Words too common to say anything about what a goal is asking for.
const NOISE: &[&str] = &[
    "the", "and", "for", "with", "this", "that", "from", "how", "many", "does", "what", "find",
    "out", "page", "are", "was", "its", "into", "than", "then", "there", "here", "about", "whether",
];

/// What to search the page for, derived from the goal in code.
///
/// The model cannot write a pattern, so this is the only way a search can be
/// aimed at anything. The longest distinctive word is a better needle than the
/// whole goal, which would match nothing.
fn pattern_for(goal: &str) -> String {
    goal.split(|character: char| !character.is_alphanumeric())
        .filter(|word| word.len() > 3)
        .filter(|word| !NOISE.contains(&word.to_lowercase().as_str()))
        .max_by_key(|word| word.len())
        .unwrap_or("")
        .to_string()
}

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

fn step_of(decision: &Decision, changed: bool, url: &str) -> Step {
    Step {
        operation: decision.operation,
        how: decision.how,
        target: decision.target.clone(),
        value: decision.value.clone(),
        confidence: decision.confidence(),
        page_changed: changed,
        url_after: url.to_string(),
        input_tokens: decision.input_tokens,
    }
}

fn said(decision: &Decision, changed: bool) -> String {
    let what = decision.operation.as_str();
    let how = decision.how.map(How::as_str).unwrap_or("");
    let on = decision.target.as_deref().unwrap_or("the page");
    if changed {
        format!("{what} {how} on {on}, and the page changed")
    } else {
        format!("{what} {how} on {on}, and the page did not change")
    }
}

#[allow(clippy::too_many_arguments)]
fn end(
    run: &Run,
    status: Status,
    view: Option<PageView>,
    space: &ActionSpace,
    trail: Vec<Step>,
    answer: Option<Answer>,
    goal_met: Option<f64>,
) -> Outcome {
    Outcome {
        status,
        offered: space.offered.len(),
        omitted: space.omitted,
        retired: run.retired.len(),
        decisions: run.decisions,
        batches: run.batches_asked,
        input_tokens: run.input_tokens,
        trail,
        url_before: run.url_before.clone().unwrap_or_default(),
        url_after: view.as_ref().map(|view| view.url.clone()).unwrap_or_default(),
        title: view.as_ref().map(|view| view.title.clone()).unwrap_or_default(),
        answer,
        goal_met,
        unlabelled_images: view.as_ref().map_or(0, |view| view.unlabelled_images),
        ended_on: view,
    }
}
