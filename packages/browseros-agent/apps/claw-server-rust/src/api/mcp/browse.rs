//! Driving a goal on a real page, and the result the caller reads.
//!
//! The loop itself lives in the policy crate and knows nothing about a
//! browser. This is the adapter: it observes through the ordinary snapshot and
//! acts through the ordinary act tool, so a run uses exactly the same code
//! paths an agent would have used by hand.

use browseros_core::{
    BrowserSession, PageId,
    snapshot::{SnapshotMode, SnapshotOptions},
};
use browseros_mcp::{
    BrowserToolDefaults, BrowserToolOptions, OutputFileAccess, ToolCtx, ToolDef, execute_tool,
};
use browseros_policy::action::{ActionSpace, Element, annotate_from_tree};
use browseros_policy::drive::{Driver, Outcome, Status, TrailEntry};
use browseros_policy::{Observation, Operation};
use serde_json::{Value, json};
use std::sync::{Arc, LazyLock};
use tokio_util::sync::CancellationToken;

/// Observes and acts on one page for the duration of a run.
pub struct PageDriver {
    browser: Arc<BrowserSession>,
    page: u32,
    output_files: OutputFileAccess,
    cancel: CancellationToken,
}

impl PageDriver {
    #[must_use]
    pub fn new(
        browser: Arc<BrowserSession>,
        page: u32,
        output_files: OutputFileAccess,
        cancel: CancellationToken,
    ) -> Self {
        Self {
            browser,
            page,
            output_files,
            cancel,
        }
    }

    fn ctx(&self) -> ToolCtx {
        ToolCtx::new(BrowserToolOptions {
            session: self.browser.clone(),
            defaults: BrowserToolDefaults::default(),
            cancel: self.cancel.child_token(),
            output_files: self.output_files.clone(),
            inner_call_hook: None,
            preloaded_helpers: Vec::new(),
        })
    }
}

impl Driver for PageDriver {
    fn stopped(&self) -> bool {
        self.cancel.is_cancelled()
    }

    async fn observe(&self) -> Result<Observation, String> {
        let snapshot = self
            .browser
            .observe(PageId(self.page))
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
        Ok(Observation {
            url: snapshot.url.clone(),
            title: title_of(&snapshot.text),
            tree: snapshot.text,
            space: ActionSpace::new(elements, true, true),
        })
    }

