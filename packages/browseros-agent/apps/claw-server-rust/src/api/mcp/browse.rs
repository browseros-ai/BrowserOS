//! Driving a goal on a real page, and the result the caller reads.
//!
//! The loop itself lives in the policy crate and knows nothing about a
//! browser. This is the adapter: it observes through the ordinary snapshot and
//! acts through the ordinary act tool, so a run uses exactly the same code
//! paths an agent would have used by hand.

use crate::AppState;
use crate::api::mcp::dispatch::{ToolCall, ToolIdentity, dispatch_tool_call_in_process};
use crate::ids::SessionId;
use browseros_core::{
    BrowserSession, PageId,
    snapshot::{SnapshotMode, SnapshotOptions},
};
use browseros_mcp::{OutputFileAccess, ToolDef, ToolResult};
use browseros_policy::action::{ActionSpace, Element, annotate_from_tree};
use browseros_policy::drive::{Driver, Outcome, Status, TrailEntry};
use browseros_policy::{Observation, Operation};
use rmcp::model::ContentBlock;
use serde_json::{Value, json};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use tokio_util::sync::CancellationToken;

/// Observes and acts on one page for the duration of a run.
///
/// Actions go through the ordinary dispatch pipeline rather than straight to
/// the tool, so a run's individual steps claim pages, record tab activity and
/// produce an ownership notice exactly as the same action made by hand would.
/// Calling the tool directly made a multi-step run less visible than the steps
/// it replaced.
pub struct PageDriver {
    browser: Arc<BrowserSession>,
    /// The page the run is working on, which moves when an action opens a new
    /// one. A product link on a shopping site opens in a new tab, so a driver
    /// pinned to one page watches the wrong one from the first click onwards.
    page: AtomicU32,
    output_files: OutputFileAccess,
    cancel: CancellationToken,
    catalog: Arc<Vec<ToolDef>>,
    act_index: usize,
    session_id: SessionId,
    identity: ToolIdentity,
    default_tab_group_id: Option<String>,
    state: AppState,
    /// Notices the effects attached to an action, such as a page belonging to
    /// someone else. Collected so the caller sees them, since a run makes these
    /// actions on its behalf.
    notices: Mutex<Vec<String>>,
    /// Scroll directions that have already been tried and moved nothing, so a
    /// short page stops being offered a scroll it cannot perform.
    scroll_down_dead: AtomicBool,
    scroll_up_dead: AtomicBool,
}

impl PageDriver {
    #[expect(
        clippy::too_many_arguments,
        reason = "a dispatch needs exactly this context; bundling it would only move the list"
    )]
    #[must_use]
    pub fn new(
        browser: Arc<BrowserSession>,
        page: u32,
        output_files: OutputFileAccess,
        cancel: CancellationToken,
        catalog: Arc<Vec<ToolDef>>,
        session_id: SessionId,
        identity: ToolIdentity,
        default_tab_group_id: Option<String>,
        state: AppState,
    ) -> Option<Self> {
        let act_index = catalog.iter().position(|tool| tool.name == "act")?;
        Some(Self {
            browser,
            page: AtomicU32::new(page),
            output_files,
            cancel,
            catalog,
            act_index,
            session_id,
            identity,
            default_tab_group_id,
            state,
            notices: Mutex::new(Vec::new()),
            scroll_down_dead: AtomicBool::new(false),
            scroll_up_dead: AtomicBool::new(false),
        })
    }

    /// Anything the effects had to say about the pages this run touched.
    #[must_use]
    pub fn notices(&self) -> Vec<String> {
        self.notices
            .lock()
            .map(|notices| notices.clone())
            .unwrap_or_default()
    }

    /// The page the run is on now, which is not always the one it started on.
    #[must_use]
    pub fn page(&self) -> u32 {
        self.page.load(Ordering::Relaxed)
    }

    /// Page ids currently open, or None if they could not be read. A failure to
    /// read them must not fail the action that was already carried out.
    async fn open_page_ids(&self) -> Option<Vec<u32>> {
        self.browser
            .pages
            .list()
            .await
            .ok()
            .map(|pages| pages.into_iter().map(|page| page.page_id.0).collect())
    }

    fn call_for(&self, args: Value) -> ToolCall {
        ToolCall::new(
            self.catalog.clone(),
            self.act_index,
            args,
            self.session_id.clone(),
            Some(self.identity.clone()),
            Some(self.browser.clone()),
            self.cancel.child_token(),
            self.cancel.clone(),
            self.cancel.child_token(),
            self.default_tab_group_id.clone(),
            self.state.clone(),
            self.output_files.clone(),
        )
    }
}

