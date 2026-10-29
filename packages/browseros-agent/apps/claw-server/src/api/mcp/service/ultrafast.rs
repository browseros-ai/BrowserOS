//! Ultrafast skills on the MCP surface: `run_skill`, and the one internal `run` dispatch that
//! carries a flow. That dispatch goes down the same path an agent's own `run` takes, so it gets
//! the same guards, tab ownership and audit record; the tabs it opens belong to the caller.

use super::{ClawMcpService, StartedSession, finish_local_dispatch, hosted, session_handle};
use crate::{
    api::mcp::{
        dispatch::{ToolCall, ToolIdentity, dispatch_tool_call, linked_cancel_token},
        observers::audit::{LocalToolDispatch, record_local_tool_dispatch},
    },
    db::skills::RunMode,
    ids::DispatchId,
    services::{
        flow::{
            self, FlowExec, FlowStatus,
            compose::{self, Mode, RUN_TIMEOUT_MS},
            outcome::{self, Outcome},
            prose,
            store::{self, Effects, FlowDoc},
        },
        sessions::ClawTargetStore,
        skills::neo_prefixed,
    },
};
use browseros_core::target::BrowserView;
use browseros_mcp::{TargetStore, ToolResult};
use futures_util::future::BoxFuture;
use rmcp::model::{CallToolResult, Tool, ToolAnnotations};
use serde_json::{Map, Value, json};
use std::{sync::Arc, time::Instant as StdInstant};
use tokio_util::sync::CancellationToken;
use tracing::warn;

pub(in crate::api::mcp) const RUN_SKILL_TOOL_NAME: &str = "run_skill";
const RUN_SKILL_DESCRIPTION: &str = "Run a saved BrowserOS neo skill's verified ultrafast flow with new inputs. Use it first whenever a skill's SKILL.md says \"Run it fast\". Returns one of three statuses. done: the task is finished; report the output. needs_agent: the flow stopped at a step; you now own the page it left open, and the reply has a compact view of it, whether the write already happened (effects.committed: never redo a write that is true, check the page when it is \"unknown\"), and the steps still to do. Continue from there with your normal tools, and follow its repair note if it has one. refused: nothing ran; call mark_skill_run and follow the skill's steps.";

pub(super) fn run_skill_tool() -> Tool {
    let Value::Object(input_schema) = json!({
        "type": "object",
        "properties": {
            "name": { "type": "string", "pattern": "^[a-z0-9-]+$", "description": "The skill name, e.g. neo-shop-add-to-cart." },
            "inputs": { "type": "object", "description": "The flow's inputs, as listed in the skill's Run it fast section." }
        },
        "required": ["name"]
    }) else {
        unreachable!();
    };
    Tool::new(RUN_SKILL_TOOL_NAME, RUN_SKILL_DESCRIPTION, input_schema).with_annotations(
        ToolAnnotations::with_title("Run skill")
            .read_only(false)
            .destructive(false)
            .idempotent(false)
            .open_world(true),
    )
}

/// Why `run_skill` declined before running anything.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Refusal {
    NoFlow,
    FlowBroken,
    InputsInvalid,
    WriteNeedsConfirm,
}

impl Refusal {
    fn as_str(self) -> &'static str {
        match self {
            Self::NoFlow => "no_flow",
            Self::FlowBroken => "flow_broken",
            Self::InputsInvalid => "inputs_invalid",
            Self::WriteNeedsConfirm => "write_needs_confirm",
        }
    }
}

fn refused(reason: Refusal, name: &str, detail: &str) -> ToolResult {
    let next = match reason {
        Refusal::WriteNeedsConfirm => format!(
            "This skill performs a write, and its first ultrafast run needs a one-time approval from the user. Tell the user to open BrowserOS, Skills, {name}, and choose Allow writes. For now, call mark_skill_run with name {name} and follow the skill's steps."
        ),
        Refusal::InputsInvalid => format!(
            "Fix the inputs ({detail}) and call run_skill again, or call mark_skill_run with name {name} and follow the skill's steps."
        ),
        Refusal::NoFlow | Refusal::FlowBroken => {
            format!("Call mark_skill_run with name {name} and follow the skill's steps.")
        }
    };
    let structured = json!({
        "status": "refused",
        "reason": reason.as_str(),
        "detail": detail,
        "next": next,
    });
    ToolResult::text(
        format!("refused ({}): {detail}\n{next}", reason.as_str()),
        Some(structured),
    )
}

