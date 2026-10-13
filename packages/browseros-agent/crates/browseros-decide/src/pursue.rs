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
    pub input_tokens: u64,
    pub trail: Vec<Step>,
    pub url_before: String,
    pub url_after: String,
    pub title: String,
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
    let mut space = ActionSpace {
        offered: Vec::new(),
        omitted: 0,
    };
    let mut goal_met = None;

    loop {
        if driver.stopped() {
            return end(&run, Status::BudgetSpent, last, &space, trail, None, goal_met);
        }
        if run.out_of_steps() || started.elapsed().as_secs() >= run.budget.max_seconds {
            return end(&run, Status::BudgetSpent, last, &space, trail, None, goal_met);
        }

        let view = match driver.observe().await {
            Ok(view) => view,
            Err(why) => {
                return end(
                    &run,
                    Status::NeedsInput(Handback::Unverifiable, why),
                    last,
                    &space,
                    trail,
                    None,
                    goal_met,
                );
            }
        };
        if run.url_before.is_none() {
            run.url_before = Some(view.url.clone());
        }

        space = ActionSpace::build(&view, &run.goal, run.last_acted.as_deref(), &run.retired);
        let destinations = keyed_destinations(&run);
        let values = keyed_values(&run);
        let surface = Surface {
            destinations: &destinations,
            can_return: run.travelled,
            tabs: &run.offered.tabs,
            values: &values,
            unavailable: &run.offered.unavailable,
        };
        let moves = questions::available(&space, &view, surface);
        let ways = questions::ways(&space, &view);
        let asked = questions::build(&run.goal, &space, &moves, &ways, surface);
        let state = questions::state(&view, &space, &run.recent, run.retired.len());

        let left = Duration::from_secs(run.budget.max_seconds).saturating_sub(started.elapsed());
        let response = match oracle.ask_within(&state, &asked, left).await {
            Ok(response) => response,
            Err(error) => {
                return end(
                    &run,
                    Status::NeedsInput(Handback::Unverifiable, error.to_string()),
                    Some(view),
                    &space,
                    trail,
                    None,
                    goal_met,
                );
            }
        };
        run.decisions += 1;
        run.input_tokens += response.usage.input_tokens;

        let decision = match gate::read(&response) {
            Ok(decision) => decision,
            Err(error) => {
                return end(
                    &run,
                    Status::NeedsInput(Handback::Unverifiable, error.to_string()),
                    Some(view),
                    &space,
                    trail,
                    None,
                    goal_met,
                );
            }
        };
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
                last = Some(view);
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

        let after = driver.observe().await.ok();
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

/// Turns a judged decision into something the driver can run.
fn resolve(decision: &Decision, run: &Run, supplied: &Option<String>) -> Option<Move> {
    match decision.operation {
        Operation::Act => {
            let how = decision.how?;
            if how == How::Fill && supplied.is_none() {
                return None;
            }
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
        input_tokens: run.input_tokens,
        trail,
        url_before: run.url_before.clone().unwrap_or_default(),
        url_after: view.as_ref().map(|view| view.url.clone()).unwrap_or_default(),
        title: view.as_ref().map(|view| view.title.clone()).unwrap_or_default(),
        answer,
        goal_met,
        unlabelled_images: view.as_ref().map_or(0, |view| view.unlabelled_images),
    }
}