impl Driver for PageDriver {
    fn stopped(&self) -> bool {
        self.cancel.is_cancelled()
    }

    async fn observe(&self) -> Result<Observation, String> {
        let snapshot = self
            .browser
            .observe(PageId(self.page()))
            .await
            .snapshot_with_options(SnapshotOptions {
                mode: SnapshotMode::Interactive,
                depth: None,
            })
            .await
            .map_err(|error| error.to_string())?;

        let mut elements: Vec<Element> = snapshot
            .refs
            .entries_in_order()
            .into_iter()
            .map(|entry| {
                Element::new(
                    entry.ref_id.0.clone(),
                    entry.role.clone(),
                    entry.name.clone(),
                )
            })
            .collect();

        // The ref table carries role and name but not the current value, which
        // only exists on the rendered line. Read it across before the tree is
        // trimmed, so a candidate stays self describing either way.
        annotate_from_tree(&mut elements, &snapshot.text);

        // Both scroll directions are offered without checking the scroll
        // position, which costs one wasted decision at a boundary at most: a
        // scroll that changes nothing leaves progress flat, and the loop's
        // stall rule ends a run that keeps doing it.
        // The page's own title, not the first line of its tree. Reading the
        // tree gave the caller things like `navigation "Shortcuts menu"` where
        // a title was expected, while the real one was a lookup away.
        let title = self
            .browser
            .pages
            .get_info(PageId(self.page()))
            .await
            .map(|info| info.title)
            .unwrap_or_default();

        Ok(Observation {
            url: snapshot.url.clone(),
            title,
            tree: snapshot.text,
            space: ActionSpace::new(
                elements,
                !self.scroll_down_dead.load(Ordering::Relaxed),
                !self.scroll_up_dead.load(Ordering::Relaxed),
            ),
        })
    }

    async fn act(&self, operation: Operation, target: Option<&str>) -> Result<bool, String> {
        let args = match operation {
            Operation::Click => json!({
                "page": self.page(),
                "kind": "click",
                "ref": target.ok_or_else(|| "click needs a target".to_string())?,
                "diff": "summary",
            }),
            Operation::ScrollDown => json!({
                "page": self.page(), "kind": "scroll", "direction": "down", "diff": "summary",
            }),
            Operation::ScrollUp => json!({
                "page": self.page(), "kind": "scroll", "direction": "up", "diff": "summary",
            }),
            // Waiting touches nothing. It used to be sent as a zero notch
            // scroll, which was a browser round trip pretending to be a pause
            // and produced no diff to read.
            Operation::Wait => {
                tokio::time::sleep(WAIT_PAUSE).await;
                return Ok(false);
            }
            // Typing hands back for a value, and the terminal operations never
            // reach the browser, so nothing else can arrive here.
            other => return Err(format!("{} is not executed here", other.as_str())),
        };

        // Read before the action so a page it opens can be told apart from one
        // that was already there.
        let before = self.open_page_ids().await;
        let result = dispatch_tool_call_in_process(self.call_for(args))
            .await
            .map_err(|error| error.to_string())?;
        if result.is_error {
            return Err(first_text(&result.content));
        }
        // An ownership notice is about a page the caller now has, so it travels
        // out with the result rather than being dropped here.
        for notice in notices_in(&result.content) {
            if let Ok(mut collected) = self.notices.lock()
                && !collected.contains(&notice)
            {
                collected.push(notice);
            }
        }
        // A link that opens in a new tab, which is what a product link does on
        // most shopping sites, leaves the page the run was watching genuinely
        // unchanged while the work has moved elsewhere. Following it is the
        // difference between a run that continues and one that repeats the same
        // click until its budget runs out, leaving a duplicate tab behind each
        // time.
        if let (Some(before), Some(after)) = (before, self.open_page_ids().await)
            && let Some(opened) = newly_opened(&before, &after)
        {
            // Claimed the same way a page opened through tabs is, so it joins the
            // caller's tab group and does not read back as the user's own tab.
            // A new page scrolls on its own terms, so both directions are
            // offered again.
            self.scroll_down_dead.store(false, Ordering::Relaxed);
            self.scroll_up_dead.store(false, Ordering::Relaxed);
            crate::api::mcp::effects::ownership_claims::record_new_page(
                &self.state,
                &self.identity,
                Some(&self.browser),
                self.session_id.as_str(),
                opened,
                crate::clock::now_epoch_ms(),
            )
            .await;
            self.page.store(opened, Ordering::Relaxed);
            return Ok(true);
        }
        let changed = changed_from(&result);
        match retired_direction(operation, changed) {
            Some(Operation::ScrollDown) => self.scroll_down_dead.store(true, Ordering::Relaxed),
            Some(Operation::ScrollUp) => self.scroll_up_dead.store(true, Ordering::Relaxed),
            _ => {}
        }
        Ok(changed)
    }
}

