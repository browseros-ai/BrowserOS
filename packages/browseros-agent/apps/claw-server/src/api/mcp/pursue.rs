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
use browseros_decide::pursue::{ActError, Arrival, Driver, Journey, Outcome, Status};
use browseros_decide::run::{Asking, Pending};
use browseros_decide::view::PageView;
use browseros_mcp::{OutputFileAccess, ToolDef};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use crate::AppState;
use crate::api::mcp::dispatch::{ToolCall, ToolIdentity, dispatch_tool_call};
use crate::ids::SessionId;

/// A parked run and the one field it is waiting for, for a caller that asked
/// for the user to be asked rather than itself.
pub struct Ask {
    pub handle: String,
    pub reference: String,
    pub control: String,
}

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
    navigate_index: usize,
    tabs_index: usize,
    screenshot_index: usize,
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
        // Moving reaches the browser through the same two tools a connected
        // agent uses, so a destination gets the same guards, effects and audit
        // record as anything else.
        let navigate_index = catalog.iter().position(|tool| tool.name == "navigate")?;
        let tabs_index = catalog.iter().position(|tool| tool.name == "tabs")?;
        let screenshot_index = catalog.iter().position(|tool| tool.name == "screenshot")?;
        Some(Self {
            session,
            page,
            act_index,
            navigate_index,
            tabs_index,
            screenshot_index,
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

    /// A picture of the page, for a caller that asked for one.
    ///
    /// Not part of the driver trait: the loop has no use for an image, because
    /// the model it asks reads an accessibility tree. This is for the caller,
    /// which has eyes, and it goes down the same dispatch path as everything
    /// else so the capture is recorded like any other tool call.
    pub async fn look(&self) -> Option<rmcp::model::ContentBlock> {
        let result =
            dispatch_tool_call(self.call_to(self.screenshot_index, json!({ "page": self.page })))
                .await
                .ok()?;
        if result.is_error.unwrap_or(false) {
            return None;
        }
        result
            .content
            .into_iter()
            .find(|block| block.as_image().is_some())
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
        self.call_to(self.act_index, args)
    }

    fn call_to(&self, index: usize, args: Value) -> ToolCall {
        // Child tokens, not clones. A clone shares one cancellation state, and
        // dispatch cancels the token it was given on every completion path
        // including success, so a clone of the run's own token would be
        // cancelled by the first action: `stopped` would then be true and the
        // run would end after one step. A child is cancelled by its parent but
        // never the other way round, which is the direction wanted here.
        ToolCall::new(
            self.catalog.clone(),
            index,
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

    async fn travel(&self, journey: &Journey) -> Result<Arrival, ActError> {
        let new_tab = matches!(journey, Journey::To(destination) if destination.new_tab);
        let call = match journey {
            Journey::To(destination) if destination.new_tab => {
                // The group id is not optional. Without it a new tab lands
                // outside the caller's tab group, which is the signal the
                // caller uses to keep its own work together.
                let mut args = json!({ "action": "new", "url": destination.url });
                if let Some(group) = &self.default_tab_group_id {
                    args["groupId"] = json!(group);
                }
                self.call_to(self.tabs_index, args)
            }
            Journey::To(destination) => self.call_to(
                self.navigate_index,
                json!({ "page": self.page, "action": "url", "url": destination.url }),
            ),
            Journey::Back => self.call_to(
                self.navigate_index,
                json!({ "page": self.page, "action": "back" }),
            ),
        };
        let result = dispatch_tool_call(call)
            .await
            .map_err(|error| ActError::Fatal(error.to_string()))?;
        if result.is_error.unwrap_or(false) {
            // Never stale. A navigation whose completion is unknown must not be
            // attempted twice: the first may already have left the page.
            return Err(ActError::Fatal(first_text(&result)));
        }
        let text = first_text(&result);
        for notice in notices_in(&text) {
            if let Ok(mut held) = self.notices.lock()
                && !held.contains(&notice)
            {
                held.push(notice);
            }
        }
        let opened = if new_tab { opened_page(&text) } else { None };
        let url = match journey {
            // A page that has just been opened may still report about:blank, so
            // the url the caller asked for is the better answer than whatever
            // the tab happens to say in the same millisecond.
            Journey::To(destination) if destination.new_tab => destination.url.clone(),
            _ => self
                .session
                .pages
                .get_info(PageId(self.page))
                .await
                .map(|info| info.url)
                .unwrap_or_default(),
        };
        Ok(Arrival {
            page: self.page,
            url,
            opened,
        })
    }
}

/// The page id a new tab was opened as.
///
/// Read from the text, not from structured content, because the dispatch strips
/// structured content from every tool without an output schema and `tabs` has
/// none. Reading it from there was wrong twice before in this file, in both
/// directions, so the wording is pinned by a test instead of trusted.
fn opened_page(text: &str) -> Option<u32> {
    text.lines()
        .filter_map(|line| line.trim().strip_prefix("opened page "))
        .filter_map(|rest| {
            rest.split_whitespace()
                .next()
                .and_then(|word| word.parse::<u32>().ok())
        })
        .find(|page| *page >= 1)
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
///
/// `handle` is the continuation this run was parked under, when it parked. It
/// is printed as part of the literal next call rather than described, because a
/// caller that has to assemble the call from prose mostly starts a new run
/// instead.
#[must_use]
pub fn render(
    goal: &str,
    page: u32,
    outcome: &Outcome,
    notices: &[String],
    handle: Option<&str>,
) -> (String, Value) {
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
        Status::NeedsValue(Pending::Text {
            reference,
            control,
            confidence,
            asking,
            ..
        }) => format!(
            "Needs one value: it chose to type into {control} ({reference}) at confidence \
             {confidence:.2}, and it does not generate text.{}\nAnswer with one call:\n  \
             pursue(page: {page}, goal: <the same goal>{}, provide: {{ \"{reference}\": \
             \"<the text>\" }})\nThat carries this run on with its {} action(s) and its \
             budget intact, rather than starting again, and it re-reads the page and checks \
             {reference} has not changed before typing, discarding your answer rather than \
             typing it into something else. If you know values this run may need, pass them \
             as `values` next time and it will not stop to ask. If {control} is not the \
             field you meant, start a fresh run instead: the list below is what it was \
             choosing between.",
            match asking {
                Asking::FirstTime => String::new(),
                Asking::FieldChanged =>
                    " You answered this already, but the field changed before the text could be \
                     typed, so that answer was discarded."
                        .to_string(),
            },
            continue_argument(handle),
            outcome.actions(),
        ),
        // Handing back after changing the page is not handing back from a
        // standing start. Measured live: a run applied a facet filter at 0.93,
        // the url gained `rh=p_n_g-1003405005111%3A30060745031`, and the
        // hand-back then said none of the controls offered could advance the
        // goal. True of the step it reached, false as a summary, because the
        // reason nothing further could advance the goal was that the goal was
        // already met.
        Status::NeedsInput(reason) if outcome.changed() > 0 => format!(
            "Stopped and handing back after changing the page: {reason}. {} of {} actions \
             changed it, so check whether what the run already did was what you wanted before \
             redoing anything. You decide what happens next: this run has one page and one \
             goal, and you have the wider intent.",
            outcome.changed(),
            outcome.actions()
        ),
        Status::NeedsInput(reason) => format!(
            "Stopped and handing back: {reason}. You decide what happens next: this run has one \
             page and one goal, and you have the wider intent. Options are to scroll or expand a \
             facet with snapshot and act, restate the goal, or stop."
        ),
        // Two different endings wear this label, and they need opposite advice.
        // A run out of steps is handed back and can be continued with a higher
        // limit; a run out of time cannot be continued at all, because nothing
        // gives it more.
        Status::OutOfBudget if handle.is_some() => format!(
            "Stopped at its step limit of {} without finishing. Carry it on with one call:\n  \
             pursue(page: {page}, goal: <the same goal>{}, maxSteps: {})\nmaxSteps counts the \
             whole run, so it has to be higher than the {} actions already taken or the next \
             call ends where this one did.",
            outcome.actions(),
            continue_argument(handle),
            outcome.actions() * 2,
            outcome.actions(),
        ),
        Status::OutOfBudget => "Stopped at the run's time limit without finishing. Start a fresh \
             run: the time a run has spent travels with it, so continuing this one would end \
             the moment it started."
            .to_string(),
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
    // Said only when the run did not finish. On a satisfied run it would be
    // noise: whatever the pictures say, the goal was met without them.
    if outcome.unlabelled_images > 0 && !matches!(outcome.status, Status::Satisfied) {
        lines.push(format!(
            "what it could not read: {} image(s) on this page carry no accessible name, so \
             whatever they say is invisible to a run that reads the accessibility tree. Take a \
             screenshot of page {page} if the goal depends on them.",
            outcome.unlabelled_images
        ));
    }
    if !outcome.opened.is_empty() {
        lines.push(
            "it opened these beside the page it was working on, and they are yours to pursue \
             separately:"
                .to_string(),
        );
        for page in &outcome.opened {
            lines.push(format!("  page {} {}", page.page, page.url));
        }
    }
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
        "needsInput": needs_input(&outcome.status),
        "continueWith": handle,
        "openedPages": outcome.opened.iter().map(|page| json!({
            "page": page.page,
            "url": page.url,
        })).collect::<Vec<_>>(),
        "unlabelledImages": outcome.unlabelled_images,
        "navigations": outcome.trail.iter().filter(|step| matches!(
            step.operation,
            browseros_decide::Operation::GoTo | browseros_decide::Operation::GoBack
        )).count(),
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

/// The `continue` argument for the next call, printed only when there is a
/// handle to print.
///
/// A run can park without one if the store refused to hold it, and telling the
/// caller to pass a handle it does not have is worse than telling it nothing.
fn continue_argument(handle: Option<&str>) -> String {
    handle.map_or_else(String::new, |handle| format!(", continue: \"{handle}\""))
}

/// The value the run is waiting for, as something a caller can branch on rather
/// than a sentence it has to read.
fn needs_input(status: &Status) -> Option<Value> {
    let Status::NeedsValue(Pending::Text {
        reference,
        control,
        confidence,
        asking,
        ..
    }) = status
    else {
        return None;
    };
    Some(json!({
        "kind": "text",
        "ref": reference,
        "control": control,
        "confidence": confidence,
        "why": match asking {
            Asking::FirstTime =>
                "the run chose to type here, and this model does not generate text",
            Asking::FieldChanged =>
                "you answered this already, but the field changed before the text could be \
                 typed, so that answer was discarded",
        },
    }))
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
        Status::NeedsValue(_) => "needs_value",
        Status::OutOfBudget => "out_of_budget",
        Status::Stopped => "stopped",
        Status::Failed(_) => "failed",
    }
}

#[cfg(test)]
mod tests {
    use super::{covered_or_gone, opened_page, render};
    use browseros_core::{CoreError, Ref};
    use browseros_decide::pursue::{Outcome, Status};
    use browseros_decide::run::{Asking, Pending};

    fn outcome(status: Status) -> Outcome {
        Outcome {
            status,
            offered: 4,
            omitted: 0,
            offered_controls: vec!["textbox \"Search\"".to_string()],
            terminal_confidence: None,
            decisions: 1,
            trail: Vec::new(),
            url_before: "https://example.com/".to_string(),
            url_after: "https://example.com/".to_string(),
            title: "Example".to_string(),
            input_tokens: 2012,
            unlabelled_images: 0,
            opened: Vec::new(),
            resume: None,
        }
    }

    fn asking_for_text(asking: Asking) -> Status {
        Status::NeedsValue(Pending::Text {
            reference: "e18".to_string(),
            control: "combobox \"Where from?\"".to_string(),
            confidence: 0.97,
            guard: Some(7),
            asking,
        })
    }

    /// The four things this line has to do, learned from the hand-backs this
    /// loop has already got wrong: name the field, show the confidence so a bad
    /// choice is visible, print the literal next call, and say what to do
    /// instead if the field is wrong.
    #[test]
    fn an_ask_for_text_prints_the_next_call() {
        let (text, structured) = render(
            "set the origin to Bengaluru",
            3,
            &outcome(asking_for_text(Asking::FirstTime)),
            &[],
            Some("ctn-01J"),
        );

        assert!(text.contains("combobox \"Where from?\""), "{text}");
        assert!(text.contains("0.97"), "{text}");
        assert!(text.contains("continue: \"ctn-01J\""), "{text}");
        assert!(text.contains("provide: { \"e18\": "), "{text}");
        assert!(text.contains("start a fresh run"), "{text}");
        // The true reason answering beats typing it yourself, and the only
        // claim here that the code actually backs: the run re-reads the page
        // and compares the control's guard before it types.
        assert!(text.contains("has not changed before typing"), "{text}");
        assert!(
            text.contains("pass them as `values`"),
            "a caller told once never has to be asked again: {text}"
        );

        assert_eq!(structured["status"], "needs_value");
        assert_eq!(structured["continueWith"], "ctn-01J");
        assert_eq!(structured["needsInput"]["ref"], "e18");
        assert_eq!(structured["needsInput"]["kind"], "text");
        assert_eq!(structured["needsInput"]["confidence"], 0.97);
    }

    /// A second ask for the same field is not the same event as the first, and
    /// a caller told only "needs text" twice has no way to know its answer was
    /// received and then discarded.
    #[test]
    fn a_second_ask_says_the_answer_was_discarded() {
        let (text, structured) = render(
            "set the origin to Bengaluru",
            3,
            &outcome(asking_for_text(Asking::FieldChanged)),
            &[],
            Some("ctn-01J"),
        );

        assert!(text.contains("discarded"), "{text}");
        assert!(
            structured["needsInput"]["why"]
                .as_str()
                .is_some_and(|why| why.contains("discarded")),
            "{structured}"
        );
    }

    /// Without a handle there is nothing to print, and telling a caller to pass
    /// one it does not have is worse than telling it nothing.
    #[test]
    fn an_ask_without_a_handle_does_not_invent_one() {
        let (text, structured) = render(
            "set the origin to Bengaluru",
            3,
            &outcome(asking_for_text(Asking::FirstTime)),
            &[],
            None,
        );

        assert!(!text.contains("continue:"), "{text}");
        assert_eq!(structured["continueWith"], serde_json::Value::Null);
    }

    /// Two different endings wear the out-of-budget label and they need
    /// opposite advice. maxSteps counts the whole run, so a caller that resends
    /// the same limit gets a call that ends where the last one did.
    #[test]
    fn a_run_out_of_steps_says_to_raise_the_limit() {
        let mut ended = outcome(Status::OutOfBudget);
        ended.trail = vec![step(), step()];
        let (text, _) = render("sort by price", 3, &ended, &[], Some("ctn-01J"));

        assert!(text.contains("step limit"), "{text}");
        assert!(text.contains("maxSteps: 4"), "{text}");
        assert!(text.contains("continue: \"ctn-01J\""), "{text}");
    }

    #[test]
    fn a_run_out_of_time_does_not_invite_a_continuation() {
        let (text, structured) =
            render("sort by price", 3, &outcome(Status::OutOfBudget), &[], None);

        assert!(text.contains("time limit"), "{text}");
        assert!(text.contains("Start a fresh run"), "{text}");
        assert!(!text.contains("continue:"), "{text}");
        assert_eq!(structured["continueWith"], serde_json::Value::Null);
    }

    /// A new tab leaves the run where it was, so the caller has to be told what
    /// exists now rather than being moved onto it.
    #[test]
    fn a_page_the_run_opened_is_reported_as_the_callers_to_pursue() {
        let mut ended = outcome(Status::Satisfied);
        ended.opened = vec![browseros_decide::OpenedPage {
            page: 7,
            url: "https://example.com/deals".to_string(),
        }];
        let (text, structured) = render("open the deals page", 3, &ended, &[], None);

        assert!(text.contains("page 7 https://example.com/deals"), "{text}");
        assert!(text.contains("yours to pursue separately"), "{text}");
        assert_eq!(structured["openedPages"][0]["page"], 7);
    }

    /// The caller has eyes and already owns the screenshot tool for this page,
    /// so the run says what it could not read rather than spending the
    /// caller's context on an image it did not ask for.
    #[test]
    fn a_page_of_pictures_says_what_it_could_not_read() {
        let mut ended = outcome(Status::Stalled(browseros_decide::Stuck::NotConfident));
        ended.unlabelled_images = 14;
        let (text, structured) = render("find the best deal", 3, &ended, &[], None);

        assert!(text.contains("14 image(s)"), "{text}");
        assert!(text.contains("Take a screenshot of page 3"), "{text}");
        assert_eq!(structured["unlabelledImages"], 14);
    }

    /// On a satisfied run it is noise: whatever the pictures said, the goal was
    /// met without them.
    #[test]
    fn a_satisfied_run_does_not_mention_the_pictures() {
        let mut ended = outcome(Status::Satisfied);
        ended.unlabelled_images = 14;
        let (text, structured) = render("find the best deal", 3, &ended, &[], None);

        assert!(!text.contains("image(s)"), "{text}");
        assert_eq!(
            structured["unlabelledImages"], 14,
            "the count still travels, for a caller that wants to check"
        );
    }

    #[test]
    fn moves_are_counted_for_the_caller() {
        let mut ended = outcome(Status::Satisfied);
        ended.trail = vec![step(), move_step(), move_step()];
        let (_, structured) = render("show page 2", 3, &ended, &[], None);

        assert_eq!(structured["navigations"], 2);
        assert_eq!(structured["actions"], 3);
    }

    fn move_step() -> browseros_decide::Step {
        browseros_decide::Step {
            operation: browseros_decide::Operation::GoTo,
            target: Some("page 2 of the results".to_string()),
            value: None,
            confidence: 0.95,
            page_changed: true,
            url_after: "https://example.com/?page=2".to_string(),
            input_tokens: 1000,
        }
    }

    fn step() -> browseros_decide::Step {
        browseros_decide::Step {
            operation: browseros_decide::Operation::Click,
            target: Some("e1".to_string()),
            value: None,
            confidence: 0.95,
            page_changed: true,
            url_after: "https://example.com/".to_string(),
            input_tokens: 1000,
        }
    }

    /// The dispatch strips structured content from every tool without an
    /// output schema, and `tabs` has none, so the page id can only come from
    /// the text. That makes the wording load-bearing, and this pins it: see
    /// `tabs.rs`, which prints `opened page {id}`.
    #[test]
    fn the_page_a_new_tab_opened_is_read_from_what_tabs_prints() {
        assert_eq!(opened_page("opened page 7"), Some(7));
        assert_eq!(
            opened_page("some preamble\nopened page 12\nand a snapshot after it"),
            Some(12)
        );
        assert_eq!(opened_page("  opened page 3  "), Some(3));

        assert_eq!(opened_page("closed page 7"), None);
        assert_eq!(opened_page("opened page none"), None);
        assert_eq!(opened_page("opened page 0"), None, "page ids start at one");
        assert_eq!(opened_page(""), None);
    }

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
