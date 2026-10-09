//! The browser side of the decision loop, and how its outcome is reported.
//!
//! Every operation reaches the page through the ordinary `act` tool, down the
//! same dispatch path a connected agent uses: the same guards, the same
//! effects, the same audit record. Nothing here talks to the browser directly,
//! which is why adding an operation is a change to the vocabulary rather than
//! to the browser.

use std::sync::{Arc, Mutex};

use browseros_core::{
    BrowserSession, PageId,
    snapshot::{SnapshotMode, SnapshotOptions},
};
use browseros_decide::gate::Decision;
use browseros_decide::operations::Operation;
use browseros_decide::pursue::{ActError, Driver, Outcome, Status};
use browseros_decide::view::PageView;
use browseros_mcp::{OutputFileAccess, ToolDef};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use crate::AppState;
use crate::api::mcp::dispatch::{ToolCall, ToolIdentity, dispatch_tool_call};
use crate::ids::SessionId;

/// How long to let a page settle after an input.
///
/// In code rather than by asking the model to wait: a wait decision costs a
/// whole request, and this costs milliseconds.
const SETTLE: std::time::Duration = std::time::Duration::from_millis(120);

/// The browser, as the loop needs it.
pub struct PageDriver {
    session: Arc<BrowserSession>,
    page: u32,
    act_index: usize,
    catalog: Arc<Vec<ToolDef>>,
    session_id: SessionId,
    identity: ToolIdentity,
    default_tab_group_id: Option<String>,
    state: AppState,
    output_files: OutputFileAccess,
    cancel: CancellationToken,
    notices: Mutex<Vec<String>>,
}

