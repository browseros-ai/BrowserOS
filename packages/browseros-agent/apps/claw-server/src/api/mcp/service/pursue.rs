//! `pursue` on the MCP surface: one errand on one page, decided step by step on the server by jev
//! through the decisions worker. Every move goes down the same dispatch path an agent's own call
//! takes, and the run is recorded as one local dispatch beside the actions it made.

use super::{ClawMcpService, StartedSession, finish_local_dispatch, hosted};
use crate::{
    api::mcp::{
        dispatch::{ToolIdentity, linked_cancel_token},
        observers::audit::{LocalToolDispatch, record_local_tool_dispatch},
        pursue::PageDriver,
    },
    ids::DispatchId,
    services::sessions::ClawTargetStore,
};
use browseros_core::target::BrowserView;
use browseros_decide::{
    offer::{Destination, NamedValue},
    pursue::{Status, advance_writing},
    report::{Told, tell},
    run::{Budget, Offered, Run},
};
use browseros_mcp::{TargetStore, ToolResult};
use rmcp::model::{CallToolResult, Tool, ToolAnnotations};
use serde_json::{Value, json};
use std::{sync::Arc, time::Instant as StdInstant};
use tokio_util::sync::CancellationToken;
use tracing::warn;

pub(in crate::api::mcp) const PURSUE_TOOL_NAME: &str = "pursue";

/// The plan feature that offers pursue.
const ULTRAFAST_FEATURE: &str = "claw.ultrafast";

const DEFAULT_MAX_STEPS: u64 = 12;
const MAX_STEPS: u64 = 40;
const MAX_SECONDS: u64 = 120;

const PURSUE_DESCRIPTION: &str = "\
Hand neo a whole errand on one page and let it do the steps itself, for multi-step work like \
filling a form, narrowing search results or adding something to a cart. State the outcome, not \
a step: \"add Sensodyne Fresh Mint 150g to the cart\", not \"click the search box\". Put text you \
already know in `values`, named by what it is; neo writes anything else it needs, but never \
passwords, card numbers, IDs or one-time codes. It will not invent a url: list the pages it may \
go to in `destinations`.

It returns `outcome`:
 - `completed`: the page should now hold what the goal asked for. Read the page to confirm.
 - `handed_back`: neo got as far as it could. Carry on with run from the page it names, without \
repeating the steps in `story`. A hand back is normal, not an error. After a hand back, use run \
for the rest of the task.";

pub(super) fn pursue_tool() -> Tool {
    let Value::Object(input_schema) = json!({
        "type": "object",
        "properties": {
            "page": {
                "type": "integer",
                "description": "The page to work on, from tabs or navigate."
            },
            "goal": {
                "type": "string",
                "description": "The whole outcome you want, not a single step."
            },
            "values": {
                "type": "object",
                "description": "Text you already know that may need typing, named by what it is, for instance {\"pincode\": \"600042\"}. Never a password, key, token or card number.",
                "additionalProperties": { "type": "string" }
            },
            "destinations": {
                "type": "array",
                "description": "Pages this errand may move to. neo picks one by label and never invents a url.",
                "items": {
                    "type": "object",
                    "properties": {
                        "label": { "type": "string" },
                        "url": { "type": "string" },
                        "newTab": { "type": "boolean" }
                    },
                    "required": ["label", "url"]
                }
            },
            "maxSteps": {
                "type": "integer",
                "description": "Stop after this many actions. Defaults to 12."
            }
        },
        "required": ["page", "goal"]
    }) else {
        unreachable!("the pursue input schema is an object")
    };
    let Value::Object(output_schema) = json!({
        "type": "object",
        "properties": {
            "outcome": { "type": "string", "enum": ["completed", "handed_back", "cancelled"] },
            "said": { "type": "string", "description": "One sentence, true of what happened." },
            "continueWith": { "type": "string", "description": "Present on a hand back: the tool to finish the task with." },
            "page": { "type": "integer" },
            "url": { "type": "string" },
            "story": { "type": "array", "items": { "type": "string" }, "description": "What neo did, in order." },
            "remaining": { "type": ["string", "null"], "description": "What is still to do, when neo can tell." },
            "status": { "type": "string", "description": "The detailed reason the run ended." },
            "actions": { "type": "integer" },
            "decisions": { "type": "integer" },
            "jevTokens": { "type": "integer" },
            "writerCalls": { "type": "integer" }
        },
        "required": ["outcome", "said", "page", "status"]
    }) else {
        unreachable!("the pursue output schema is an object")
    };
    Tool::new(PURSUE_TOOL_NAME, PURSUE_DESCRIPTION, Arc::new(input_schema))
        .with_raw_output_schema(Arc::new(output_schema))
        .with_annotations(
            ToolAnnotations::new()
                .read_only(false)
                .destructive(false)
                .idempotent(false),
        )
}

