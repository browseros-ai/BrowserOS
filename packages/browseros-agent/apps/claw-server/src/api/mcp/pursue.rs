//! The browser, as the decision loop needs it.
//!
//! Every move goes down the same dispatch path a connected agent uses, so it
//! gets the same guards, effects and audit record as anything else. Nothing
//! here reaches the browser by a private route.
//!
//! The tool indexes are optional on purpose. A peer that does not ship `grep`
//! should get a working loop with `grep` left out of the vocabulary, not no loop
//! at all, and an earlier version that returned `None` from the constructor when
//! any tool was missing would have refused to start against a slightly older
//! extension.

use std::sync::Arc;

use browseros_core::{
    BrowserSession, BrowserView, PageId,
    snapshot::{SnapshotMode, SnapshotOptions},
};
use browseros_decide::drive::{ActError, Driver, Journey, Move, Performed};
use browseros_decide::view::PageView;
use browseros_decide::vocabulary::{How, Operation};
use browseros_mcp::{OutputFileAccess, TargetStore, ToolDef};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use crate::AppState;
use crate::api::mcp::dispatch::{ToolCall, ToolIdentity, dispatch_tool_call};
use crate::ids::SessionId;

/// Whether this server can offer the decision loop at all.
///
/// One type rather than four comparisons against `Option::None`. The catalogue,
/// the server instructions, `run`'s own description and the body of the managed
/// skill all have to answer this question the same way, and nothing enforces
/// that while each of them asks it separately. An agent told to prefer a tool
/// that is not listed, or offered a tool its instructions never mention, is
/// worse off than one told nothing.
///
/// Read fresh wherever it is needed rather than cached, which keeps the
/// catalogue and the instructions correct from the moment the environment has
/// the credential. The managed skill is the one exception and says so where it
/// is loaded: it is written to disk once per launch.
// Both live in config so the skill loader, which is a service, can ask the same
// question without reaching up into this layer.
pub use crate::config::PursueOffering;

/// How long to let a page keep changing before reading it anyway.
///
/// In code rather than by asking the model to wait: a wait decision costs a
/// whole request, and this costs milliseconds.
///
/// What this replaces was a flat 120ms sleep, which is shorter than one DOM
/// quiet interval, so a page that was still moving was read as though it had
/// stopped. The loop decides whether a move did anything by comparing
/// fingerprints, so a read taken too early reports that a working control did
/// nothing, and the control is retired.
const SETTLE_BUDGET: std::time::Duration = std::time::Duration::from_millis(1_500);

/// The same, for a wait the model chose and paid a whole request for.
///
/// Longer than the settle every observe already does, because otherwise WAIT
/// would be a request spent on nothing and the honest change would be to drop
/// it from the vocabulary.
const WAITED_FOR_BUDGET: std::time::Duration = std::time::Duration::from_millis(5_000);

/// Waits for the page to stop changing, for a bounded time.
///
/// `browseros_core` already has this: it reads the tab's loading flag, waits out
/// a navigation if it saw one, then watches a `MutationObserver` counter until
/// two reads agree. The shared default budget is 50ms, which cannot reach its
/// own 150ms navigation detect window or its own 100ms quiet interval, and its
/// comment says it is short on purpose so that an agent's actions stay
/// responsive, with the explicit `wait` tool for anything slower. That trade is
/// right for a model that can name what it is waiting for. jev cannot write a
/// selector or a substring, and a wait decision costs it a whole request, so it
/// passes its own budget instead.
///
/// The outcome is deliberately discarded. `BudgetExpired` means the page is
/// still moving and the ceiling was reached, which is the case the ceiling is
/// for; `Skipped` means the page could not be evaluated, and a snapshot is
/// still the right next move. There is nothing a caller could do differently
/// with either.
async fn settle(
    pages: &browseros_core::pages::PageManager,
    page: u32,
    budget: std::time::Duration,
) {
    let _ = browseros_core::settle::wait_for_action_settle(pages, PageId(page), budget).await;
}

/// Where each tool sits in the connected peer's catalogue.
///
/// Absent means the peer does not offer it, which is a vocabulary question
/// rather than an error.
#[derive(Debug, Clone, Copy, Default)]
struct Reach {
    act: Option<usize>,
    navigate: Option<usize>,
    tabs: Option<usize>,
}

