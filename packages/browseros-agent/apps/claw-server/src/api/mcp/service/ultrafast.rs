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
            compose::{self, Mode},
            consent::{self, Decision},
            journal as run_journal,
            lifecycle::{self, Cause, Event},
            outcome::{self, Outcome},
            prose,
            store::{self, Effects, FlowDoc},
            triage, verify,
        },
        sessions::ClawTargetStore,
        skills::{Publish, RunItFast, SkillMd, neo_prefixed},
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
const RUN_SKILL_DESCRIPTION: &str = "Run a saved BrowserOS neo skill's verified ultrafast flow with new inputs. Use it first whenever a skill's SKILL.md says \"Run it fast\". Returns one of three statuses. done: the task is finished; report the output. needs_agent: the flow stopped at a step; you now own the page it left open, and the reply has a compact view of it, whether the write already happened (effects.committed: never redo a write that is true, check the page when it is \"unknown\"), and the steps still to do. Continue from there with your normal tools, and follow its repair note if it has one. refused: nothing ran; follow its next note. With reason needs_consent, ask the user the question it gives you before anything else.";

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
    /// The user chose to run this write skill with their agent.
    WithAgent,
    /// The click pays, orders, sends or deletes, so it never runs without the agent.
    AlwaysWithAgent,
}

impl Refusal {
    fn as_str(self) -> &'static str {
        match self {
            Self::NoFlow => "no_flow",
            Self::FlowBroken => "flow_broken",
            Self::InputsInvalid => "inputs_invalid",
            Self::WithAgent => "with_agent",
            Self::AlwaysWithAgent => "always_with_agent",
        }
    }
}