/// How a run ends, as the caller acts on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Ending {
    Completed,
    HandedBack,
    Cancelled,
}

impl Ending {
    pub(super) fn of(status: &Status) -> Self {
        match status {
            Status::Satisfied => Self::Completed,
            Status::Cancelled => Self::Cancelled,
            _ => Self::HandedBack,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::HandedBack => "handed_back",
            Self::Cancelled => "cancelled",
        }
    }
}

impl ClawMcpService {
    /// Whether pursue is listed: a desktop server whose plan has ultrafast and which has a
    /// decisions worker to reach. A call checks the plan again before anything is asked.
    pub(super) fn pursue_offered(&self) -> bool {
        !self.mode().is_hosted()
            && self.state.decisions.url().is_some()
            && self.state.billing.has_feature_now(ULTRAFAST_FEATURE)
    }

    pub(super) async fn call_pursue(
        &self,
        started: &StartedSession,
        raw_args: &Value,
        client_cancel: CancellationToken,
    ) -> CallToolResult {
        let dispatch_id = DispatchId::new();
        if !started
            .session
            .try_register_dispatch(dispatch_id.clone(), CancellationToken::new())
            .await
        {
            return CallToolResult::error(vec![rmcp::model::ContentBlock::text(
                "BrowserOS neo session is no longer live",
            )]);
        }
        let started_at = StdInstant::now();
        let result = self.pursue(started, raw_args, client_cancel).await;
        if let Err(error) = record_local_tool_dispatch(
            &self.state,
            LocalToolDispatch {
                session: &started.session,
                agent_label: &started.agent_label,
                tool_name: PURSUE_TOOL_NAME,
                raw_args,
                result: &result,
                duration_ms: i64::try_from(started_at.elapsed().as_millis()).unwrap_or(i64::MAX),
                dispatch_id: dispatch_id.clone(),
            },
        )
        .await
        {
            warn!(error = %error, "local tool audit submission failed");
        }
        finish_local_dispatch(started.session.as_ref(), &dispatch_id, result)
            .await
            .into_call_tool_result()
    }