impl Reach {
    fn of(catalog: &[ToolDef]) -> Self {
        let at = |name: &str| catalog.iter().position(|tool| tool.name == name);
        Self {
            act: at("act"),
            navigate: at("navigate"),
            tabs: at("tabs"),
        }
    }

    /// Moves this peer cannot make, so the loop never offers them.
    fn missing(self) -> Vec<Operation> {
        let mut missing = Vec::new();
        if self.navigate.is_none() {
            missing.push(Operation::Navigate);
            missing.push(Operation::Back);
            missing.push(Operation::Reload);
        }
        if self.tabs.is_none() {
            missing.push(Operation::Tabs);
        }
        missing
    }
}

pub struct PageDriver {
    session: Arc<BrowserSession>,
    page: u32,
    reach: Reach,
    catalog: Arc<Vec<ToolDef>>,
    session_id: SessionId,
    identity: ToolIdentity,
    /// Which window and tab group this session is bound to, captured once for
    /// the run.
    view: Option<BrowserView>,
    target_store: Arc<dyn TargetStore>,
    default_tab_group_id: Option<String>,
    state: AppState,
    output_files: OutputFileAccess,
    cancel: CancellationToken,
}

impl PageDriver {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        session: Arc<BrowserSession>,
        page: u32,
        catalog: Arc<Vec<ToolDef>>,
        session_id: SessionId,
        identity: ToolIdentity,
        view: Option<BrowserView>,
        target_store: Arc<dyn TargetStore>,
        default_tab_group_id: Option<String>,
        state: AppState,
        output_files: OutputFileAccess,
        cancel: CancellationToken,
    ) -> Option<Self> {
        let reach = Reach::of(&catalog);
        // Acting is the one tool without which there is no loop at all.
        reach.act?;
        Some(Self {
            session,
            page,
            reach,
            catalog,
            session_id,
            identity,
            view,
            target_store,
            default_tab_group_id,
            state,
            output_files,
            cancel,
        })
    }

    pub fn page(&self) -> u32 {
        self.page
    }

    /// Moves this browser cannot make, for the question builder to leave out.
    pub fn unavailable(&self) -> Vec<Operation> {
        self.reach.missing()
    }

    /// One tool call, with tokens whose lifetimes are its own.
    ///
    /// The three token slots are not interchangeable, and passing one token into
    /// all three is a run-ending bug rather than a tidy shortcut. `dispatch_tool_call`
    /// ends every call by cancelling both the dispatch token and the call token,
    /// so a loop that hands over its own long-lived token gets that token
    /// cancelled by its first action and reports itself finished before its
    /// second decision. Measured in the wild: every pursue run ended after
    /// exactly one decision.
    ///
    /// So each call gets a fresh dispatch token and a fresh child of the run's
    /// token. A child dies when its parent does, which is what cancellation
    /// needs, and cancelling a child leaves the parent alone, which is what the
    /// loop needs.
    fn call_to(&self, index: usize, args: Value) -> ToolCall {
        ToolCall::new(
            Arc::clone(&self.catalog),
            index,
            args,
            self.session_id.clone(),
            Some(self.identity.clone()),
            Some(Arc::clone(&self.session)),
            // Captured once for the run rather than per call. A view is a
            // snapshot of which window and tab group this session is bound to,
            // which does not change under a run working one page, and capturing
            // it per decision would put a round trip in front of every action.
            self.view.clone(),
            Arc::clone(&self.target_store),
            self.cancel.child_token(),
            self.cancel.clone(),
            CancellationToken::new(),
            self.default_tab_group_id.clone(),
            self.state.clone(),
            self.output_files.clone(),
        )
    }

    /// Runs one tool and returns its text, or why it refused.
    async fn run(
        &self,
        index: Option<usize>,
        what: Operation,
        args: Value,
    ) -> Result<String, ActError> {
        let Some(index) = index else {
            return Err(ActError::NotOffered(what));
        };
        let result = dispatch_tool_call(self.call_to(index, args))
            .await
            .map_err(|error| ActError::Unreachable(error.to_string()))?;
        let text = first_text(&result);
        if result.is_error.unwrap_or(false) {
            return Err(if covered_or_gone(&text) {
                ActError::GoneOrCovered(text)
            } else {
                ActError::Refused(text)
            });
        }
        Ok(text)
    }

    /// The act tool's arguments for one decision.
    fn args_for(&self, decision: &browseros_decide::gate::Decision) -> Option<Value> {
        let how = decision.how?;
        // The loop observes the page itself, so the act tool's readback is built
        // and then dropped. `resolve_diff_detail(None)` is `Full`, so leaving
        // this out renders a whole page diff into a result nothing reads.
        let mut args = json!({ "page": self.page, "kind": how.act_kind(), "diff": "none" });
        if let Some(reference) = &decision.target {
            args["ref"] = json!(reference);
        }
        match how {
            How::Fill | How::Select => {
                args["value"] = json!(decision.value.clone()?);
            }
            How::Press | How::Submit => {
                args["key"] = json!(decision.key.clone()?);
            }
            How::Scroll => {
                args["direction"] = json!(
                    decision
                        .direction
                        .map(|way| way.as_str().to_lowercase())
                        .unwrap_or_else(|| "down".to_string())
                );
            }
            How::Click | How::Check | How::Uncheck | How::Hover => {}
        }
        Some(args)
    }

    async fn act(&self, decision: &browseros_decide::gate::Decision) -> Result<(), ActError> {
        let Some(args) = self.args_for(decision) else {
            return Err(ActError::Refused(
                "the chosen way of acting had no argument to act with".to_string(),
            ));
        };
        // A key press carries no target: `ActKind::Press` reads `key` and
        // ignores `ref`, sending the key to whatever has focus. So the chosen
        // control is focused first, and a failure to focus stops the step rather
        // than pressing a key at something else. A submit is the same mechanism
        // with the field supplied by the run rather than by the model, and
        // focusing it is what makes Enter land in the box that was just filled
        // rather than wherever the page left the focus.
        // Focus the chosen control before acting on it.
        //
        // For a press or a submit this is what makes the key land in the box
        // that was just filled rather than wherever the page left the focus:
        // `ActKind::Press` reads `key` and ignores `ref`, and a failure to
        // focus stops the step rather than pressing a key at something else.
        //
        // For everything else it is a guard. Focusing a control scrolls it into
        // view, and a click dispatched at a point outside the viewport does
        // nothing while reporting success: measured on a live page, a button at
        // top -50 against a 941px viewport returned Done and the url never
        // changed. The loop reads that as the control being dead and strikes it
        // off, and on a results page the control it strikes off is an Add to
        // cart button.
        //
        // A control that cannot be focused is not treated as a failed step
        // here, because plenty act perfectly well without taking focus. Only a
        // press and a submit, which genuinely depend on it, still stop.
        if let Some(reference) = decision.target.as_deref() {
            let focusing = self
                .run(
                    self.reach.act,
                    Operation::Act,
                    json!({ "page": self.page, "kind": "focus", "ref": reference }),
                )
                .await;
            if matches!(decision.how, Some(How::Press | How::Submit)) {
                focusing?;
            }
        }
        self.run(self.reach.act, Operation::Act, args).await?;
        // No sleep here. The observe that follows waits for the page to stop
        // changing, and sleeping first only delays the start of that wait.
        Ok(())
    }

    async fn travel(&self, journey: &Journey) -> Result<(), ActError> {
        match journey {
            Journey::To(destination) if destination.new_tab => {
                self.run(
                    self.reach.tabs,
                    Operation::Tabs,
                    json!({ "action": "new", "url": destination.url }),
                )
                .await?;
            }
            Journey::To(destination) => {
                self.run(
                    self.reach.navigate,
                    Operation::Navigate,
                    json!({ "page": self.page, "url": destination.url }),
                )
                .await?;
            }
            Journey::Back => {
                self.run(
                    self.reach.navigate,
                    Operation::Back,
                    json!({ "page": self.page, "action": "back" }),
                )
                .await?;
            }
            Journey::Again => {
                self.run(
                    self.reach.navigate,
                    Operation::Reload,
                    json!({ "page": self.page, "action": "reload" }),
                )
                .await?;
            }
            Journey::Tab(page) => {
                self.run(
                    self.reach.tabs,
                    Operation::Tabs,
                    json!({ "action": "select", "page": page }),
                )
                .await?;
            }
        }
        // As in `act`: the observe that follows settles, so there is nothing
        // for a sleep to add.
        Ok(())
    }
}