    async fn act(&self, operation: Operation, target: Option<&str>) -> Result<bool, String> {
        let args = match operation {
            Operation::Click => json!({
                "page": self.page,
                "kind": "click",
                "ref": target.ok_or_else(|| "click needs a target".to_string())?,
                "diff": "summary",
            }),
            Operation::ScrollDown => json!({
                "page": self.page, "kind": "scroll", "direction": "down", "diff": "summary",
            }),
            Operation::ScrollUp => json!({
                "page": self.page, "kind": "scroll", "direction": "up", "diff": "summary",
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

        let ctx = self.ctx();
        let result = execute_tool(cached_act_tool(), args, &ctx)
            .await
            .map_err(|error| error.to_string())?;
        if result.is_error {
            return Err(first_text(&result));
        }
        Ok(changed_from(&result))
    }
}

/// How long a wait pauses for before the page is read again.
const WAIT_PAUSE: std::time::Duration = std::time::Duration::from_millis(300);

/// Whether the action changed the page, read from the diff's own flag.
///
/// The prose was matched before, against a phrase that does not exist: an
/// unchanged diff says "no change since last snapshot", so a test for "no
/// changes" never matched and every no-op reported a change. That went into
/// the trail and into the history later decisions are shown.
fn changed_from(result: &browseros_mcp::ToolResult) -> bool {
    result
        .structured_content
        .as_ref()
        .and_then(|value| value.get("changed"))
        .and_then(Value::as_bool)
        // No diff was asked for or none came back, so there is nothing to
        // claim. Reporting no change is the honest default.
        .unwrap_or(false)
}

fn cached_act_tool() -> &'static ToolDef {
    static ACT_TOOL: LazyLock<ToolDef> = LazyLock::new(|| {
        browseros_mcp::catalog()
            .into_iter()
            .find(|tool| tool.name == "act")
            .unwrap_or_else(|| panic!("act tool missing from catalog"))
    });
    &ACT_TOOL
}

fn first_text(result: &browseros_mcp::ToolResult) -> String {
    result
        .content
        .iter()
        .find_map(|block| match block {
            rmcp::model::ContentBlock::Text(text) => Some(text.text.clone()),
            _ => None,
        })
        .unwrap_or_default()
}

/// The page title, read off the rendered tree's root line when it has one.
fn title_of(tree: &str) -> String {
    tree.lines()
        .next()
        .map(|line| line.trim_start_matches(['-', ' ']).to_string())
        .unwrap_or_default()
}

/// What the caller is told, as text and as structured content.
///
/// The outcome carries evidence rather than a verdict. A done decision is not
/// proof, so the caller gets the final url, what ran, and the confidence behind
/// each step, and verifies in one turn of its own.
#[must_use]
pub fn render(goal: &str, outcome: &Outcome) -> (String, Value) {
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
    lines.push(format!(
        "goal: {goal}\nat: {} ({})\ndecisions: {}, {}ms, {} input and {} output tokens",
        outcome.url,
        outcome.title,
        outcome.decisions(),
        outcome.elapsed_ms,
        outcome.input_tokens,
        outcome.output_tokens,
    ));
    if !outcome.trail.is_empty() {
        lines.push("what ran:".to_string());
        for (index, entry) in outcome.trail.iter().enumerate() {
            lines.push(format!("  {}. {}", index + 1, describe(entry)));
        }
    }

    let structured = json!({
        "status": status_name(&outcome.status),
        "url": outcome.url,
        "title": outcome.title,
        "decisions": outcome.decisions(),
        "elapsedMs": outcome.elapsed_ms,
        "inputTokens": outcome.input_tokens,
        "outputTokens": outcome.output_tokens,
        "needsText": match &outcome.status {
            Status::NeedsText { reference, field } => json!({ "ref": reference, "field": field }),
            _ => Value::Null,
        },
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
    format!(
        "{}{target} (confidence {:.2}, {changed}, {}ms)",
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
        let (text, structured) =
            render("Find a flight.", &outcome(Status::Satisfied, vec![entry()]));
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
            let (text, _) = render("Find a flight.", &outcome(status.clone(), Vec::new()));
            assert!(
                text.contains("page tools") || text.contains("browse again"),
                "{status:?} did not name a way forward: {text}"
            );
        }
    }

    /// An operator Stop is the one ending that must NOT invite the caller to
    /// carry on, because the session it would carry on in has been cancelled.
    #[test]
    fn an_operator_stop_does_not_invite_the_caller_to_continue() {
        let (text, structured) = render("Find a flight.", &outcome(Status::Stopped, vec![entry()]));
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
            &outcome(
                Status::NeedsText {
                    reference: "e4".to_string(),
                    field: "[e4] textbox Email".to_string(),
                },
                Vec::new(),
            ),
        );
        assert!(text.contains("[e4] textbox Email"), "{text}");
        assert!(text.contains("kind=\"fill\""), "{text}");
        assert_eq!(structured["needsText"]["ref"], json!("e4"));
    }

    #[test]
    fn the_trail_carries_the_cost_and_the_confidence() {
        let (text, structured) = render("Anything.", &outcome(Status::Stalled, vec![entry()]));
        assert!(text.contains("3200 input"), "{text}");
        assert!(text.contains("confidence 0.82"), "{text}");
        assert_eq!(structured["trail"][0]["targetConfidence"], json!(0.91));
        assert_eq!(structured["decisions"], json!(1));
    }

    /// The bug this closes: an unchanged diff says "no change since last
    /// snapshot", so matching on "no changes" never fired and every no-op was
    /// reported as a change.
    #[test]
    fn page_changed_comes_from_the_diff_flag_not_its_prose() {
        let unchanged = browseros_mcp::ToolResult::text(
            "no change since last snapshot",
            Some(json!({ "changed": false })),
        );
        assert!(!changed_from(&unchanged));

        let changed = browseros_mcp::ToolResult::text(
            "2 added, 1 removed",
            Some(json!({ "changed": true, "added": 2, "removed": 1 })),
        );
        assert!(changed_from(&changed));

        // Nothing to go on is reported as no change rather than guessed.
        let silent = browseros_mcp::ToolResult::text("done", None);
        assert!(!changed_from(&silent));
    }

    #[test]
    fn the_title_comes_off_the_tree_root() {
        assert_eq!(
            title_of("- Flight search\n  - button \"Go\""),
            "Flight search"
        );
        assert_eq!(title_of(""), "");
    }
}