    async fn pursue(
        &self,
        started: &StartedSession,
        raw_args: &Value,
        client_cancel: CancellationToken,
    ) -> ToolResult {
        let Some(page) = raw_args
            .get("page")
            .and_then(Value::as_u64)
            .and_then(|page| u32::try_from(page).ok())
        else {
            return ToolResult::error("pursue needs the page to work on");
        };
        if started.session.pursue_handed_back() {
            return already_handed_back(page);
        }
        let Some(goal) = raw_args
            .get("goal")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|goal| !goal.is_empty())
        else {
            return ToolResult::error("pursue needs a goal");
        };
        if self.mode().is_hosted() || !self.state.billing.has_feature(ULTRAFAST_FEATURE).await {
            return not_started(page, "fast browsing is not on this plan");
        }
        let Some(oracle) = self
            .state
            .decisions
            .oracle(self.state.billing.clone())
            .await
        else {
            return not_started(page, "fast browsing is not available right now");
        };
        let writer = self.state.decisions.writer(self.state.billing.clone());
        let Some(browser_session) = self.state.browser.session().await else {
            return not_started(page, "no browser is connected");
        };
        let binding = match self
            .state
            .targets
            .binding_for_execution(started.session.id())
            .await
        {
            Ok(binding) => binding,
            Err(error) => return not_started(page, &error.to_string()),
        };
        let view = hosted::with_viewports(
            BrowserView::capture(browser_session.clone(), binding, None).await,
            self.state.viewports.as_ref(),
        );
        let target_store: Arc<dyn TargetStore> = Arc::new(ClawTargetStore {
            bindings: self.state.targets.clone(),
            session: started.session.id().clone(),
        });
        let ownership_key = started.session.convo_id().clone();
        let ownership = self.state.sessions.ownership();
        let default_tab_group_id = match view.scope() {
            Ok(Some(scope)) => ownership
                .group_for_capture(
                    &ownership_key,
                    &scope.binding.intent.browser_profile_id,
                    scope.placement_window,
                    scope.binding.revision,
                )
                .await
                .and_then(|group| group.group_ref),
            Err(_) => None,
            Ok(None) => ownership.tab_group_ref(&ownership_key).await,
        };
        let cancel = linked_cancel_token(
            started.session.child_token(),
            client_cancel.child_token(),
            CancellationToken::new(),
        );
        let identity = ToolIdentity {
            session: started.session.clone(),
            agent: started.session.agent().clone(),
            ownership_key,
            agent_label: started.agent_label.clone(),
        };
        let Some(driver) = PageDriver::new(
            browser_session,
            page,
            self.catalog.clone(),
            started.session.id().clone(),
            identity,
            view,
            target_store,
            default_tab_group_id,
            self.state.clone(),
            self.output_files.clone(),
            cancel,
        ) else {
            return not_started(page, "this browser does not offer the act tool");
        };

        let offered = Offered {
            destinations: destinations_from(raw_args),
            values: values_from(raw_args),
            tabs: Vec::new(),
            // A move this browser cannot make is never offered as a choice.
            unavailable: driver.unavailable(),
        };
        let budget = Budget {
            max_steps: u32::try_from(
                raw_args
                    .get("maxSteps")
                    .and_then(Value::as_u64)
                    .unwrap_or(DEFAULT_MAX_STEPS)
                    .clamp(1, MAX_STEPS),
            )
            .unwrap_or(12),
            max_seconds: MAX_SECONDS,
        };
        let run = Run::new(goal, budget).offering(offered);
        let outcome = advance_writing(&oracle, &driver, writer.as_ref(), run).await;
        let told = tell(&outcome);
        let ending = Ending::of(&outcome.status);
        if uses_up_pursue(ending, outcome.decisions) {
            started.session.mark_pursue_handed_back();
        }
        finished(page, ending, &told, outcome.writes)
    }
}

/// A task may keep using pursue while its runs keep finding a next step. The first hand back after
/// something was decided uses it up; a run that ended before deciding anything does not.
pub(super) fn uses_up_pursue(ending: Ending, decisions: u32) -> bool {
    ending == Ending::HandedBack && decisions > 0
}

/// A call after this task's pursue handed back: run carries the task from here.
fn already_handed_back(page: u32) -> ToolResult {
    let said = format!(
        "neo already handed this task back, so it will not take it again. Carry on with run on page {page}."
    );
    ToolResult::text(
        said.clone(),
        Some(json!({
            "outcome": Ending::HandedBack.as_str(),
            "said": said,
            "continueWith": "run",
            "page": page,
            "story": [],
            "remaining": null,
            "status": "already_handed_back",
            "actions": 0,
            "decisions": 0,
            "jevTokens": 0,
            "writerCalls": 0,
        })),
    )
}

/// A run that never asked anything: a hand back with nothing done, which leaves pursue usable.
fn not_started(page: u32, why: &str) -> ToolResult {
    let said = format!(
        "neo's fast browsing did not start because {why}, so it is handing this task to you. Carry on with run on page {page}."
    );
    ToolResult::text(
        said.clone(),
        Some(json!({
            "outcome": Ending::HandedBack.as_str(),
            "said": said,
            "continueWith": "run",
            "page": page,
            "story": [],
            "remaining": null,
            "status": "not_started",
            "actions": 0,
            "decisions": 0,
            "jevTokens": 0,
            "writerCalls": 0,
        })),
    )
}