/// The browser half of a flow run, bound to the session that asked for it.
pub(super) struct SessionFlowExec<'s> {
    pub service: &'s ClawMcpService,
    pub started: &'s StartedSession,
    pub cancel: CancellationToken,
}

impl FlowExec for SessionFlowExec<'_> {
    fn exec<'a>(
        &'a self,
        doc: &'a FlowDoc,
        inputs: &'a Map<String, Value>,
        mode: Mode,
        stop_after: Option<u32>,
    ) -> BoxFuture<'a, Outcome> {
        Box::pin(async move {
            let code = compose::compose(doc, inputs, mode, stop_after);
            match self
                .service
                .internal_run(self.started, code, self.cancel.clone())
                .await
            {
                Ok(value) => outcome::parse(&value),
                Err(message) => Outcome::RunFailed(message),
            }
        })
    }
}

impl ClawMcpService {
    pub(super) fn ultrafast(&self) -> bool {
        self.state.agent_settings.ultrafast_enabled() && !self.mode().is_hosted()
    }

    pub(super) fn run_skill_offer(&self) -> Option<Tool> {
        self.ultrafast()
            .then(|| session_handle::offer(run_skill_tool()))
    }

    /// Dispatches one `run` call for the session, exactly as the agent's own call would go,
    /// and returns the value the script returned.
    pub(super) async fn internal_run(
        &self,
        started: &StartedSession,
        code: String,
        client_cancel: CancellationToken,
    ) -> Result<Value, String> {
        let Some(tool_index) = self.find_tool_index("run") else {
            return Err("this browser does not offer the run tool".to_string());
        };
        let browser_session = self.state.browser.session().await;
        let binding = self
            .state
            .targets
            .binding_for_execution(started.session.id())
            .await
            .map_err(|error| error.to_string())?;
        let view = match &browser_session {
            Some(browser) => Some(BrowserView::capture(browser.clone(), binding, None).await),
            None => None,
        }
        .map(|view| hosted::with_viewports(view, self.state.viewports.as_ref()));
        let target_store: Arc<dyn TargetStore> = Arc::new(ClawTargetStore {
            bindings: self.state.targets.clone(),
            session: started.session.id().clone(),
        });
        let ownership_key = started.session.convo_id().clone();
        let ownership = self.state.sessions.ownership();
        let default_tab_group_id = match view.as_ref().map(BrowserView::scope) {
            Some(Ok(Some(scope))) => ownership
                .group_for_capture(
                    &ownership_key,
                    &scope.binding.intent.browser_profile_id,
                    scope.placement_window,
                    scope.binding.revision,
                )
                .await
                .and_then(|group| group.group_ref),
            Some(Err(_)) => None,
            _ => ownership.tab_group_ref(&ownership_key).await,
        };
        // Fresh tokens per call: dispatch cancels its own tokens when the call ends, and the
        // caller's token must outlive this call (see PageDriver::call_to).
        let dispatch_cancel = CancellationToken::new();
        let cancel = linked_cancel_token(
            started.session.child_token(),
            client_cancel.child_token(),
            dispatch_cancel.clone(),
        );
        let identity = ToolIdentity {
            session: started.session.clone(),
            agent: started.session.agent().clone(),
            ownership_key,
            agent_label: started.agent_label.clone(),
        };
        let call = ToolCall::new(
            self.catalog.clone(),
            tool_index,
            json!({ "code": code, "timeout": RUN_TIMEOUT_MS }),
            started.session.id().clone(),
            Some(identity),
            browser_session,
            view,
            target_store,
            cancel,
            client_cancel.child_token(),
            dispatch_cancel,
            default_tab_group_id,
            self.state.clone(),
            self.output_files.clone(),
        );
        let result: CallToolResult = dispatch_tool_call(call)
            .await
            .map_err(|error| error.to_string())?;
        let structured = result.structured_content.clone().unwrap_or(Value::Null);
        if result.is_error.unwrap_or(false) {
            let message = structured
                .get("error")
                .and_then(Value::as_str)
                .map(str::to_string)
                .or_else(|| {
                    result
                        .content
                        .first()
                        .and_then(|block| block.as_text())
                        .map(|text| text.text.clone())
                })
                .unwrap_or_else(|| "the flow's run call failed".to_string());
            return Err(message);
        }
        Ok(structured.get("value").cloned().unwrap_or(Value::Null))
    }