impl PageDriver {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        session: Arc<BrowserSession>,
        page: u32,
        catalog: Arc<Vec<ToolDef>>,
        session_id: SessionId,
        identity: ToolIdentity,
        default_tab_group_id: Option<String>,
        state: AppState,
        output_files: OutputFileAccess,
        cancel: CancellationToken,
    ) -> Option<Self> {
        let act_index = catalog.iter().position(|tool| tool.name == "act")?;
        Some(Self {
            session,
            page,
            act_index,
            catalog,
            session_id,
            identity,
            default_tab_group_id,
            state,
            output_files,
            cancel,
            notices: Mutex::new(Vec::new()),
        })
    }

    pub fn page(&self) -> u32 {
        self.page
    }

    pub fn notices(&self) -> Vec<String> {
        self.notices
            .lock()
            .map(|held| held.clone())
            .unwrap_or_default()
    }

    /// Builds the arguments for one operation.
    fn args_for(&self, decision: &Decision) -> Option<Value> {
        let kind = decision.operation.act_kind()?;
        let mut args = json!({ "page": self.page, "kind": kind, "diff": "summary" });
        match decision.operation {
            Operation::ScrollDown => args["direction"] = json!("down"),
            Operation::ScrollUp => args["direction"] = json!("up"),
            _ => {
                args["ref"] = json!(decision.target.as_deref()?);
            }
        }
        if let Some(value) = &decision.value {
            args["value"] = json!(value);
        }
        if let Some(key) = &decision.key {
            args["key"] = json!(key);
        }
        Some(args)
    }

    fn call_for(&self, args: Value) -> ToolCall {
        // Child tokens, not clones. A clone shares one cancellation state, and
        // dispatch cancels the token it was given on every completion path
        // including success, so a clone of the run's own token would be
        // cancelled by the first action: `stopped` would then be true and the
        // run would end after one step. A child is cancelled by its parent but
        // never the other way round, which is the direction wanted here.
        ToolCall::new(
            self.catalog.clone(),
            self.act_index,
            args,
            self.session_id.clone(),
            Some(self.identity.clone()),
            Some(self.session.clone()),
            self.cancel.child_token(),
            self.cancel.child_token(),
            CancellationToken::new(),
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

    async fn observe(&self) -> Result<PageView, String> {
        let observer = self.session.observe(PageId(self.page)).await;
        let snapshot = observer
            .snapshot_with_options(SnapshotOptions {
                mode: SnapshotMode::Full,
                depth: None,
            })
            .await
            .map_err(|error| error.to_string())?;
        let title = self
            .session
            .pages
            .get_info(PageId(self.page))
            .await
            .map(|info| info.title)
            .unwrap_or_default();
        let mut view = PageView::from_snapshot(snapshot.url, title, &snapshot.text, &snapshot.refs);
        // The measured cost of this design assumes a budgeted state. Applying
        // it here rather than in each caller means a real run gets the same
        // state a measurement does.
        view.budget_text(PageView::TEXT_BUDGET);
        Ok(view)
    }

    async fn act(&self, decision: &Decision) -> Result<(), ActError> {
        let Some(args) = self.args_for(decision) else {
            return Err(ActError::Fatal(format!(
                "{} does not reach the browser",
                decision.operation.as_str()
            )));
        };
        // A key press carries no target: act sends the key to whatever has
        // focus. So the chosen control is focused first, and a failure to focus
        // stops the step rather than pressing a key at something else.
        if decision.operation == Operation::Press
            && let Some(reference) = decision.target.as_deref()
        {
            let focus = dispatch_tool_call(self.call_for(json!({
                "page": self.page,
                "kind": "focus",
                "ref": reference,
            })))
            .await
            .map_err(|error| ActError::Fatal(error.to_string()))?;
            if focus.is_error.unwrap_or(false) {
                let reason = first_text(&focus);
                return Err(if covered_or_gone(&reason) {
                    ActError::Stale(reason)
                } else {
                    ActError::Fatal(reason)
                });
            }
        }
        let result = dispatch_tool_call(self.call_for(args))
            .await
            .map_err(|error| ActError::Fatal(error.to_string()))?;

        let text = first_text(&result);
        if result.is_error.unwrap_or(false) {
            // A control that moved or is covered is worth another look. A
            // mutation whose completion is unknown is not: a select whose change
            // event may already have fired would be set twice by a retry.
            if decision.operation == Operation::Select && !text.contains("covered") {
                return Err(ActError::Fatal(text));
            }
            if covered_or_gone(&text) {
                return Err(ActError::Stale(text));
            }
            return Err(ActError::Fatal(text));
        }
        for notice in notices_in(&text) {
            if let Ok(mut held) = self.notices.lock()
                && !held.contains(&notice)
            {
                held.push(notice);
            }
        }
        Ok(())
    }

    async fn settle(&self) {
        tokio::time::sleep(SETTLE).await;
    }
}

/// Whether a failure is one another observation could clear.
///
/// Matched on what the browser's errors actually say, read from
/// `browseros_core::error`, not on words guessed at. Two of the three patterns
/// here were previously wrong: the error says `Stale ref` with a capital and
/// `Unknown ref` rather than "no ref", so a lost reference was treated as fatal
/// and the retry path it exists for never ran.
///
/// The two reference errors and the capture error all end by saying what to do,
/// and that instruction is the most reliable signal available: an error that
/// asks for a new snapshot is telling us a new snapshot would help.
fn covered_or_gone(reason: &str) -> bool {
    reason.contains("take a new snapshot")
        || reason.contains("; retry.")
        || reason.contains("is covered by")
}

fn first_text(result: &rmcp::model::CallToolResult) -> String {
    result
        .content
        .iter()
        .filter_map(|block| block.as_text().map(|text| text.text.clone()))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Anything the dispatch said about pages the caller now owns.
fn notices_in(text: &str) -> Vec<String> {
    text.lines()
        .filter(|line| line.contains("belongs to") || line.contains("now yours"))
        .map(str::to_string)
        .collect()
}

/// Renders an outcome for the caller.
///
/// Every ending carries what the run observed rather than only a label: the url
/// it started on, the url it ended on, and how many of its actions changed the
/// page. A caller can check that against its goal, which it cannot do with a
/// status alone.
#[must_use]
pub fn render(goal: &str, page: u32, outcome: &Outcome, notices: &[String]) -> (String, Value) {
    let mut lines = Vec::new();
    lines.push(match &outcome.status {
        Status::Satisfied => format!(
            "The goal looks satisfied, on a page that had not moved since the run looked at it, \
             claimed at confidence {}. That is a claim and not evidence, so verify it yourself; \
             the lower the confidence, the more that matters.",
            confidence_text(outcome.terminal_confidence)
        ),
        Status::Blocked(reason) => format!(
            "Stopped: {reason}, at confidence {}. The page tools still work, so continue with \
             snapshot and act.",
            confidence_text(outcome.terminal_confidence)
        ),
        Status::Stalled(cause) => format!(
            "Stopped making progress: {}. Continue with the page tools.",
            cause.describe(outcome.actions())
        ),
        Status::NeedsInput(reason) => format!(
            "Stopped and handing back: {reason}. You decide what happens next: this run has one \
             page and one goal, and you have the wider intent. Options are to scroll or expand a \
             facet with snapshot and act, restate the goal, or stop."
        ),
        Status::OutOfBudget => {
            "Stopped at the run's budget without finishing. Call pursue again to continue."
                .to_string()
        }
        Status::Stopped => "Stopped by the operator. The page is where it was left.".to_string(),
        Status::Failed(reason) => format!(
            "Stopped: {reason}. The page tools still work, so continue with snapshot and act."
        ),
    });
    lines.push(format!(
        "what changed: {} of {} actions changed the page. If that does not match what your goal \
         asked for, the goal is not done, whatever the status says.",
        outcome.changed(),
        outcome.actions()
    ));
    if outcome.offered > 0 || outcome.omitted > 0 {
        lines.push(format!(
            "what the last decision could see: {} controls offered, {} held back by the cap.",
            outcome.offered, outcome.omitted
        ));
    }
    if !outcome.offered_controls.is_empty() {
        lines.push("it was choosing between these, and said none of them would do:".to_string());
        for control in &outcome.offered_controls {
            lines.push(format!("  {control}"));
        }
    }
    lines.push(format!(
        "goal: {goal}\nat: page {page} {} ({})\nstarted at: {}\ndecisions: {} over {} actions, \
         {} input tokens",
        outcome.url_after,
        outcome.title,
        outcome.url_before,
        outcome.decisions,
        outcome.actions(),
        outcome.input_tokens,
    ));
    if !notices.is_empty() {
        lines.push("about the pages used:".to_string());
        for notice in notices {
            lines.push(format!("  {notice}"));
        }
    }

    let structured = json!({
        "status": status_name(&outcome.status),
        "stoppedBecause": stopped_because(&outcome.status),
        "page": page,
        "url": outcome.url_after,
        "urlBefore": outcome.url_before,
        "title": outcome.title,
        "decisions": outcome.decisions,
        "actions": outcome.actions(),
        "actionsThatChangedThePage": outcome.changed(),
        "inputTokens": outcome.input_tokens,
        "controlsOffered": outcome.offered,
        "controlsHeldBack": outcome.omitted,
        "offeredControls": outcome.offered_controls,
        "terminalConfidence": outcome.terminal_confidence,
        "trail": outcome.trail.iter().map(|step| json!({
            "operation": step.operation.as_str(),
            "target": step.target,
            "value": step.value,
            "confidence": step.confidence,
            "pageChanged": step.page_changed,
            "url": step.url_after,
        })).collect::<Vec<_>>(),
    });
    (lines.join("\n"), structured)
}

/// Reads a terminal confidence for a caller, saying plainly when there is none
/// rather than printing a zero that would look like certainty of the opposite.
fn confidence_text(confidence: Option<f64>) -> String {
    confidence.map_or_else(
        || "unknown".to_string(),
        |confidence| format!("{confidence:.2}"),
    )
}

/// The reason a run stopped making progress, as a short word a caller can
/// branch on rather than a sentence it would have to read.
fn stopped_because(status: &Status) -> Option<&'static str> {
    match status {
        Status::Stalled(cause) => Some(match cause {
            browseros_decide::Stuck::NothingChanged => "nothing_changed",
            browseros_decide::Stuck::NotConfident => "not_confident",
            browseros_decide::Stuck::ControlKeptMoving => "control_kept_moving",
            browseros_decide::Stuck::BrowserKeptFailing => "browser_kept_failing",
            browseros_decide::Stuck::OnlyWaiting => "only_waiting",
        }),
        _ => None,
    }
}

fn status_name(status: &Status) -> &'static str {
    match status {
        Status::Satisfied => "satisfied",
        Status::Blocked(_) => "blocked",
        Status::Stalled(_) => "stalled",
        Status::NeedsInput(_) => "needs_input",
        Status::OutOfBudget => "out_of_budget",
        Status::Stopped => "stopped",
        Status::Failed(_) => "failed",
    }
}