/// What the decision provider charged for a run, read back off its own result.
///
/// The audit's token totals measure a session's tool traffic, which is what an
/// agent pays. A goal-driven run also pays a provider, and recording only the
/// first made a run that spent hundreds of thousands of provider tokens look
/// like one that spent none.
#[must_use]
pub fn decision_tokens_in(result: &ToolResult) -> (u64, u64) {
    let Some(structured) = result.structured_content.as_ref() else {
        return (0, 0);
    };
    let read = |key: &str| structured.get(key).and_then(Value::as_u64).unwrap_or(0);
    (read("inputTokens"), read("outputTokens"))
}

/// The scroll direction to stop offering, if this action retired one.
///
/// A page that fits the viewport cannot scroll, and offering both directions
/// anyway cost one wasted decision per direction per step until the budget ran
/// out. A direction that moved nothing once will not move anything later on the
/// same page, so it is withdrawn rather than offered again.
fn retired_direction(operation: Operation, changed: bool) -> Option<Operation> {
    if changed {
        return None;
    }
    match operation {
        Operation::ScrollDown | Operation::ScrollUp => Some(operation),
        _ => None,
    }
}

/// The page an action opened, if it opened one.
///
/// The highest new id is the most recently opened, which is the one the action
/// just produced when a single click somehow spawns several.
fn newly_opened(before: &[u32], after: &[u32]) -> Option<u32> {
    after
        .iter()
        .copied()
        .filter(|page| !before.contains(page))
        .max()
}

/// How long a wait pauses for before the page is read again.
const WAIT_PAUSE: std::time::Duration = std::time::Duration::from_millis(300);

/// Whether the action changed the page, read from the diff's own flags.
///
/// This signal has now broken twice, in both directions. First the prose was
/// matched against a phrase that does not exist, so every no-op reported a
/// change. Then the structured flag was read through the wire envelope, which
/// strips it for a tool with no output schema, so every action including a
/// navigation reported no change. Both went into the trail and into the history
/// later decisions are shown, so a run was told nothing it did had any effect.
///
/// A navigation is therefore read from two independent fields: it takes both of
/// them being wrong to lose it again.
fn changed_from(result: &ToolResult) -> bool {
    let Some(structured) = result.structured_content.as_ref() else {
        // No diff was asked for or none came back, so there is nothing to
        // claim. Reporting no change is the honest default.
        return false;
    };
    let changed = structured
        .get("changed")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let url_changed = structured
        .get("urlChanged")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let moved = match (structured.get("beforeUrl"), structured.get("afterUrl")) {
        (Some(before), Some(after)) => !before.is_null() && !after.is_null() && before != after,
        _ => false,
    };
    changed || url_changed || moved
}

fn first_text(content: &[ContentBlock]) -> String {
    content
        .iter()
        .find_map(|block| match block {
            rmcp::model::ContentBlock::Text(text) => Some(text.text.clone()),
            _ => None,
        })
        .unwrap_or_default()
}

/// Notices an effect appended to an action's result.
///
/// The effects add these as extra text blocks after the tool's own output, so
/// anything that reads as a notice is collected and the action's own readback
/// is left alone.
fn notices_in(content: &[ContentBlock]) -> Vec<String> {
    content
        .iter()
        .filter_map(|block| match block {
            ContentBlock::Text(text) => Some(text.text.clone()),
            _ => None,
        })
        .filter(|text| text.starts_with("Note:") || text.starts_with("note:"))
        .collect()
}