    pub(super) async fn call_run_skill(
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
        let result = self.run_skill(started, raw_args, client_cancel).await;
        if let Err(error) = record_local_tool_dispatch(
            &self.state,
            LocalToolDispatch {
                session: &started.session,
                agent_label: &started.agent_label,
                tool_name: RUN_SKILL_TOOL_NAME,
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

    async fn run_skill(
        &self,
        started: &StartedSession,
        raw_args: &Value,
        client_cancel: CancellationToken,
    ) -> ToolResult {
        if !self.ultrafast() {
            return ToolResult::error("ultrafast skills are not enabled on this server");
        }
        let Some(raw_name) = raw_args.get("name").and_then(Value::as_str) else {
            return ToolResult::error("name must be a skill name");
        };
        let name = neo_prefixed(raw_name.trim());
        let Ok(detail) = self.state.skills.get(&name).await else {
            return refused(Refusal::NoFlow, &name, "no skill has that name");
        };
        let row = match self.state.skills.flow_row(&name).await {
            Ok(row) => row,
            Err(error) => return ToolResult::error(error.to_string()),
        };
        let status = row
            .as_ref()
            .map_or(FlowStatus::None, |row| FlowStatus::parse(&row.status));
        if status == FlowStatus::Broken {
            return refused(
                Refusal::FlowBroken,
                &name,
                "the flow kept failing and is switched off until it is repaired",
            );
        }
        if !status.runs() {
            return refused(Refusal::NoFlow, &name, "the skill has no verified flow");
        }
        let skill_dir = self.state.skills.skill_dir(&name);
        let doc = match store::read(&skill_dir) {
            Ok(Some(doc)) => doc,
            Ok(None) => return refused(Refusal::NoFlow, &name, "the skill has no flow file"),
            Err(error) => {
                return refused(
                    Refusal::NoFlow,
                    &name,
                    &format!("the flow file is unreadable: {error}"),
                );
            }
        };
        let inputs = match flow::check_inputs(&doc, raw_args.get("inputs").unwrap_or(&Value::Null))
        {
            Ok(inputs) => inputs,
            Err(message) => return refused(Refusal::InputsInvalid, &name, &message),
        };
        if doc.meta.effects == Effects::Write && !row.as_ref().is_some_and(|row| row.write_approved)
        {
            return refused(
                Refusal::WriteNeedsConfirm,
                &name,
                "write flows need a one-time approval",
            );
        }
        let labels = prose::prose_steps(&detail.body);
        let exec = SessionFlowExec {
            service: self,
            started,
            cancel: client_cancel,
        };
        let session_id = started.session.id().as_str().to_string();
        match exec.exec(&doc, &inputs, Mode::Live, None).await {
            Outcome::Done {
                output,
                done,
                duration_ms,
            } => {
                self.record_mark(&session_id, &name, &RunMode::ultrafast())
                    .await;
                let structured = json!({
                    "status": "done",
                    "output": output,
                    "steps": done,
                    "durationMs": duration_ms,
                });
                ToolResult::text(
                    format!(
                        "done in {} ms\n{}",
                        duration_ms.unwrap_or(0),
                        serde_json::to_string_pretty(&output).unwrap_or_default()
                    ),
                    Some(structured),
                )
            }
            Outcome::NeedsAgent(handover) => {
                let cause = "unknown";
                let step = handover.step.unwrap_or(1);
                self.record_mark(
                    &session_id,
                    &name,
                    &RunMode::handed_back(i64::from(step), cause),
                )
                .await;
                let source = handover
                    .step
                    .and_then(|n| doc.step(n))
                    .map(|step| step.src.as_str());
                let packet =
                    outcome::packet(&handover, &labels, doc.commit_step(), cause, source, None);
                handover_result(packet)
            }
            Outcome::RunFailed(message) => ToolResult::error(format!(
                "the flow could not run: {message}. Call mark_skill_run with name {name} and follow the skill's steps."
            )),
            other => ToolResult::error(format!("unexpected flow outcome in a live run: {other:?}")),
        }
    }

    async fn record_mark(&self, session_id: &str, name: &str, mode: &RunMode) {
        if let Err(error) = self.state.skill_runs.mark_as(session_id, name, mode).await {
            warn!(error = %error, "recording the ultrafast run failed");
        }
    }
}

pub(super) fn handover_result(packet: Value) -> ToolResult {
    let failed = &packet["failed"];
    let mut text = format!(
        "needs_agent: step {} ({}) stopped: {}\n{}",
        failed["step"],
        failed["label"].as_str().unwrap_or(""),
        failed["message"].as_str().unwrap_or(""),
        packet["next"].as_str().unwrap_or("")
    );
    if let Some(repair) = packet.get("repair").and_then(Value::as_str) {
        text.push('\n');
        text.push_str(repair);
    }
    ToolResult::text(text, Some(packet))
}