fn refused(reason: Refusal, name: &str, detail: &str) -> ToolResult {
    let next = match reason {
        Refusal::WithAgent | Refusal::AlwaysWithAgent => format!(
            "It runs with you. Call mark_skill_run with name {name} and follow the skill's steps, reading each page as you go."
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
    fn run_policy(&self) -> browseros_mcp::RunPolicy {
        self.service.state.config.behavior.run_policy
    }

    fn exec<'a>(
        &'a self,
        doc: &'a FlowDoc,
        inputs: &'a Map<String, Value>,
        mode: Mode,
        stop_after: Option<u32>,
    ) -> BoxFuture<'a, Outcome> {
        Box::pin(async move {
            let code = compose::compose(doc, inputs, mode, stop_after, self.run_policy());
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
        self.state.ultrafast_enabled()
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
            json!({ "code": code }),
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
        let mut structured = result.structured_content.clone().unwrap_or(Value::Null);
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
        if structured.get("writtenToFile").and_then(Value::as_bool) == Some(true) {
            // Internal flows consume the complete typed value, not the model's preview.
            // Recover it from the same saved call rather than rerunning browser effects
            // or silently turning an oversized successful return into null.
            let id = structured
                .pointer("/output/id")
                .and_then(Value::as_str)
                .ok_or_else(|| "the flow's complete output could not be published".to_string())?;
            let task = self
                .state
                .targets
                .canonical_task(started.session.id())
                .await;
            let store = self.output_files.for_session(task.as_str());
            let mut bytes = Vec::new();
            let mut offset = 0;
            loop {
                let chunk = store
                    .read_bytes(
                        id,
                        "result.json",
                        offset,
                        browseros_mcp::output_store::MAX_READ_BYTES,
                    )
                    .await
                    .map_err(|error| error.to_string())?;
                bytes.extend(chunk.bytes);
                match chunk.next_offset {
                    Some(next) => offset = next,
                    None => break,
                }
            }
            let complete: Value =
                serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
            structured = complete
                .get("structuredContent")
                .cloned()
                .ok_or_else(|| "the flow's saved output has no typed result".to_string())?;
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
        let labels = prose::prose_steps(&detail.body);
        match consent::decision(row.as_ref(), &doc, &labels) {
            Decision::WithoutAgent => {}
            Decision::WithAgent => {
                return refused(
                    Refusal::WithAgent,
                    &name,
                    "the user chose to run this skill with you",
                );
            }
            Decision::AlwaysWith(kind) => {
                return refused(Refusal::AlwaysWithAgent, &name, kind.reason());
            }
            Decision::Undecided => {
                let question = self.ask_consent(
                    started,
                    &name,
                    &doc,
                    &labels,
                    row.as_ref(),
                    detail.view.model.site.as_deref(),
                );
                let text = format!(
                    "refused (needs_consent): nothing ran. This skill changes something, and the user hasn't chosen yet whether it runs with you or without you.\n{}",
                    question.guidance
                );
                return ToolResult::text(
                    text,
                    Some(json!({
                        "status": "refused",
                        "reason": "needs_consent",
                        "detail": "the user hasn't chosen whether this skill runs with or without you",
                        "next": question.guidance,
                        "consent": question.structured,
                    })),
                );
            }
        }
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
                self.remember_input(&name, &inputs, cause).await;
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
            Outcome::RunFailed(message) => {
                ToolResult::error(run_failed_next(&doc, &name, &message))
            }
            other => ToolResult::error(format!("unexpected flow outcome in a live run: {other:?}")),
        }
    }

    async fn record_mark(&self, session_id: &str, name: &str, mode: &RunMode) {
        if let Err(error) = self.state.skill_runs.mark_as(session_id, name, mode).await {
            warn!(error = %error, "recording the ultrafast run failed");
        }
    }
}

/// What the agent does after a flow run failed without reporting how far it got. A write flow
/// may have finished its write before the failure, so redoing every step could repeat it.
fn run_failed_next(doc: &FlowDoc, name: &str, message: &str) -> String {
    let mut text = format!("the flow could not run: {message}.");
    if let Some(commit) = doc
        .commit_step()
        .filter(|_| doc.meta.effects == Effects::Write)
    {
        text.push_str(&format!(
            " It stopped without reporting how far it got, so its write (step {commit}) may already have happened. Check the page or the site's own record first, and never repeat that write if it took effect. Then call mark_skill_run with name {name} and finish whatever steps are still undone."
        ));
    } else {
        text.push_str(&format!(
            " Call mark_skill_run with name {name} and follow the skill's steps."
        ));
    }
    text
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
            prose_hash: None,
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

/// What publishing a flow needs to ask the user about running it without the agent.
struct Asking<'a> {
    started: &'a StartedSession,
    labels: &'a [String],
    site: Option<&'a str>,
}

/// The question for the agent to put to the user, as guidance text and as structured content.
struct ConsentQuestion {
    guidance: String,
    structured: Value,
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
    /// Mints a single-use token for this click and phrases the question around it.
    fn ask_consent(
        &self,
        started: &StartedSession,
        name: &str,
        doc: &FlowDoc,
        labels: &[String],
        row: Option<&crate::db::entities::skill_flows::Model>,
        skill_site: Option<&str>,
    ) -> ConsentQuestion {
        let now = now_epoch_ms();
        let click =
            consent::commit_label(doc, labels).unwrap_or_else(|| "its last step".to_string());
        let site = consent::site_label(doc, skill_site);
        let fast_seconds = consent::fast_seconds(row.and_then(|row| row.p50_ms));
        let token = self.state.consent_tokens.mint(
            consent::Pending {
                session: started.session.id().as_str().to_string(),
                skill: name.to_string(),
                flow_version: doc.meta.flow_version,
                commit: consent::commit_fingerprint(doc, labels).unwrap_or_default(),
                expires_at: now + consent::TOKEN_TTL_MS,
            },
            now,
        );
        let question = consent::Question {
            name,
            click: &click,
            site: &site,
            fast_seconds,
            token: &token,
            reask: consent::is_reask(row, doc, labels),
        };
        ConsentQuestion {
            guidance: question.guidance(),
            structured: json!({
                "token": token,
                "expiresInSec": consent::TOKEN_TTL_MS / 1000,
                "click": click,
                "site": site,
                "fastSeconds": fast_seconds,
                "reask": question.reask,
                "facts": question.facts(),
            }),
        }
    }

    /// The user's answer, relayed by the agent: run without the agent (`allow: true`), or with
    /// it. `{ allow: false }` with no token takes an earlier "without" back.
    async fn record_consent(&self, started: &StartedSession, raw_args: &Value) -> ToolResult {
        let Some(name) = raw_args
            .get("name")
            .and_then(Value::as_str)
            .map(neo_prefixed)
        else {
            return ToolResult::error("name must be a skill name");
        };
        let consent_args = &raw_args["consent"];
        let Some(allow) = consent_args.get("allow").and_then(Value::as_bool) else {
            return ToolResult::error(
                "consent.allow must be true (run without you) or false (run with you)",
            );
        };
        let token = consent_args.get("token").and_then(Value::as_str);
        let answer = consent_args
            .get("answer")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|answer| !answer.is_empty())
            .map(|answer| answer.chars().take(MAX_ANSWER_CHARS).collect::<String>());
        let Ok(detail) = self.state.skills.get(&name).await else {
            return ToolResult::error("no skill has that name");
        };
        let mut row = match self.state.skills.flow_row(&name).await {
            Ok(Some(row)) => row,
            Ok(None) => return ToolResult::error("this skill has no fast version to choose for"),
            Err(error) => return ToolResult::error(error.to_string()),
        };
        let doc = match store::read(&self.state.skills.skill_dir(&name)) {
            Ok(Some(doc)) => doc,
            Ok(None) => return ToolResult::error("this skill has no fast version to choose for"),
            Err(error) => {
                return ToolResult::error(format!("the flow file is unreadable: {error}"));
            }
        };
        let labels = prose::prose_steps(&detail.body);
        let Some(fingerprint) = consent::commit_fingerprint(&doc, &labels) else {
            return ToolResult::error(
                "this skill doesn't change anything, so there is nothing to choose",
            );
        };
        if let Some(kind) = consent::always_with_agent(&doc, &labels) {
            return ToolResult::text(
                format!(
                    "{} Skills like this always run with you, so there is nothing to choose.",
                    kind.reason()
                ),
                Some(json!({ "runs": "with_agent", "alwaysWithAgent": kind.as_str() })),
            );
        }
        let now = now_epoch_ms();
        match token {
            None if allow => {
                return ToolResult::error(format!(
                    "running without you needs the user's answer to the question: call run_skill with name {name} to get it, ask them, then send their answer with that token"
                ));
            }
            None => {}
            Some(token) => {
                let Some(pending) = self.state.consent_tokens.take(token, now) else {
                    return ToolResult::error(format!(
                        "that token is used up or has expired. Call run_skill with name {name} to get a new question, and ask the user again."
                    ));
                };
                let session_id = started.session.id().as_str();
                if pending.session != session_id
                    || pending.skill != name
                    || pending.flow_version != doc.meta.flow_version
                    || pending.commit != fingerprint
                {
                    return ToolResult::error(format!(
                        "that token was for a different question. Call run_skill with name {name} to get the current one, and ask the user again."
                    ));
                }
                if allow && answer.is_none() {
                    return ToolResult::error(
                        "send the user's exact words in consent.answer along with allow: true",
                    );
                }
            }
        }
        let click =
            consent::commit_label(&doc, &labels).unwrap_or_else(|| "its last step".to_string());
        let site = consent::site_label(&doc, detail.view.model.site.as_deref());
        row.write_approved = allow;
        row.approved_commit = allow.then(|| fingerprint.clone());
        row.with_agent_commit = (!allow).then(|| fingerprint.clone());
        row.approved_via = Some("chat".to_string());
        row.approved_at = Some(now);
        row.approved_answer = answer;
        row.approved_client = Some(started.agent_label.clone());
        row.updated_at = now;
        if let Err(error) = self.state.skills.put_flow_row(row).await {
            return ToolResult::error(error.to_string());
        }
        let fast = run_it_fast(&doc, FlowStatus::Verified);
        let mode = if allow {
            SkillMd::Fast(&fast)
        } else {
            SkillMd::Plain
        };
        if let Err(error) = self.state.skills.rerender(&name, mode).await {
            warn!(error = %error, "re-rendering SKILL.md failed");
        }
        if allow {
            ToolResult::text(
                format!(
                    "without_agent: neo will run /{name} by itself, including this click: {click} (on {site})."
                ),
                Some(json!({ "runs": "without_agent" })),
            )
        } else {
            ToolResult::text(
                format!("with_agent: /{name} runs with you."),
                Some(json!({ "runs": "with_agent" })),
            )
        }
    }

    /// `save_skill` while ultrafast skills are on. The first call saves the prose exactly as
    /// before and returns a trace brief; a second call with `flow` compiles and verifies it.
    pub(super) async fn save_skill_ultrafast(
        &self,
        started: &StartedSession,
        raw_args: &Value,
        client_cancel: CancellationToken,
    ) -> ToolResult {
        if raw_args.get("consent").is_some()
            && raw_args.get("flow").is_none()
            && raw_args.get("patch").is_none()
        {
            return self.record_consent(started, raw_args).await;
        }
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
        let mut doc = if let Some(patch) = raw_args.get("patch") {
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
        doc.meta.prose_hash = Some(prose::fingerprint(&labels));
        let exec = SessionFlowExec {
            service: self,
            started,
            cancel: client_cancel,
        };
        let verdict = verify::verify(&exec, &doc, &labels).await;
        let asked = Asking {
            started,
            labels: &labels,
            site: detail.view.model.site.as_deref(),
        };
        self.settle(&name, doc, verdict, &session_id, is_patch, &asked)
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
        asked: &Asking<'_>,
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
                match self
                    .state
                    .skills
                    .publish_flow(name, &mut doc, row.clone(), is_patch)
                    .await
                {
                    Ok(Publish::Saved) => {}
                    Ok(Publish::StepsChanged) => {
                        return ToolResult::error(
                            "ultrafast: the skill's steps were edited while this flow was being verified, so it was not saved. Read the skill again and call save_skill with a flow for its current steps.",
                        );
                    }
                    Err(error) => {
                        return ToolResult::error(format!("saving the flow failed: {error}"));
                    }
                }
                let decision = consent::decision(Some(&row), &doc, asked.labels);
                let fast = run_it_fast(&doc, FlowStatus::Verified);
                let mode = if consent::offers_run_skill(decision) {
                    SkillMd::Fast(&fast)
                } else {
                    SkillMd::Plain
                };
                if let Err(error) = self.state.skills.rerender(name, mode).await {
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
                    "ultrafast: {} on {} inputs ({}), p50 {} ms. Writes were not performed while verifying.",
                    if repaired {
                        "repaired and verified"
                    } else {
                        "verified"
                    },
                    pass.runs,
                    tried.join(", "),
                    pass.p50_ms
                );
                let mut structured = json!({
                    "ultrafast": "verified",
                    "flowVersion": doc.meta.flow_version,
                    "p50Ms": pass.p50_ms,
                    "examples": doc.meta.examples.len(),
                    "repaired": repaired,
                });
                match decision {
                    Decision::WithoutAgent => {
                        text.push_str(&format!(" /{name} runs through run_skill."));
                        if doc.meta.effects == Effects::Write {
                            structured["runs"] = json!("without_agent");
                        }
                    }
                    Decision::WithAgent => {
                        text.push_str(&format!(
                            " The user chose to run /{name} with you, so nothing to ask: it keeps running the regular way."
                        ));
                        structured["runs"] = json!("with_agent");
                    }
                    Decision::AlwaysWith(kind) => {
                        text.push_str(&format!(
                            " {} Skills like this always run with you, so there is nothing to ask.",
                            kind.reason()
                        ));
                        structured["runs"] = json!("with_agent");
                        structured["alwaysWithAgent"] = json!(kind.as_str());
                    }
                    Decision::Undecided => {
                        let question = self.ask_consent(
                            asked.started,
                            name,
                            &doc,
                            asked.labels,
                            Some(&row),
                            asked.site,
                        );
                        text.push('\n');
                        text.push_str(&question.guidance);
                        structured["ultrafast"] = json!("needs_consent");
                        structured["consent"] = question.structured;
                    }
                }
                ToolResult::text(text, Some(structured))
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
        if let Err(error) = self.state.skills.put_flow_row(row.clone()).await {
            warn!(error = %error, "recording the flow status failed");
            return;
        }
        if next.status != before.status {
            let labels = match self.state.skills.get(name).await {
                Ok(detail) => prose::prose_steps(&detail.body),
                Err(_) => Vec::new(),
            };
            let fast = doc
                .filter(|_| next.status.runs())
                .filter(|doc| {
                    consent::offers_run_skill(consent::decision(Some(&row), doc, &labels))
                })
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
    async fn remember_input(&self, name: &str, inputs: &Map<String, Value>, cause: Cause) {
        let expect = match cause {
            Cause::SiteChanged => "done",
            Cause::InputOnly => "either",
            Cause::Transient => return,
        };
        let example = store::Example {
            input: inputs.clone(),
            source: "handover".to_string(),
            expect: expect.to_string(),
        };
        if let Err(error) = self.state.skills.add_flow_example(name, example).await {
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

/// Keeps a pasted essay from filling the stored answer.
const MAX_ANSWER_CHARS: usize = 500;

const SAVE_SKILL_ULTRAFAST_NOTE: &str = " With ultrafast skills on, saving takes two calls. The first (name, description, steps) saves the skill exactly as before and returns a trace brief of this session's run scripts. Answer it with a second call carrying the same name and a flow: one step function per numbered step, inputs, output, effects, and at least 3 example inputs, the first being this run's. The server verifies the flow on every example without performing writes, and the skill then runs in about a second through run_skill. Send the same call again to repair a flow. Send ultrafast: \"skip\" with a reason when the task needs judgement on every run. When a skill changes something, a reply may ask you to put a question to the user; send their answer back with consent, exactly as that reply says.";

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
            "consent".to_string(),
            json!({
                "type": "object",
                "description": "Only after you asked the user the question a save_skill or run_skill reply gave you. token from that reply; allow: true means run without you, false means with you; answer is their exact words. { allow: false } alone switches an earlier choice back to running with you.",
                "properties": {
                    "token": { "type": "string" },
                    "allow": { "type": "boolean" },
                    "answer": { "type": "string" }
                },
                "required": ["allow"]
            }),
        );
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

#[cfg(test)]
mod consent_tests {
    use super::*;
    use crate::services::{
        flow::{new_row, store::FlowStep},
        skills::{CreateSkill, SkillOrigin},
    };

    const SKILL: &str = "neo-shop-add";

    struct Setup {
        service: ClawMcpService,
        started: StartedSession,
        state: crate::AppState,
    }

    async fn setup(last_step: &str, commit_src: &str) -> anyhow::Result<Setup> {
        let call = crate::api::mcp::test_support::tool_call("tabs", json!({})).await?;
        let session = call
            .identity
            .as_ref()
            .map(|identity| identity.session.clone())
            .ok_or_else(|| anyhow::anyhow!("session missing"))?;
        let state = call.state.clone();
        state.ultrafast.set(true);
        state
            .skills
            .create(CreateSkill {
                name: SKILL.to_string(),
                description: "Add a product to the Amazon cart".to_string(),
                site: Some("amazon.in".to_string()),
                steps: vec!["Open Amazon.".to_string(), last_step.to_string()],
                learned_notes: vec![],
                origin: SkillOrigin::Agent,
                source_session_id: None,
            })
            .await?;
        let doc = FlowDoc {
            meta: store::FlowMeta {
                name: SKILL.to_string(),
                flow_version: 1,
                effects: Effects::Write,
                hosts: vec!["www.amazon.in".to_string()],
                inputs: Default::default(),
                output: Default::default(),
                examples: vec![],
                source_session: None,
                prose_hash: None,
            },
            steps: vec![
                FlowStep {
                    n: 1,
                    commit: false,
                    src: "async (s) => { await s.page.goto('https://www.amazon.in') }".to_string(),
                },
                FlowStep {
                    n: 2,
                    commit: true,
                    src: commit_src.to_string(),
                },
            ],
        };
        store::write(&state.skills.skill_dir(SKILL), &doc)?;
        let mut row = new_row(SKILL, FlowStatus::Verified);
        row.effects = Some("write".to_string());
        row.flow_version = 1;
        row.p50_ms = Some(2_100);
        state.skills.put_flow_row(row).await?;
        Ok(Setup {
            service: ClawMcpService::new(state.clone()),
            started: StartedSession {
                session,
                agent_label: "Claude Code".to_string(),
            },
            state,
        })
    }

    const ADD_TO_CART: &str = "async (s) => { await s.page.click({ name: 'Add to cart' }) }";

    fn structured(result: &CallToolResult) -> Value {
        result.structured_content.clone().unwrap_or(Value::Null)
    }

    fn text(result: &CallToolResult) -> String {
        result
            .content
            .first()
            .and_then(|block| block.as_text())
            .map(|text| text.text.clone())
            .unwrap_or_default()
    }

    impl Setup {
        async fn run_skill(&self) -> CallToolResult {
            self.service
                .call_run_skill(
                    &self.started,
                    &json!({ "name": SKILL }),
                    CancellationToken::new(),
                )
                .await
        }

        async fn answer(&self, consent: Value) -> CallToolResult {
            self.service
                .call_save_skill(
                    &self.started,
                    &json!({ "name": SKILL, "consent": consent }),
                    CancellationToken::new(),
                )
                .await
        }

        async fn token(&self) -> String {
            let asked = self.run_skill().await;
            structured(&asked)["consent"]["token"]
                .as_str()
                .unwrap_or_default()
                .to_string()
        }
    }

    #[tokio::test]
    async fn an_undecided_write_skill_asks_before_anything_runs() -> anyhow::Result<()> {
        let setup = setup("Add it to the cart.", ADD_TO_CART).await?;
        let asked = setup.run_skill().await;
        let body = structured(&asked);
        assert_eq!(body["status"], "refused", "{body}");
        assert_eq!(body["reason"], "needs_consent");
        assert_eq!(body["consent"]["click"], "Add it to the cart.");
        assert_eq!(body["consent"]["site"], "amazon.in");
        assert_eq!(body["consent"]["fastSeconds"], 3);
        assert_eq!(body["consent"]["reask"], false);
        let guidance = text(&asked);
        assert!(guidance.contains("with me or without me?"), "{guidance}");
        assert!(guidance.contains("Never answer for the user"), "{guidance}");
        assert!(
            !guidance.contains("BrowserOS, Skills"),
            "no pointer to the app: {guidance}"
        );
        Ok(())
    }

    #[tokio::test]
    async fn without_the_agent_is_recorded_with_the_users_words_and_the_token_works_once()
    -> anyhow::Result<()> {
        let setup = setup("Add it to the cart.", ADD_TO_CART).await?;
        let token = setup.token().await;

        let missing_words = setup.answer(json!({ "token": token, "allow": true })).await;
        assert_eq!(missing_words.is_error, Some(true));
        let fresh = setup.token().await;
        let answered = setup
            .answer(json!({ "token": fresh, "allow": true, "answer": "without you is fine" }))
            .await;
        assert_ne!(answered.is_error, Some(true), "{answered:?}");
        assert_eq!(structured(&answered)["runs"], "without_agent");
        assert!(text(&answered).contains("including this click: Add it to the cart."));

        let row = setup
            .state
            .skills
            .flow_row(SKILL)
            .await?
            .ok_or_else(|| anyhow::anyhow!("row"))?;
        assert!(row.write_approved);
        assert_eq!(row.approved_via.as_deref(), Some("chat"));
        assert_eq!(row.approved_answer.as_deref(), Some("without you is fine"));
        assert_eq!(row.approved_client.as_deref(), Some("Claude Code"));
        let body = setup.state.skills.get(SKILL).await?.body;
        assert!(body.contains("## Run it fast"), "{body}");

        let replayed = setup
            .answer(json!({ "token": fresh, "allow": true, "answer": "yes" }))
            .await;
        assert_eq!(replayed.is_error, Some(true), "a token works once");
        assert!(text(&replayed).contains("used up or has expired"));
        Ok(())
    }

    #[tokio::test]
    async fn forged_and_borrowed_tokens_change_nothing() -> anyhow::Result<()> {
        let setup = setup("Add it to the cart.", ADD_TO_CART).await?;
        let forged = setup
            .answer(json!({ "token": "c_000000000000", "allow": true, "answer": "yes" }))
            .await;
        assert_eq!(forged.is_error, Some(true));

        let without_token = setup
            .answer(json!({ "allow": true, "answer": "yes" }))
            .await;
        assert_eq!(without_token.is_error, Some(true));
        assert!(text(&without_token).contains("needs the user's answer"));

        // A token minted for another session's question.
        let now = now_epoch_ms();
        let borrowed = setup.state.consent_tokens.mint(
            consent::Pending {
                session: "another-session".to_string(),
                skill: SKILL.to_string(),
                flow_version: 1,
                commit: "whatever".to_string(),
                expires_at: now + consent::TOKEN_TTL_MS,
            },
            now,
        );
        let refused = setup
            .answer(json!({ "token": borrowed, "allow": true, "answer": "yes" }))
            .await;
        assert_eq!(refused.is_error, Some(true));
        assert!(text(&refused).contains("different question"));

        let row = setup
            .state
            .skills
            .flow_row(SKILL)
            .await?
            .ok_or_else(|| anyhow::anyhow!("row"))?;
        assert!(!row.write_approved);
        assert_eq!(row.approved_via, None);
        Ok(())
    }

    #[tokio::test]
    async fn with_the_agent_is_kept_and_not_asked_again() -> anyhow::Result<()> {
        let setup = setup("Add it to the cart.", ADD_TO_CART).await?;
        let token = setup.token().await;
        let answered = setup
            .answer(json!({ "token": token, "allow": false, "answer": "with you please" }))
            .await;
        assert_eq!(structured(&answered)["runs"], "with_agent", "{answered:?}");

        let refused = setup.run_skill().await;
        let body = structured(&refused);
        assert_eq!(body["reason"], "with_agent", "{body}");
        assert!(body.get("consent").is_none(), "no second question: {body}");
        let skill_md = setup.state.skills.get(SKILL).await?.body;
        assert!(!skill_md.contains("run_skill"), "{skill_md}");
        Ok(())
    }

    #[tokio::test]
    async fn the_agent_can_switch_back_to_running_with_it() -> anyhow::Result<()> {
        let setup = setup("Add it to the cart.", ADD_TO_CART).await?;
        let token = setup.token().await;
        setup
            .answer(json!({ "token": token, "allow": true, "answer": "go ahead" }))
            .await;
        let back = setup.answer(json!({ "allow": false })).await;
        assert_eq!(structured(&back)["runs"], "with_agent", "{back:?}");
        let row = setup
            .state
            .skills
            .flow_row(SKILL)
            .await?
            .ok_or_else(|| anyhow::anyhow!("row"))?;
        assert!(!row.write_approved);
        assert_eq!(structured(&setup.run_skill().await)["reason"], "with_agent");
        Ok(())
    }

    #[tokio::test]
    async fn an_order_always_runs_with_the_agent_whatever_the_answer() -> anyhow::Result<()> {
        let setup = setup(
            "Place the order.",
            "async (s) => { await s.page.click({ name: 'Place your order' }) }",
        )
        .await?;
        let refused = setup.run_skill().await;
        let body = structured(&refused);
        assert_eq!(body["reason"], "always_with_agent", "{body}");
        assert!(
            body["detail"]
                .as_str()
                .is_some_and(|detail| detail.contains("places an order"))
        );

        let now = now_epoch_ms();
        let token = setup.state.consent_tokens.mint(
            consent::Pending {
                session: setup.started.session.id().as_str().to_string(),
                skill: SKILL.to_string(),
                flow_version: 1,
                commit: "anything".to_string(),
                expires_at: now + consent::TOKEN_TTL_MS,
            },
            now,
        );
        let answered = setup
            .answer(json!({ "token": token, "allow": true, "answer": "yes" }))
            .await;
        assert_eq!(
            structured(&answered)["alwaysWithAgent"],
            "order",
            "{answered:?}"
        );
        let row = setup
            .state
            .skills
            .flow_row(SKILL)
            .await?
            .ok_or_else(|| anyhow::anyhow!("row"))?;
        assert!(!row.write_approved);
        Ok(())
    }

    #[tokio::test]
    async fn a_changed_click_asks_again() -> anyhow::Result<()> {
        let setup = setup("Add it to the cart.", ADD_TO_CART).await?;
        let token = setup.token().await;
        setup
            .answer(json!({ "token": token, "allow": true, "answer": "fine" }))
            .await;
        let mut doc = store::read(&setup.state.skills.skill_dir(SKILL))?
            .ok_or_else(|| anyhow::anyhow!("flow"))?;
        doc.steps[1].src =
            "async (s) => { await s.page.click({ name: 'Add to basket' }) }".to_string();
        store::write(&setup.state.skills.skill_dir(SKILL), &doc)?;

        let asked = setup.run_skill().await;
        let body = structured(&asked);
        assert_eq!(body["reason"], "needs_consent", "{body}");
        assert_eq!(body["consent"]["reask"], true);
        assert!(text(&asked).starts_with("refused (needs_consent)"));
        assert!(text(&asked).contains("Their earlier choice covered a different click"));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flow(effects: Effects, commit: Option<u32>) -> FlowDoc {
        FlowDoc {
            meta: store::FlowMeta {
                name: "neo-shop-add".to_string(),
                flow_version: 1,
                effects,
                hosts: vec![],
                inputs: Default::default(),
                output: Default::default(),
                examples: vec![],
                source_session: None,
                prose_hash: None,
            },
            steps: (1..=4)
                .map(|n| store::FlowStep {
                    n,
                    commit: Some(n) == commit,
                    src: "async () => {}".to_string(),
                })
                .collect(),
        }
    }

    #[test]
    fn a_failed_write_flow_says_the_write_may_have_happened() {
        let text = run_failed_next(&flow(Effects::Write, Some(3)), "neo-shop-add", "timed out");
        assert!(
            text.contains("its write (step 3) may already have happened"),
            "{text}"
        );
        assert!(text.contains("never repeat that write"), "{text}");
        assert!(!text.contains("follow the skill's steps"), "{text}");

        let text = run_failed_next(&flow(Effects::Read, None), "neo-shop-add", "timed out");
        assert!(text.contains("follow the skill's steps"), "{text}");
        assert!(!text.contains("write"), "{text}");
    }
}