/// The page title, read off the rendered tree's root line when it has one.
/// What the caller is told, as text and as structured content.
///
/// The outcome carries evidence rather than a verdict. A done decision is not
/// proof, so the caller gets the page and url it finished on, what ran, and the
/// confidence behind each step, and verifies in one turn of its own.
#[must_use]
pub fn render(goal: &str, page: u32, outcome: &Outcome, notices: &[String]) -> (String, Value) {
    let mut lines = Vec::new();
    lines.push(match &outcome.status {
        Status::Satisfied => {
            "The goal looks satisfied. Verify it yourself: a decision is not evidence.".to_string()
        }
        Status::Blocked => {
            "Stopped: no supported operation could make progress on this page. The page tools \
             still work, so continue with snapshot and act."
                .to_string()
        }
        Status::NeedsText { reference, field } => format!(
            "Needs a value for {field}. Type it with act kind=\"fill\" ref=\"{reference}\", then call browse again to continue."
        ),
        Status::OutOfBudget { steps, elapsed_ms } => format!(
            "Stopped after {steps} steps and {elapsed_ms}ms without finishing. Continue with the page tools, or call browse again."
        ),
        Status::Stalled => {
            "Stopped: the run was no longer making progress. Continue with the page tools."
                .to_string()
        }
        Status::Unconfirmed { satisfied, progress } => format!(
            "The run stopped claiming the goal is done, at confidence {satisfied:.2} and progress {progress:.2}, below the bar to assert it. Neither number is evidence: check the facts below against your goal, and read the page."
        ),
        Status::Stopped => {
            "Stopped by the operator. Nothing further was run; the page is where it was left."
                .to_string()
        }
        Status::Refused(reason) => format!(
            "Stopped without acting: {reason}. Continue with the page tools."
        ),
        Status::Failed(reason) => format!(
            "Stopped: {reason}. The page tools still work, so continue with snapshot and act."
        ),
    });
    let actions = outcome.actions();
    let changed = outcome
        .trail
        .iter()
        .filter(|entry| entry.page_changed)
        .count();
    lines.push(format!(
        "what changed: {changed} of {actions} actions changed the page. If that does not match what your goal asked for, the goal is not done, whatever the status says."
    ));
    lines.push(format!(
        "goal: {goal}\nat: page {} {} ({})\ndecisions: {} over {} actions, {}ms, {} input and {} output tokens",
        page,
        outcome.url,
        outcome.title,
        outcome.decisions,
        actions,
        outcome.elapsed_ms,
        outcome.input_tokens,
        outcome.output_tokens,
    ));
    // Anything the effects said about the pages this run touched. The caller
    // owns those pages now, so these belong in its result.
    if !notices.is_empty() {
        lines.push("about the pages used:".to_string());
        for notice in notices {
            lines.push(format!("  {notice}"));
        }
    }
    if !outcome.trail.is_empty() {
        lines.push("what ran:".to_string());
        for (index, entry) in outcome.trail.iter().enumerate() {
            lines.push(format!("  {}. {}", index + 1, describe(entry)));
        }
    }

    let structured = json!({
        "status": status_name(&outcome.status),
        // Where the run actually finished, which is not where it started once an
        // action opened a new tab. Without it a caller had to list tabs and guess
        // which one held the result.
        "page": page,
        "url": outcome.url,
        "title": outcome.title,
        "decisions": outcome.decisions,
        "actions": outcome.actions(),
        "elapsedMs": outcome.elapsed_ms,
        "inputTokens": outcome.input_tokens,
        "outputTokens": outcome.output_tokens,
        "needsText": match &outcome.status {
            Status::NeedsText { reference, field } => json!({ "ref": reference, "field": field }),
            _ => Value::Null,
        },
        "notices": notices,
        "trail": outcome.trail.iter().map(|entry| json!({
            "operation": entry.operation.as_str(),
            "target": entry.target,
            "operationConfidence": entry.operation_confidence,
            "targetConfidence": entry.target_confidence,
            "satisfied": entry.satisfied,
            "progress": entry.progress,
            "pageChanged": entry.page_changed,
            "latencyMs": entry.latency_ms,
            "droppedTargets": entry.dropped_targets,
        })).collect::<Vec<_>>(),
    });
    (lines.join("\n"), structured)
}

