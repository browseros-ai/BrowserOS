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
    BrowserSession, PageId,
    snapshot::{SnapshotMode, SnapshotOptions},
};
use browseros_decide::drive::{ActError, Answer, Ask, Driver, Journey, Move, Performed};
use browseros_decide::view::PageView;
use browseros_decide::vocabulary::{How, Operation};
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

/// Where each tool sits in the connected peer's catalogue.
///
/// Absent means the peer does not offer it, which is a vocabulary question
/// rather than an error.
#[derive(Debug, Clone, Copy, Default)]
struct Reach {
    act: Option<usize>,
    navigate: Option<usize>,
    tabs: Option<usize>,
    screenshot: Option<usize>,
    read: Option<usize>,
    grep: Option<usize>,
    evaluate: Option<usize>,
}

impl Reach {
    fn of(catalog: &[ToolDef]) -> Self {
        let at = |name: &str| catalog.iter().position(|tool| tool.name == name);
        Self {
            act: at("act"),
            navigate: at("navigate"),
            tabs: at("tabs"),
            screenshot: at("screenshot"),
            read: at("read"),
            grep: at("grep"),
            evaluate: at("evaluate"),
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
        if self.screenshot.is_none() {
            missing.push(Operation::Screenshot);
        }
        if self.read.is_none() {
            missing.push(Operation::Read);
            // Counting is done over the page's text, so it needs the same tool.
            missing.push(Operation::Count);
        }
        if self.grep.is_none() {
            missing.push(Operation::Grep);
        }
        if self.evaluate.is_none() {
            missing.push(Operation::Evaluate);
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

    fn call_to(&self, index: usize, args: Value) -> ToolCall {
        ToolCall::new(
            Arc::clone(&self.catalog),
            index,
            args,
            self.session_id.clone(),
            Some(self.identity.clone()),
            Some(Arc::clone(&self.session)),
            self.cancel.clone(),
            self.cancel.clone(),
            self.cancel.clone(),
            self.default_tab_group_id.clone(),
            self.state.clone(),
            self.output_files.clone(),
        )
    }

    /// Runs one tool and returns its text, or why it refused.
    async fn run(&self, index: Option<usize>, what: Operation, args: Value) -> Result<String, ActError> {
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
        let mut args = json!({ "page": self.page, "kind": how.act_kind() });
        if let Some(reference) = &decision.target {
            args["ref"] = json!(reference);
        }
        match how {
            How::Fill | How::Select => {
                args["value"] = json!(decision.value.clone()?);
            }
            How::Press => {
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
        // A key press carries no target: act sends the key to whatever has
        // focus. So the chosen control is focused first, and a failure to focus
        // stops the step rather than pressing a key at something else.
        if decision.how == Some(How::Press)
            && let Some(reference) = decision.target.as_deref()
        {
            self.run(
                self.reach.act,
                Operation::Act,
                json!({ "page": self.page, "kind": "focus", "ref": reference }),
            )
            .await?;
        }
        self.run(self.reach.act, Operation::Act, args).await?;
        tokio::time::sleep(SETTLE).await;
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
        tokio::time::sleep(SETTLE).await;
        Ok(())
    }

    /// Answers a question about the page, which ends the run.
    async fn report(&self, ask: &Ask) -> Result<Answer, ActError> {
        let answer = |text: Option<String>, pictured: bool| Answer {
            asked: ask.as_str().to_string(),
            text,
            pictured,
        };
        match ask {
            Ask::Read => {
                let text = self
                    .run(self.reach.read, Operation::Read, json!({ "page": self.page }))
                    .await?;
                Ok(answer(Some(text), false))
            }
            Ask::Grep(pattern) => {
                let text = self
                    .run(
                        self.reach.grep,
                        Operation::Grep,
                        json!({ "page": self.page, "pattern": pattern }),
                    )
                    .await?;
                Ok(answer(Some(text), false))
            }
            // Counted here rather than asked for. The model is documented as
            // counting unreliably, with the error growing with the size of the
            // thing counted, so the page is read and the matches are counted in
            // code.
            Ask::Count(pattern) => {
                let text = self
                    .run(self.reach.read, Operation::Count, json!({ "page": self.page }))
                    .await?;
                let needle = pattern.to_lowercase();
                let found = text
                    .lines()
                    .filter(|line| line.to_lowercase().contains(&needle))
                    .count();
                Ok(answer(
                    Some(format!("{found} lines mention {pattern:?}")),
                    false,
                ))
            }
            // No tool call at all: the loop already holds the page, so this is
            // answered from the snapshot it was decided against.
            Ask::State { reference, state } => {
                let view = self.observe().await.map_err(ActError::Unreachable)?;
                let said = view.control(reference).map_or_else(
                    || format!("{reference} is no longer on the page"),
                    |control| match state.as_str() {
                        "checked" => format!(
                            "{reference} is {}",
                            match control.state.checked {
                                Some(true) => "ticked",
                                Some(false) => "not ticked",
                                None => "not something that ticks",
                            }
                        ),
                        "enabled" => format!(
                            "{reference} is {}",
                            if control.state.disabled {
                                "disabled"
                            } else {
                                "enabled"
                            }
                        ),
                        _ => format!("{reference} is on the page"),
                    },
                );
                Ok(answer(Some(said), false))
            }
            Ask::Picture => {
                self.run(
                    self.reach.screenshot,
                    Operation::Screenshot,
                    json!({ "page": self.page }),
                )
                .await?;
                Ok(answer(None, true))
            }
            Ask::Evaluate(what) => {
                let text = self
                    .run(
                        self.reach.evaluate,
                        Operation::Evaluate,
                        json!({
                            "page": self.page,
                            // The script is written here, not by the model, which
                            // cannot write one.
                            "script": format!(
                                "document.body.innerText.split('\\n').filter(l => l.toLowerCase().includes({what:?}.toLowerCase())).length"
                            ),
                        }),
                    )
                    .await?;
                Ok(answer(Some(text), false))
            }
        }
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
            Move::Resnapshot => Ok(Performed::Unmoved),
            Move::Settle => {
                tokio::time::sleep(SETTLE).await;
                Ok(Performed::Unmoved)
            }
            Move::Report(ask) => Ok(Performed::Answer(self.report(ask).await?)),
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
    ["no longer", "not found", "stale", "is covered by", "detached"]
        .iter()
        .any(|phrase| reason.contains(phrase))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reach as a peer that ships these tools and no others.
    fn reaching(act: bool, navigate: bool, tabs: bool, screenshot: bool, read: bool, grep: bool, evaluate: bool) -> Reach {
        let at = |present: bool| if present { Some(0) } else { None };
        Reach {
            act: at(act),
            navigate: at(navigate),
            tabs: at(tabs),
            screenshot: at(screenshot),
            read: at(read),
            grep: at(grep),
            evaluate: at(evaluate),
        }
    }

    /// A peer missing a tool gets a working loop with that move left out, not
    /// no loop at all. An earlier version refused to start at all.
    #[test]
    fn a_peer_without_grep_still_drives() {
        let missing = reaching(true, true, true, true, true, false, false).missing();
        assert!(missing.contains(&Operation::Grep));
        assert!(missing.contains(&Operation::Evaluate));
        assert!(
            !missing.contains(&Operation::Read),
            "what it does have is still offered"
        );
        assert!(!missing.contains(&Operation::Act));
    }

    /// Counting reads the page, so losing read loses counting with it.
    #[test]
    fn losing_read_loses_counting_too() {
        let missing = reaching(true, false, false, false, false, false, false).missing();
        assert!(missing.contains(&Operation::Read));
        assert!(missing.contains(&Operation::Count));
    }

    /// Navigating, going back and reloading all reach the browser through the
    /// same tool, so they are offered or withheld together.
    #[test]
    fn the_three_moves_that_share_a_tool_are_withheld_together() {
        let missing = reaching(true, false, true, true, true, true, true).missing();
        for operation in [Operation::Navigate, Operation::Back, Operation::Reload] {
            assert!(missing.contains(&operation), "{}", operation.as_str());
        }
    }

    /// A peer that ships everything withholds nothing.
    #[test]
    fn a_complete_peer_gets_the_whole_vocabulary() {
        let missing = reaching(true, true, true, true, true, true, true).missing();
        assert!(missing.is_empty(), "{missing:?}");
    }

    /// Gone and covered are one case. A retry clears neither, and treating "is
    /// covered by" as retryable is what let a run spend four attempts on one
    /// control while an overlay sat on top of it.
    #[test]
    fn a_covered_control_reads_the_same_as_a_vanished_one() {
        assert!(covered_or_gone("ref e12 is covered by <div class=\"overlay\">"));
        assert!(covered_or_gone("ref e12 no longer exists"));
        assert!(covered_or_gone("the handle is stale"));
        assert!(
            !covered_or_gone("the page is still loading"),
            "that one is worth waiting for"
        );
    }
}