impl Driver for PageDriver {
    fn stopped(&self) -> bool {
        self.cancel.is_cancelled()
    }

    async fn observe(&self) -> Result<PageView, String> {
        settle(self.session.pages.as_ref(), self.page, SETTLE_BUDGET).await;
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
        // The measured cost of this design assumes a budgeted state. Applying it
        // here rather than in each caller means a real run gets the same state a
        // measurement does.
        view.budget_text(PageView::TEXT_BUDGET);
        Ok(view)
    }

    async fn perform(&self, act: &Move) -> Result<Performed, ActError> {
        match act {
            Move::Act(decision) => {
                self.act(decision).await?;
                // Whether it moved is decided by the loop looking again, not by
                // what the tool said about itself.
                Ok(Performed::Moved)
            }
            Move::Travel(journey) => {
                self.travel(journey).await?;
                Ok(Performed::Moved)
            }
            Move::Settle => {
                settle(self.session.pages.as_ref(), self.page, WAITED_FOR_BUDGET).await;
                Ok(Performed::Unmoved)
            }
        }
    }
}

/// The text of a tool result.
///
/// Dispatch strips structured content from a tool with no output schema, so
/// text is what actually arrives and parsing it is not a shortcut.
fn first_text(result: &rmcp::model::CallToolResult) -> String {
    result
        .content
        .iter()
        .filter_map(|block| block.as_text().map(|text| text.text.clone()))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Whether a refusal means the control moved, vanished, or is behind something.
///
/// All one case on purpose. A retry clears none of them, and treating "is
/// covered by" as retryable is what let a run spend four attempts on one
/// control while an overlay sat on top of it.
fn covered_or_gone(reason: &str) -> bool {
    let reason = reason.to_lowercase();
    [
        "no longer",
        "not found",
        "stale",
        "is covered by",
        "detached",
    ]
    .iter()
    .any(|phrase| reason.contains(phrase))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reach as a peer that ships these tools and no others.
    fn reaching(act: bool, navigate: bool, tabs: bool) -> Reach {
        let at = |present: bool| if present { Some(0) } else { None };
        Reach {
            act: at(act),
            navigate: at(navigate),
            tabs: at(tabs),
        }
    }

    /// Navigating, going back and reloading all reach the browser through the
    /// same tool, so they are offered or withheld together.
    #[test]
    fn the_three_moves_that_share_a_tool_are_withheld_together() {
        let missing = reaching(true, false, true).missing();
        for operation in [Operation::Navigate, Operation::Back, Operation::Reload] {
            assert!(missing.contains(&operation), "{}", operation.as_str());
        }
    }

    /// A peer missing a tool gets a working loop with that move left out, not
    /// no loop at all. An earlier version refused to start at all.
    #[test]
    fn a_peer_without_tabs_still_drives() {
        let missing = reaching(true, true, false).missing();
        assert!(missing.contains(&Operation::Tabs));
        assert!(
            !missing.contains(&Operation::Navigate),
            "what it does have is still offered"
        );
        assert!(!missing.contains(&Operation::Act));
    }

    /// A peer that ships everything withholds nothing.
    #[test]
    fn a_complete_peer_gets_the_whole_vocabulary() {
        let missing = reaching(true, true, true).missing();
        assert!(missing.is_empty(), "{missing:?}");
    }

    /// Gone and covered are one case. A retry clears neither, and treating "is
    /// covered by" as retryable is what let a run spend four attempts on one
    /// control while an overlay sat on top of it.
    /// The bug that ended every run after one decision.
    ///
    /// Dispatch cancels the call token when a call finishes. If that token is
    /// the run's own, the run is over after its first action.
    #[test]
    fn a_finished_call_does_not_end_the_run_that_made_it() {
        let run_token = CancellationToken::new();

        // What dispatch does at the end of every call, to both tokens it owns.
        let per_call = run_token.child_token();
        per_call.cancel();
        CancellationToken::new().cancel();

        assert!(
            !run_token.is_cancelled(),
            "the loop must still be alive after one of its own actions"
        );
    }

    /// And cancelling the run still stops anything it is in the middle of.
    #[test]
    fn cancelling_the_run_cancels_a_call_in_flight() {
        let run_token = CancellationToken::new();
        let per_call = run_token.child_token();
        run_token.cancel();
        assert!(per_call.is_cancelled());
    }

    #[test]
    fn a_covered_control_reads_the_same_as_a_vanished_one() {
        assert!(covered_or_gone(
            "ref e12 is covered by <div class=\"overlay\">"
        ));
        assert!(covered_or_gone("ref e12 no longer exists"));
        assert!(covered_or_gone("the handle is stale"));
        assert!(
            !covered_or_gone("the page is still loading"),
            "that one is worth waiting for"
        );
    }
}