fn describe(entry: &TrailEntry) -> String {
    let target = entry
        .target
        .as_deref()
        .map(|reference| format!(" {reference}"))
        .unwrap_or_default();
    let changed = if entry.page_changed {
        "page changed"
    } else {
        "no change"
    };
    // A step that chose from a capped list chose from part of the page. Saying
    // so in the text matters: a caller reading only the prose had no way to know
    // the decision never saw the rest.
    let capped = if entry.dropped_targets > 0 {
        format!(", {} targets not offered", entry.dropped_targets)
    } else {
        String::new()
    };
    format!(
        "{}{target} (confidence {:.2}, {changed}, {}ms{capped})",
        entry.operation.as_str(),
        entry.operation_confidence,
        entry.latency_ms
    )
}

fn status_name(status: &Status) -> &'static str {
    match status {
        Status::Satisfied => "satisfied",
        Status::Blocked => "blocked",
        Status::NeedsText { .. } => "needs_text",
        Status::OutOfBudget { .. } => "out_of_budget",
        Status::Stalled => "stalled",
        Status::Unconfirmed { .. } => "unconfirmed",
        Status::Stopped => "stopped",
        Status::Refused(_) => "refused",
        Status::Failed(_) => "failed",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use browseros_policy::drive::Outcome;

    fn outcome(status: Status, trail: Vec<TrailEntry>) -> Outcome {
        Outcome {
            status,
            // One more than the trail: the decision that ended the run made no
            // action of its own, which is the case the old counter lost.
            decisions: u32::try_from(trail.len())
                .unwrap_or(u32::MAX)
                .saturating_add(1),
            trail,
            url: "https://example.com/results".to_string(),
            title: "Results".to_string(),
            elapsed_ms: 2400,
            input_tokens: 3200,
            output_tokens: 410,
        }
    }

    fn entry() -> TrailEntry {
        TrailEntry {
            operation: Operation::Click,
            target: Some("e7".to_string()),
            operation_confidence: 0.82,
            target_confidence: Some(0.91),
            satisfied: 0.3,
            progress: 1.0,
            page_changed: true,
            latency_ms: 310,
            input_tokens: Some(1200),
            output_tokens: Some(180),
            dropped_targets: 0,
        }
    }

    /// A satisfied run must not read as proof, because a decision is not
    /// evidence and the caller is the one that can verify.
    #[test]
    fn a_satisfied_run_tells_the_caller_to_verify() {
        let (text, structured) = render(
            "Find a flight.",
            25,
            &outcome(Status::Satisfied, vec![entry()]),
            &[],
        );
        assert!(text.contains("Verify it yourself"), "{text}");
        assert_eq!(structured["status"], json!("satisfied"));
        assert_eq!(structured["url"], json!("https://example.com/results"));
        assert_eq!(structured["trail"][0]["operation"], json!("CLICK"));
        assert_eq!(structured["trail"][0]["target"], json!("e7"));
    }

    /// Every stop that is not success has to name the way forward, since the
    /// caller still owns the tabs and can carry on by hand.
    #[test]
    fn every_other_ending_names_the_fallback() {
        let endings = [
            Status::Blocked,
            Status::OutOfBudget {
                steps: 4,
                elapsed_ms: 900,
            },
            Status::Stalled,
            Status::Refused("a target was not offered".to_string()),
            Status::Failed("provider returned 503".to_string()),
        ];
        for status in endings {
            let (text, _) = render(
                "Find a flight.",
                25,
                &outcome(status.clone(), Vec::new()),
                &[],
            );
            assert!(
                text.contains("page tools") || text.contains("browse again"),
                "{status:?} did not name a way forward: {text}"
            );
        }
    }

    /// The gap this closes: a run that followed a link into a new tab reported
    /// the page it started on, so the caller had to list tabs and guess which
    /// one held the result.
    #[test]
    fn the_result_names_the_page_the_run_finished_on() {
        let (text, structured) = render(
            "Open the product page.",
            29,
            &outcome(Status::Satisfied, vec![entry()]),
            &[],
        );
        assert_eq!(structured["page"], json!(29));
        assert!(
            text.contains("page 29"),
            "the model reads the text, not the structured block: {text}"
        );
    }

    /// An unconfirmed ending must not read like a failure. The run believes the
    /// goal is met and the page is sitting on the result, so the caller is told
    /// to look rather than told the run went nowhere.
    #[test]
    fn an_unconfirmed_ending_points_at_the_page_not_at_a_failure() {
        let (text, structured) = render(
            "Open the product page.",
            29,
            &outcome(
                Status::Unconfirmed {
                    satisfied: 0.42,
                    progress: 0.98,
                },
                vec![entry()],
            ),
            &[],
        );
        assert_eq!(structured["status"], json!("unconfirmed"));
        assert!(text.contains("0.42"), "the confidence is shown: {text}");
        assert!(
            text.contains("0.98"),
            "the progress that justified this ending is shown too: {text}"
        );
        assert!(
            !text.contains("looks met"),
            "the ending must not assert the goal is met: a run that applied one of two \
             filters reported progress 1.00, so the claim cannot be made from these \
             numbers: {text}"
        );
        assert!(
            text.contains("1 of 1 actions changed the page"),
            "the ending carries what the run observed, which a caller can check \
             against the goal: {text}"
        );
        assert!(
            text.contains("read the page"),
            "the caller is pointed at the page: {text}"
        );
        assert!(
            !text.contains("no longer making progress"),
            "an unconfirmed finish is not a stall: {text}"
        );
        assert!(
            structured["url"]
                .as_str()
                .is_some_and(|url| !url.is_empty()),
            "the page it finished on has to be in the result"
        );
    }

    /// An operator Stop is the one ending that must NOT invite the caller to
    /// carry on, because the session it would carry on in has been cancelled.
    #[test]
    fn an_operator_stop_does_not_invite_the_caller_to_continue() {
        let (text, structured) = render(
            "Find a flight.",
            25,
            &outcome(Status::Stopped, vec![entry()]),
            &[],
        );
        assert_eq!(structured["status"], json!("stopped"));
        assert!(text.contains("Stopped by the operator"), "{text}");
        assert!(
            !text.contains("continue with") && !text.contains("browse again"),
            "a cancelled session is not something to resume: {text}"
        );
    }

    #[test]
    fn needing_text_names_the_field_and_the_exact_next_call() {
        let (text, structured) = render(
            "Fill the form.",
            25,
            &outcome(
                Status::NeedsText {
                    reference: "e4".to_string(),
                    field: "[e4] textbox Email".to_string(),
                },
                Vec::new(),
            ),
            &[],
        );
        assert!(text.contains("[e4] textbox Email"), "{text}");
        assert!(text.contains("kind=\"fill\""), "{text}");
        assert_eq!(structured["needsText"]["ref"], json!("e4"));
    }

    #[test]
    fn the_trail_carries_the_cost_and_the_confidence() {
        let (text, structured) = render(
            "Anything.",
            25,
            &outcome(Status::Stalled, vec![entry()]),
            &[],
        );
        assert!(text.contains("3200 input"), "{text}");
        assert!(text.contains("confidence 0.82"), "{text}");
        assert_eq!(structured["trail"][0]["targetConfidence"], json!(0.91));
        // Reported apart, because an ending that acted on nothing still paid
        // for the decision that produced it.
        assert_eq!(structured["actions"], json!(1));
        assert_eq!(structured["decisions"], json!(2));
        assert!(text.contains("2 over 1 actions"), "{text}");
    }

    /// The bug this closes: an unchanged diff says "no change since last
    /// snapshot", so matching on "no changes" never fired and every no-op was
    /// reported as a change.
    #[test]
    fn page_changed_comes_from_the_diff_flag_not_its_prose() {
        let unchanged = ToolResult::text(
            "no change since last snapshot",
            Some(json!({ "changed": false })),
        );
        assert!(!changed_from(&unchanged));

        let changed = ToolResult::text(
            "2 added, 1 removed",
            Some(json!({ "changed": true, "added": 2, "removed": 1 })),
        );
        assert!(changed_from(&changed));

        // Nothing to go on is reported as no change rather than guessed.
        let silent = ToolResult::text("done", None);
        assert!(!changed_from(&silent));
    }

    /// The cost this closes: both scroll directions were offered on every step
    /// regardless of whether the page could scroll, so a page that fits the
    /// viewport burned a decision per direction per step. Observed as four dead
    /// scrolls on one short page.
    #[test]
    fn a_scroll_that_moves_nothing_retires_that_direction() {
        assert_eq!(
            retired_direction(Operation::ScrollDown, false),
            Some(Operation::ScrollDown)
        );
        assert_eq!(
            retired_direction(Operation::ScrollUp, false),
            Some(Operation::ScrollUp)
        );

        // A scroll that worked stays available.
        assert_eq!(retired_direction(Operation::ScrollDown, true), None);

        // Only scrolling is withdrawn. A click that changes nothing may still be
        // the right thing to try on a control that needs two.
        assert_eq!(retired_direction(Operation::Click, false), None);
        assert_eq!(retired_direction(Operation::Wait, false), None);
    }

    /// The bug this closes: a click on a product link opens a new tab, so the
    /// page the run was watching did not change and the run clicked the same
    /// link until its budget ran out, leaving a duplicate tab behind each time.
    #[test]
    fn the_page_an_action_opened_is_the_one_to_follow() {
        assert_eq!(newly_opened(&[28], &[28, 29]), Some(29));

        // Several at once: the newest is the one this action produced.
        assert_eq!(newly_opened(&[28], &[28, 29, 30, 31]), Some(31));

        // Nothing opened, so the run stays where it is and the diff decides.
        assert_eq!(newly_opened(&[28, 29], &[28, 29]), None);

        // A page closing is not a page opening.
        assert_eq!(newly_opened(&[28, 29], &[28]), None);

        // Ids are not assumed to arrive in order.
        assert_eq!(newly_opened(&[40], &[44, 40, 41]), Some(44));
    }

    /// A navigation must survive either flag going missing, because losing it
    /// is what told a run that a click which changed the page did nothing.
    #[test]
    fn a_navigation_is_a_change_from_either_field() {
        let only_url_flag = ToolResult::text(
            "navigated",
            Some(json!({ "changed": false, "urlChanged": true })),
        );
        assert!(
            changed_from(&only_url_flag),
            "urlChanged alone must count as a change"
        );

        let only_urls = ToolResult::text(
            "navigated",
            Some(json!({
                "beforeUrl": "https://example.com/",
                "afterUrl": "https://www.iana.org/help/example-domains",
            })),
        );
        assert!(
            changed_from(&only_urls),
            "differing before and after urls must count as a change"
        );

        let same_urls = ToolResult::text(
            "no change since last snapshot",
            Some(json!({
                "changed": false,
                "beforeUrl": "https://example.com/",
                "afterUrl": "https://example.com/",
            })),
        );
        assert!(!changed_from(&same_urls));

        let null_urls = ToolResult::text(
            "no change since last snapshot",
            Some(json!({ "changed": false, "beforeUrl": null, "afterUrl": null })),
        );
        assert!(
            !changed_from(&null_urls),
            "two absent urls are not two different urls"
        );
    }

    /// An action on a page the caller does not own produces an ownership notice,
    /// and a run makes that action on the caller's behalf, so the notice has to
    /// travel out with the run's result rather than being dropped.
    #[test]
    fn notices_from_the_actions_reach_the_caller() {
        let notices = vec![
            "Note: page 7 belongs to another agent (Cowork).".to_string(),
            "note: tab group abc is not open.".to_string(),
        ];
        let (text, structured) = render(
            "Find a flight.",
            25,
            &outcome(Status::Stalled, vec![entry()]),
            &notices,
        );
        assert!(text.contains("about the pages used"), "{text}");
        assert!(text.contains("belongs to another agent"), "{text}");
        assert_eq!(structured["notices"], json!(notices));

        // Nothing to say means nothing is said.
        let (quiet, _) = render(
            "Find a flight.",
            25,
            &outcome(Status::Stalled, vec![entry()]),
            &[],
        );
        assert!(!quiet.contains("about the pages used"), "{quiet}");
    }

    /// Only notices are collected, not the action's own readback.
    #[test]
    fn only_notice_blocks_are_collected() {
        let content = vec![
            ContentBlock::text("3 added, 1 removed"),
            ContentBlock::text("Note: page 7 belongs to another agent (Cowork)."),
            ContentBlock::text("note: tab group abc is not open."),
        ];
        assert_eq!(
            notices_in(&content),
            vec![
                "Note: page 7 belongs to another agent (Cowork).".to_string(),
                "note: tab group abc is not open.".to_string()
            ]
        );
    }
}
