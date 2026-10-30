//! Ultrafast skills on the MCP surface: `run_skill`, and the one internal `run` dispatch that
//! carries a flow. That dispatch goes down the same path an agent's own `run` takes, so it gets
//! the same guards, tab ownership and audit record; the tabs it opens belong to the caller.

use super::{ClawMcpService, StartedSession, finish_local_dispatch, hosted, session_handle};
use crate::{
    api::mcp::{
        dispatch::{ToolCall, ToolIdentity, dispatch_tool_call, linked_cancel_token},
        observers::audit::{LocalToolDispatch, record_local_tool_dispatch},
    },
    clock::now_epoch_ms,
    db::skills::RunMode,
    ids::DispatchId,
    services::{
        flow::{
            self, FlowExec, FlowStatus,
            brief::{self, CallTrace, RunTrace},
            compose::{self, Mode, RUN_TIMEOUT_MS},
            journal as run_journal,
            lifecycle::{self, Cause, Event},
            outcome::{self, Outcome},
            prose,
            store::{self, Effects, FlowDoc},
            triage, verify,
        },
        sessions::ClawTargetStore,
        skills::{RunItFast, SkillMd, neo_prefixed},
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
        crate::services::flow::ULTRAFAST_ENABLED && !self.mode().is_hosted()
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
        if doc.meta.effects == Effects::Write && !row.as_ref().is_some_and(flow::writes_allowed) {
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
                self.apply_event(&name, Event::RunDone, Some(&doc)).await;
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
                let cause = triage::cause(&exec, &doc, &handover).await;
                let step = handover.step.unwrap_or(1);
                self.record_mark(
                    &session_id,
                    &name,
                    &RunMode::handed_back(i64::from(step), cause.as_str()),
                )
                .await;
                self.remember_input(&name, &doc, &inputs, cause);
                self.apply_event(&name, Event::Handover(cause), Some(&doc))
                    .await;
                let source = handover
                    .step
                    .and_then(|n| doc.step(n))
                    .map(|step| step.src.as_str());
                let repair = (cause == Cause::SiteChanged).then(|| {
                    let after_write = doc.commit_step().is_some_and(|commit| step > commit);
                    let mut note = format!(
                        "The site changed at step {step}. Tell the user the result first, then call save_skill with name {name} and patch [{{ step: {step}, fn }}] holding a fixed version of that step. laterSources has the current source of this step and every later one; if the change reaches them too, put each fixed step in the same patch list."
                    );
                    if after_write {
                        note.push_str(" If the page simply had not updated yet, skip the patch.");
                    }
                    note
                });
                let mut packet = outcome::packet(
                    &handover,
                    &labels,
                    doc.commit_step(),
                    cause.as_str(),
                    source,
                    repair,
                );
                if cause == Cause::SiteChanged {
                    let later: Map<String, Value> = doc
                        .steps
                        .iter()
                        .filter(|later| later.n >= step)
                        .map(|later| (later.n.to_string(), json!(later.src)))
                        .collect();
                    packet["laterSources"] = Value::Object(later);
                }
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

/// Reads a patch: one `{ step, fn }` or a list of them, applied together.
fn parse_patches(patch: &Value) -> Result<Vec<(u32, String)>, String> {
    let entries = match patch {
        Value::Array(entries) => entries.clone(),
        Value::Object(_) => vec![patch.clone()],
        _ => return Err("patch must be [{ step, fn }, ...]".to_string()),
    };
    if entries.is_empty() {
        return Err("patch must list at least one { step, fn }".to_string());
    }
    entries
        .iter()
        .map(|entry| {
            let step = entry
                .get("step")
                .and_then(Value::as_u64)
                .and_then(|step| u32::try_from(step).ok())
                .ok_or("every patch entry needs a step number")?;
            let src = entry
                .get("fn")
                .and_then(Value::as_str)
                .ok_or_else(|| format!("patch for step {step} needs fn"))?;
            Ok((step, src.to_string()))
        })
        .collect()
}

/// Reads `save_skill`'s second-call arguments into a candidate flow.
fn candidate_flow(raw_args: &Value, name: &str, session: &str) -> Result<FlowDoc, String> {
    let steps = raw_args
        .get("flow")
        .and_then(Value::as_array)
        .ok_or("flow must be an array of { step, fn, commit? }")?
        .iter()
        .map(|entry| {
            let n = entry
                .get("step")
                .and_then(Value::as_u64)
                .and_then(|n| u32::try_from(n).ok())
                .filter(|n| *n > 0)
                .ok_or("every flow entry needs a step number from 1")?;
            let src = entry
                .get("fn")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|src| !src.is_empty())
                .ok_or_else(|| format!("flow step {n} needs fn, the step's function source"))?;
            Ok(store::FlowStep {
                n,
                commit: entry
                    .get("commit")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                src: src.to_string(),
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let effects = match raw_args.get("effects").and_then(Value::as_str) {
        Some("write") => Effects::Write,
        Some("read") => Effects::Read,
        _ => return Err("effects must be \"read\" or \"write\"".to_string()),
    };
    let types = |field: &str| -> Result<std::collections::BTreeMap<String, String>, String> {
        match raw_args.get(field) {
            None | Some(Value::Null) => Ok(Default::default()),
            Some(Value::Object(map)) => map
                .iter()
                .map(|(key, kind)| {
                    kind.as_str()
                        .map(|kind| (key.clone(), kind.to_string()))
                        .ok_or_else(|| {
                            format!("{field}.{key} must be a type name such as \"string\"")
                        })
                })
                .collect(),
            Some(_) => Err(format!("{field} must map names to type names")),
        }
    };
    let examples = raw_args
        .get("examples")
        .and_then(Value::as_array)
        .ok_or("examples must be an array of input objects")?
        .iter()
        .enumerate()
        .map(|(index, example)| {
            let (input, expect) = match example.get("input") {
                Some(Value::Object(input)) => (
                    input.clone(),
                    example
                        .get("expect")
                        .and_then(Value::as_str)
                        .unwrap_or("done")
                        .to_string(),
                ),
                _ => (
                    example
                        .as_object()
                        .cloned()
                        .ok_or("each example must be an object of inputs")?,
                    "done".to_string(),
                ),
            };
            Ok(store::Example {
                input,
                source: if index == 0 { "original" } else { "agent" }.to_string(),
                expect,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let mut steps = steps;
    steps.sort_by_key(|step| step.n);
    let hosts = flow_hosts(&steps);
    Ok(FlowDoc {
        meta: store::FlowMeta {
            name: name.to_string(),
            flow_version: 0,
            effects,
            hosts,
            inputs: types("inputs")?,
            output: types("output")?,
            examples,
            source_session: Some(session.to_string()),
        },
        steps,
    })
}

fn flow_hosts(steps: &[store::FlowStep]) -> Vec<String> {
    let mut hosts: Vec<String> = steps
        .iter()
        .flat_map(|step| step.src.split(['\'', '"', '`']))
        .filter_map(|piece| {
            let rest = piece
                .strip_prefix("https://")
                .or_else(|| piece.strip_prefix("http://"))?;
            let host = rest.split(['/', '?', '#']).next()?;
            (!host.is_empty()).then(|| host.to_string())
        })
        .collect();
    hosts.sort();
    hosts.dedup();
    hosts
}

fn run_it_fast(doc: &FlowDoc, status: FlowStatus) -> RunItFast {
    RunItFast {
        status: status.as_str(),
        inputs: doc
            .meta
            .inputs
            .iter()
            .map(|(name, kind)| (name.clone(), kind.clone()))
            .collect(),
    }
}

impl ClawMcpService {
    /// `save_skill` while ultrafast skills are on. The first call saves the prose exactly as
    /// before and returns a trace brief; a second call with `flow` compiles and verifies it.
    pub(super) async fn save_skill_ultrafast(
        &self,
        started: &StartedSession,
        raw_args: &Value,
        client_cancel: CancellationToken,
    ) -> ToolResult {
        if raw_args.get("ultrafast").and_then(Value::as_str) == Some("skip") {
            return self.skip_ultrafast(raw_args).await;
        }
        if raw_args.get("flow").is_some() || raw_args.get("patch").is_some() {
            return self.compile_flow(started, raw_args, client_cancel).await;
        }
        self.save_prose_and_brief(started, raw_args).await
    }

    async fn save_prose_and_brief(&self, started: &StartedSession, raw_args: &Value) -> ToolResult {
        let session_id = started.session.id().as_str().to_string();
        let input = match super::parse_save_skill(raw_args, session_id.clone()) {
            Ok(input) => input,
            Err(message) => return ToolResult::error(message),
        };
        let steps = input.steps.clone();
        let view = match self.state.skills.upsert(input).await {
            Ok(view) => view,
            Err(error) => return ToolResult::error(error.to_string()),
        };
        let name = view.model.name.clone();
        let mut row = match self.state.skills.flow_row(&name).await {
            Ok(Some(row)) => row,
            Ok(None) => flow::new_row(&name, FlowStatus::Pending),
            Err(error) => return ToolResult::error(error.to_string()),
        };
        row.status = FlowStatus::Pending.as_str().to_string();
        row.status_reason = None;
        row.attempts = 0;
        row.source_session_id = Some(session_id.clone());
        row.updated_at = now_epoch_ms();
        if let Err(error) = self.state.skills.put_flow_row(row).await {
            return ToolResult::error(error.to_string());
        }
        let brief = self.trace_brief(&session_id, &steps).await;
        let text = format!(
            "saved skill /{name}\nultrafast: pending. To make it ultrafast, call save_skill again with name {name} and a flow, following the brief below. If this task needs judgement on every run, call save_skill with name {name}, ultrafast: \"skip\" and a reason instead.\n{}",
            serde_json::to_string_pretty(&brief).unwrap_or_default()
        );
        ToolResult::text(
            text,
            Some(json!({ "saved": format!("/{name}"), "ultrafast": "pending", "brief": brief })),
        )
    }

    async fn trace_brief(&self, session_id: &str, steps: &[String]) -> Value {
        let dir = self.state.config.browserclaw_dir.clone();
        let entries = run_journal::read(&dir, session_id);
        let mut runs = Vec::with_capacity(entries.len());
        for entry in entries {
            let children = self
                .state
                .audit_log
                .dispatches_with_parent(&entry.dispatch_id)
                .await
                .unwrap_or_default();
            let calls = children
                .into_iter()
                .map(|row| {
                    let error = row
                        .result_meta
                        .as_deref()
                        .and_then(|meta| serde_json::from_str::<Value>(meta).ok())
                        .and_then(|meta| meta.get("isError").and_then(Value::as_bool))
                        .unwrap_or(false);
                    CallTrace::from_row(
                        row.tool_name,
                        row.duration_ms.unwrap_or(0),
                        error,
                        row.args_json.as_deref(),
                    )
                })
                .collect();
            runs.push(RunTrace { entry, calls });
        }
        let goal = self
            .state
            .audit_log
            .get_task_summary(session_id)
            .await
            .ok()
            .flatten()
            .map(|task| task.title);
        brief::build(goal.as_deref(), steps, &runs)
    }

    async fn skip_ultrafast(&self, raw_args: &Value) -> ToolResult {
        let Some(name) = raw_args
            .get("name")
            .and_then(Value::as_str)
            .map(neo_prefixed)
        else {
            return ToolResult::error("name must be a skill name");
        };
        if self.state.skills.get(&name).await.is_err() {
            return ToolResult::error(
                "save the skill first: call save_skill with name, description and steps",
            );
        }
        let reason = raw_args
            .get("reason")
            .and_then(Value::as_str)
            .unwrap_or("needs judgement on every run");
        let mut row = match self.state.skills.flow_row(&name).await {
            Ok(row) => row.unwrap_or_else(|| flow::new_row(&name, FlowStatus::Off)),
            Err(error) => return ToolResult::error(error.to_string()),
        };
        row.status = FlowStatus::Off.as_str().to_string();
        row.status_reason = Some(format!("skipped: {reason}"));
        row.updated_at = now_epoch_ms();
        if let Err(error) = self.state.skills.put_flow_row(row).await {
            return ToolResult::error(error.to_string());
        }
        if let Err(error) = self.state.skills.rerender(&name, SkillMd::Plain).await {
            warn!(error = %error, "re-rendering SKILL.md failed");
        }
        ToolResult::text(
            format!("ultrafast: off for /{name} ({reason}). It runs the normal way."),
            Some(json!({ "ultrafast": "off", "reason": reason })),
        )
    }

    async fn compile_flow(
        &self,
        started: &StartedSession,
        raw_args: &Value,
        client_cancel: CancellationToken,
    ) -> ToolResult {
        let Some(name) = raw_args
            .get("name")
            .and_then(Value::as_str)
            .map(neo_prefixed)
        else {
            return ToolResult::error("name must be a skill name");
        };
        let Ok(detail) = self.state.skills.get(&name).await else {
            return ToolResult::error(
                "save the skill first: call save_skill with name, description and steps, then send the flow",
            );
        };
        let session_id = started.session.id().as_str().to_string();
        let is_patch = raw_args.get("patch").is_some();
        let doc = if let Some(patch) = raw_args.get("patch") {
            let patches = match parse_patches(patch) {
                Ok(patches) => patches,
                Err(message) => return ToolResult::error(message),
            };
            let mut doc = match store::read(&self.state.skills.skill_dir(&name)) {
                Ok(Some(doc)) => doc,
                Ok(None) => {
                    return ToolResult::error(
                        "this skill has no flow to patch yet; send the whole flow instead",
                    );
                }
                Err(error) => {
                    return ToolResult::error(format!("the flow file is unreadable: {error}"));
                }
            };
            for (step, src) in &patches {
                if let Err(message) = doc.patch_step(*step, src) {
                    return ToolResult::error(message);
                }
            }
            doc
        } else {
            match candidate_flow(raw_args, &name, &session_id) {
                Ok(doc) => doc,
                Err(message) => return ToolResult::error(message),
            }
        };
        let labels = prose::prose_steps(&detail.body);
        let exec = SessionFlowExec {
            service: self,
            started,
            cancel: client_cancel,
        };
        let verdict = verify::verify(&exec, &doc, &labels).await;
        self.settle(&name, doc, verdict, &session_id, is_patch)
            .await
    }

    /// Records a verification result and answers the agent.
    async fn settle(
        &self,
        name: &str,
        mut doc: FlowDoc,
        verdict: Result<verify::Pass, verify::Failure>,
        session_id: &str,
        is_patch: bool,
    ) -> ToolResult {
        let mut row = match self.state.skills.flow_row(name).await {
            Ok(row) => row.unwrap_or_else(|| flow::new_row(name, FlowStatus::Pending)),
            Err(error) => return ToolResult::error(error.to_string()),
        };
        let before = FlowStatus::parse(&row.status);
        let state = lifecycle::State {
            status: before,
            attempts: row.attempts,
            site_failures: row.site_failures,
        };
        let now = now_epoch_ms();
        match verdict {
            Ok(pass) => {
                let next = lifecycle::next(state, Event::FlowPassed);
                doc.meta.flow_version = row.flow_version + 1;
                if let Err(error) = store::write(&self.state.skills.skill_dir(name), &doc) {
                    return ToolResult::error(format!("saving the flow failed: {error}"));
                }
                let same_effects = row.effects.as_deref() == Some(doc.meta.effects.as_str());
                row.write_approved = row.write_approved && same_effects;
                row.status = next.status.as_str().to_string();
                row.status_reason = None;
                row.flow_version = doc.meta.flow_version;
                row.attempts = next.attempts;
                row.site_failures = next.site_failures;
                row.effects = Some(doc.meta.effects.as_str().to_string());
                row.p50_ms = Some(pass.p50_ms);
                if matches!(before, FlowStatus::Degraded | FlowStatus::Broken) {
                    row.repaired_at = Some(now);
                }
                row.verified_at = Some(now);
                row.source_session_id = Some(session_id.to_string());
                row.updated_at = now;
                if let Err(error) = self.state.skills.put_flow_row(row.clone()).await {
                    return ToolResult::error(error.to_string());
                }
                if let Err(error) = self
                    .state
                    .skills
                    .rerender(
                        name,
                        SkillMd::Fast(&run_it_fast(&doc, FlowStatus::Verified)),
                    )
                    .await
                {
                    warn!(error = %error, "re-rendering SKILL.md failed");
                }
                run_journal::remove(&self.state.config.browserclaw_dir, session_id);
                let tried: Vec<String> = doc
                    .meta
                    .examples
                    .iter()
                    .map(|example| Value::Object(example.input.clone()).to_string())
                    .collect();
                let repaired = matches!(before, FlowStatus::Degraded | FlowStatus::Broken);
                let mut text = format!(
                    "ultrafast: {} on {} inputs ({}), p50 {} ms. /{name} runs through run_skill. Writes were not performed while verifying.",
                    if repaired {
                        "repaired and verified"
                    } else {
                        "verified"
                    },
                    pass.runs,
                    tried.join(", "),
                    pass.p50_ms
                );
                if doc.meta.effects == Effects::Write && !flow::writes_allowed(&row) {
                    text.push_str(&format!(
                        " Its first live run needs a one-time approval: BrowserOS, Skills, {name}, Allow writes."
                    ));
                }
                ToolResult::text(
                    text,
                    Some(json!({
                        "ultrafast": "verified",
                        "flowVersion": doc.meta.flow_version,
                        "p50Ms": pass.p50_ms,
                        "examples": doc.meta.examples.len(),
                        "repaired": repaired,
                    })),
                )
            }
            Err(failure) => {
                let next = lifecycle::next(state, Event::FlowFailed);
                let left = (lifecycle::BREAK_AFTER - next.attempts).max(0);
                row.status = next.status.as_str().to_string();
                row.attempts = next.attempts;
                if next.status != before {
                    row.status_reason = Some(format!("{} failed", failure.gate.label()));
                }
                row.updated_at = now;
                if let Err(error) = self.state.skills.put_flow_row(row).await {
                    return ToolResult::error(error.to_string());
                }
                if next.status == FlowStatus::Broken
                    && before != FlowStatus::Broken
                    && let Err(error) = self.state.skills.rerender(name, SkillMd::Repair).await
                {
                    warn!(error = %error, "re-rendering SKILL.md failed");
                }
                let tail = match next.status {
                    FlowStatus::Off => format!(
                        "ultrafast: off after {} tries. /{name} still runs the normal way.",
                        lifecycle::BREAK_AFTER
                    ),
                    FlowStatus::Broken => format!(
                        "ultrafast: broken after {} failed repairs. /{name} runs the normal way until a flow passes.",
                        lifecycle::BREAK_AFTER
                    ),
                    _ if is_patch => format!(
                        "A failed patch is not saved, so send every step you changed together in your next call: patch [{{ step, fn }}, ...]. {left} attempts left."
                    ),
                    _ => format!("Fix the flow and call save_skill again; {left} attempts left."),
                };
                let failing_source = failure
                    .step
                    .and_then(|n| doc.step(n))
                    .map(|step| step.src.clone());
                let source_note = failing_source
                    .as_deref()
                    .map(|src| {
                        format!("\nThe source of that step, to fix in your next call:\n{src}")
                    })
                    .unwrap_or_default();
                ToolResult::text(
                    format!("ultrafast: {}{source_note}\n{tail}", failure.describe()),
                    Some(json!({
                        "stepSource": failing_source,
                        "ultrafast": match next.status {
                            FlowStatus::Off => "off",
                            FlowStatus::Broken => "broken",
                            _ => "failing",
                        },
                        "gate": failure.gate.label(),
                        "step": failure.step,
                        "example": failure.example.map(Value::Object),
                        "message": failure.message,
                        "attemptsLeft": left,
                    })),
                )
            }
        }
    }

    /// Moves the flow's status for a run outcome, and keeps SKILL.md in step with it.
    async fn apply_event(&self, name: &str, event: Event, doc: Option<&FlowDoc>) {
        let Ok(Some(mut row)) = self.state.skills.flow_row(name).await else {
            return;
        };
        let before = lifecycle::State {
            status: FlowStatus::parse(&row.status),
            attempts: row.attempts,
            site_failures: row.site_failures,
        };
        let next = lifecycle::next(before, event);
        if next == before {
            return;
        }
        row.status = next.status.as_str().to_string();
        row.attempts = next.attempts;
        row.site_failures = next.site_failures;
        if next.status == FlowStatus::Broken && before.status != FlowStatus::Broken {
            row.status_reason = Some(format!(
                "the site changed {} times without a passing repair",
                lifecycle::BREAK_AFTER
            ));
        }
        row.updated_at = now_epoch_ms();
        if let Err(error) = self.state.skills.put_flow_row(row).await {
            warn!(error = %error, "recording the flow status failed");
            return;
        }
        if next.status != before.status {
            let fast = doc
                .filter(|_| next.status.runs())
                .map(|doc| run_it_fast(doc, next.status));
            let mode = match (&fast, next.status) {
                (Some(fast), _) => SkillMd::Fast(fast),
                (None, FlowStatus::Broken) => SkillMd::Repair,
                (None, _) => SkillMd::Plain,
            };
            if let Err(error) = self.state.skills.rerender(name, mode).await {
                warn!(error = %error, "re-rendering SKILL.md failed");
            }
        }
    }

    /// Keeps an input that made the flow hand back, so every later repair must handle it.
    fn remember_input(&self, name: &str, doc: &FlowDoc, inputs: &Map<String, Value>, cause: Cause) {
        let expect = match cause {
            Cause::SiteChanged => "done",
            Cause::InputOnly => "either",
            Cause::Transient => return,
        };
        let mut updated = doc.clone();
        updated.push_example(store::Example {
            input: inputs.clone(),
            source: "handover".to_string(),
            expect: expect.to_string(),
        });
        if updated.meta.examples != doc.meta.examples
            && let Err(error) = store::write(&self.state.skills.skill_dir(name), &updated)
        {
            warn!(error = %error, "saving the handover example failed");
        }
    }

    /// What `mark_skill_run` adds for a skill that has no working flow: repairing it is part of
    /// running it, so the user's saved skill gets fast again without being asked twice.
    pub(super) async fn upgrade_nudge(&self, name: &str) -> Option<String> {
        if !self.ultrafast() {
            return None;
        }
        let row = self.state.skills.flow_row(name).await.ok().flatten();
        let status = row
            .as_ref()
            .map_or(FlowStatus::None, |row| FlowStatus::parse(&row.status));
        let skipped = row
            .as_ref()
            .and_then(|row| row.status_reason.as_deref())
            .is_some_and(|reason| reason.starts_with("skipped"));
        if skipped
            || !matches!(
                status,
                FlowStatus::None | FlowStatus::Off | FlowStatus::Broken
            )
        {
            return None;
        }
        let existing = store::read(&self.state.skills.skill_dir(name))
            .ok()
            .flatten();
        Some(match (status, existing) {
            (FlowStatus::Broken, Some(doc)) => {
                let sources: Map<String, Value> = doc
                    .steps
                    .iter()
                    .map(|step| (step.n.to_string(), json!(step.src)))
                    .collect();
                format!(
                    "Its ultrafast flow is paused because the site changed. Repairing it is part of running this skill: do the task by hand first, tell the user the result, then call save_skill with name {name} and patch [{{ step, fn }}, ...] holding every step that needs the change. The current step sources: {}",
                    Value::Object(sources)
                )
            }
            _ => format!(
                "It has no ultrafast flow yet. Making it fast is part of running this skill: do the task by hand first, tell the user the result, then call save_skill with name {name} and a flow (one step function per numbered step, inputs, output, effects and at least 3 example inputs that each change every input)."
            ),
        })
    }
}

const SAVE_SKILL_ULTRAFAST_NOTE: &str = " With ultrafast skills on, saving takes two calls. The first (name, description, steps) saves the skill exactly as before and returns a trace brief of this session's run scripts. Answer it with a second call carrying the same name and a flow: one step function per numbered step, inputs, output, effects, and at least 3 example inputs, the first being this run's. The server verifies the flow on every example without performing writes, and the skill then runs in about a second through run_skill. Send the same call again to repair a flow. Send ultrafast: \"skip\" with a reason when the task needs judgement on every run.";

/// `save_skill` as listed while ultrafast skills are on: the same tool, with the second call's
/// fields, and `description` required only by the first call (checked by the server).
pub(super) fn ultrafast_save_skill_tool(base: &Tool) -> Tool {
    let mut tool = base.clone();
    let mut schema = tool.input_schema.as_ref().clone();
    if let Some(Value::Object(properties)) = schema.get_mut("properties") {
        properties.insert(
            "flow".to_string(),
            json!({
                "type": "array",
                "description": "Second call: one entry per numbered step, { step, fn, commit? }. fn is the step's async function source; commit: true marks the single irreversible step of a write flow.",
                "items": {
                    "type": "object",
                    "properties": {
                        "step": { "type": "integer", "minimum": 1 },
                        "fn": { "type": "string" },
                        "commit": { "type": "boolean" }
                    },
                    "required": ["step", "fn"]
                }
            }),
        );
        properties.insert(
            "inputs".to_string(),
            json!({ "type": "object", "description": "Second call: input name to type (string, number or boolean)." }),
        );
        properties.insert(
            "output".to_string(),
            json!({ "type": "object", "description": "Second call: output field to type." }),
        );
        properties.insert(
            "effects".to_string(),
            json!({ "type": "string", "enum": ["read", "write"] }),
        );
        properties.insert(
            "examples".to_string(),
            json!({ "type": "array", "items": { "type": "object" }, "description": "Second call: at least 3 input objects; the first is this session's input." }),
        );
        properties.insert(
            "ultrafast".to_string(),
            json!({ "type": "string", "enum": ["skip"], "description": "Skip the flow for a task that needs judgement on every run." }),
        );
        properties.insert("reason".to_string(), json!({ "type": "string" }));
        properties.insert(
            "patch".to_string(),
            json!({
                "type": "array",
                "description": "Repair a saved flow after a handover asked for it: the fixed steps, [{ step, fn }, ...], applied together. The whole flow is verified again; a failed patch is not saved.",
                "items": {
                    "type": "object",
                    "properties": { "step": { "type": "integer", "minimum": 1 }, "fn": { "type": "string" } },
                    "required": ["step", "fn"]
                }
            }),
        );
    }
    schema.insert("required".to_string(), json!(["name"]));
    tool.input_schema = Arc::new(schema);
    if let Some(description) = tool.description.as_deref() {
        tool.description = Some(format!("{description}{SAVE_SKILL_ULTRAFAST_NOTE}").into());
    }
    tool
}

impl ClawMcpService {
    pub(super) fn save_skill_offer(&self) -> Tool {
        if self.ultrafast() {
            ultrafast_save_skill_tool(&self.save_skill_tool)
        } else {
            self.save_skill_tool.clone()
        }
    }
}