fn finished(page: u32, ending: Ending, told: &Told, writes: u32) -> ToolResult {
    let text = match ending {
        Ending::Completed => format!("{}\nRead page {page} to confirm.", told.said),
        Ending::Cancelled => told.said.clone(),
        Ending::HandedBack => {
            let mut text = format!(
                "neo got partway and is handing this back to you. {}",
                told.said
            );
            if !told.story.is_empty() {
                text.push_str(&format!(" Done so far: {}.", told.story.join("; ")));
            }
            if let Some(next) = &told.try_next {
                text.push_str(&format!(" Still to do: {next}"));
            }
            text.push_str(&format!(
                " Carry on with run on page {page}; there is no need to repeat what is done."
            ));
            text
        }
    };
    let mut body = json!({
        "outcome": ending.as_str(),
        "said": told.said,
        "page": page,
        "url": told.url,
        "story": told.story,
        "remaining": told.try_next,
        "status": told.status,
        "actions": told.actions,
        "decisions": told.decisions,
        "jevTokens": told.input_tokens,
        "writerCalls": writes,
    });
    if ending == Ending::HandedBack {
        body["continueWith"] = json!("run");
    }
    ToolResult::text(text, Some(body))
}

/// Destinations the caller authored, which is the only way this run may move.
fn destinations_from(raw_args: &Value) -> Vec<Destination> {
    raw_args
        .get("destinations")
        .and_then(Value::as_array)
        .map(|listed| {
            listed
                .iter()
                .filter_map(|entry| {
                    Some(Destination {
                        label: entry.get("label")?.as_str()?.to_string(),
                        url: entry.get("url")?.as_str()?.to_string(),
                        new_tab: entry
                            .get("newTab")
                            .and_then(Value::as_bool)
                            .unwrap_or(false),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Values the caller supplied, typed only into a field jev chose for them.
fn values_from(raw_args: &Value) -> Vec<NamedValue> {
    raw_args
        .get("values")
        .and_then(Value::as_object)
        .map(|given| {
            given
                .iter()
                .filter_map(|(name, text)| {
                    Some(NamedValue::new(name.clone(), text.as_str()?.to_string()))
                })
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_satisfied_run_is_completed() {
        assert_eq!(Ending::of(&Status::Satisfied), Ending::Completed);
        assert_eq!(Ending::of(&Status::Cancelled), Ending::Cancelled);
        assert_eq!(Ending::of(&Status::BudgetSpent), Ending::HandedBack);
        assert_eq!(
            Ending::of(&Status::Blocked("nothing".to_string())),
            Ending::HandedBack
        );
    }

    #[test]
    fn only_a_hand_back_after_a_decision_uses_pursue_up() {
        assert!(uses_up_pursue(Ending::HandedBack, 3));
        assert!(!uses_up_pursue(Ending::HandedBack, 0));
        assert!(!uses_up_pursue(Ending::Completed, 5));
        assert!(!uses_up_pursue(Ending::Cancelled, 2));
    }

    #[test]
    fn a_run_that_never_started_hands_back_without_an_error() {
        let result = not_started(3, "fast browsing is not available right now");
        assert!(!result.is_error);
        let body = result.structured_content.unwrap_or_default();
        assert_eq!(body["outcome"], "handed_back");
        assert_eq!(body["continueWith"], "run");
        assert_eq!(body["decisions"], 0);
    }

    #[test]
    fn values_and_destinations_come_from_the_arguments() {
        let args = json!({
            "values": { "pincode": "600042", "bad": 4 },
            "destinations": [{ "label": "Cart", "url": "https://shop.test/cart" }, { "label": "no url" }],
        });
        let values = values_from(&args);
        assert_eq!(values.len(), 1);
        assert_eq!(values[0].text, "600042");
        let destinations = destinations_from(&args);
        assert_eq!(destinations.len(), 1);
        assert!(!destinations[0].new_tab);
    }
}