#[cfg(test)]
mod tests {
    use super::covered_or_gone;
    use browseros_core::{CoreError, Ref};

    /// The wordings this has to recognise come from the browser's own error
    /// type, so they are built from it rather than copied into a string here
    /// where they could drift apart from it silently.
    #[test]
    fn the_browsers_recoverable_errors_are_recognised() {
        let stale = CoreError::StaleRef {
            ref_id: Ref::from("e34".to_string()),
            role: "button".to_string(),
            name: "Apply".to_string(),
        };
        assert!(
            covered_or_gone(&stale.to_string()),
            "a stale reference is worth another snapshot: {stale}"
        );

        let unknown = CoreError::UnknownRef(Ref::from("e99".to_string()));
        assert!(
            covered_or_gone(&unknown.to_string()),
            "so is an unknown one: {unknown}"
        );

        let changed = CoreError::DocumentChanged;
        assert!(
            covered_or_gone(&changed.to_string()),
            "and a capture that raced the page: {changed}"
        );
    }

    /// The invariant the dispatch tokens rest on, which a clone gets wrong.
    ///
    /// Dispatch cancels the token it was handed on every completion path,
    /// success included. A clone shares one state, so handing dispatch a clone
    /// of the run's own token meant the first action cancelled the run: it
    /// would report stopped after one step. A child is cancelled by its parent
    /// and never the reverse.
    #[test]
    fn a_child_token_does_not_cancel_its_parent() {
        let run = tokio_util::sync::CancellationToken::new();

        let shared = run.clone();
        shared.cancel();
        assert!(
            run.is_cancelled(),
            "a clone shares one state, which is why it cannot be handed to dispatch"
        );

        let run = tokio_util::sync::CancellationToken::new();
        let child = run.child_token();
        child.cancel();
        assert!(
            !run.is_cancelled(),
            "a child cancelling leaves the run alone"
        );

        let run = tokio_util::sync::CancellationToken::new();
        let child = run.child_token();
        run.cancel();
        assert!(
            child.is_cancelled(),
            "and an operator stopping the run still reaches the action"
        );
    }

    /// A covered control is the case the dropdown work was about, and it is
    /// retryable because dismissing the cover may well succeed.
    #[test]
    fn a_covered_control_is_recognised() {
        let covered = "Element e101 (combobox \"Sort by:\") is covered by <div.nav-left> at its \
                       click point; the click would hit that element instead.";
        assert!(covered_or_gone(covered));
    }

    /// Something that is not recoverable must not be retried, or the loop would
    /// spin on it.
    #[test]
    fn an_unrecoverable_failure_is_not_recognised() {
        for reason in [
            "the browser is not connected",
            "Dropdown execution was not confirmed; inspect before retrying.",
            "page 7 does not belong to this session",
        ] {
            assert!(
                !covered_or_gone(reason),
                "{reason} is not cleared by looking again"
            );
        }
    }
}
