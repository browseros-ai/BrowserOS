//! claw's host-local MCP tools: saving and marking neo skills, and handing a page to a
//! human through the cockpit. They plug into the shared host as `LocalTool`s and keep
//! their exact pre-extraction wire behaviour.

use crate::{
    services::{
        agent_settings::{AgentSettingsStore, HumanHelp},
        cockpit::{CockpitQuery, LiveSessionFilters},
        help::{HelpEntry, HelpOpenParams, HelpRegistry, HelpWaitOutcome},
        skill_runs::SkillRunService,
        skills::{CreateSkill, SkillOrigin, SkillService},
    },
};
use browseros_mcp::ToolResult;
use browseros_mcp_host::{
    ids::SessionId,
    mcp::dispatch::linked_cancel_token,
    seams::{LocalTool, LocalToolCall},
};
use claw_api::models::HelpRequestKind;
use futures_util::future::BoxFuture;
use rmcp::model::{CallToolResult, ContentBlock, Tool, ToolAnnotations};
use serde_json::{Value, json};
use std::{sync::Arc, time::Instant as StdInstant};
use tokio_util::sync::CancellationToken;
use ulid::Ulid;

const SAVE_SKILL_TOOL_NAME: &str = "save_skill";
const SAVE_SKILL_DESCRIPTION: &str = "When you finish a repeatable browser task the user is likely to run again, save it as a BrowserOS neo skill so it can be re-run by name later; save genuinely repeatable, user-valuable tasks, not one-offs. Give a lowercase-hyphen name, a one-line description, the ordered steps, and any shortcuts learned this run. In the steps, name the exact browser SDK calls you actually used this session (e.g. browser.wait, browser.read, browser.pages.newPage) so a later run reuses them verbatim; never invent, rename, or guess a method that is not in the run tool's SDK (there is no browser.waitFor, for example). The skill is saved and linked into your agents under a neo- prefix (neo-<name>) so it never clobbers your own skills and you can list them all by typing /neo; a name given without the prefix is namespaced automatically. Call again with the same name to update it in place.";
const MARK_SKILL_RUN_TOOL_NAME: &str = "mark_skill_run";
const MARK_SKILL_RUN_DESCRIPTION: &str = "Mark this browser session as a run of a saved skill so BrowserOS neo records the run and its cost once the session ends. Call this once, at the start, when you are running a skill, with the skill's name.";
const REQUEST_HELP_TOOL_NAME: &str = "request_human_help";
const REQUEST_HELP_DESCRIPTION: &str = "Ask a human to take over this page when you hit something only a person can do: a sign-in, a one-time code, a captcha, an account choice, or an approval you should not make yourself. Give a short `reason` (what you need), optional `details` (what the human should know), an optional `resumeHint` (what you will do after, so the human knows the task continues), and an optional `kind` (login, captcha, approval, other). This blocks for a short while and returns a status. If the status is \"waiting\", call await_human_help to keep waiting; do nothing else on the page until the status is \"resolved\". The cockpit shows your request so a human can take over the tab and hand control back.";
const AWAIT_HELP_TOOL_NAME: &str = "await_human_help";
const AWAIT_HELP_DESCRIPTION: &str = "Keep waiting on an open human-help request. Call this repeatedly after request_human_help until the status is \"resolved\" (the human handed control back), \"cancelled\" (the run was stopped), or \"timed_out\" (no human responded in time). Do nothing else on the page while waiting.";
/// For an agent whose cached tool list still offers request_human_help after the user turned
/// human help off.
const HUMAN_HELP_OFF_MESSAGE: &str = "Human help is turned off in BrowserOS neo settings. Do not wait for a human here: stop and tell the user in your chat what you need.";
/// How long each wait call blocks before returning a `waiting` status. Short enough to stay under
/// any reasonable client tool-call timeout so an agent that cannot hold a long call just re-calls.
const HELP_WAIT_CHUNK: std::time::Duration = std::time::Duration::from_secs(75);

pub(crate) struct SaveSkillTool {
    pub skills: Arc<SkillService>,
}

impl LocalTool for SaveSkillTool {
    fn definition(&self) -> Tool {
        save_skill_tool()
    }

    fn call<'a>(&'a self, call: LocalToolCall<'a>) -> BoxFuture<'a, CallToolResult> {
        Box::pin(async move {
            let Some(dispatch_id) = call.begin(CancellationToken::new()).await else {
                return call.session_gone();
            };
            let started_at = StdInstant::now();
            let session_id = call.session.id().as_str().to_string();
            let result = match parse_save_skill(call.args, session_id) {
                Ok(input) => match self.skills.upsert(input).await {
                    Ok(view) => ToolResult::text(format!("saved skill /{}", view.model.name), None),
                    Err(error) => ToolResult::error(error.to_string()),
                },
                Err(message) => ToolResult::error(message),
            };
            call.record(
                SAVE_SKILL_TOOL_NAME,
                call.args,
                &result,
                i64::try_from(started_at.elapsed().as_millis()).unwrap_or(i64::MAX),
                &dispatch_id,
            )
            .await;
            call.finish(&dispatch_id, result).await
        })
    }
}

pub(crate) struct MarkSkillRunTool {
    pub skill_runs: Arc<SkillRunService>,
}

impl LocalTool for MarkSkillRunTool {
    fn definition(&self) -> Tool {
        mark_skill_run_tool()
    }

    fn call<'a>(&'a self, call: LocalToolCall<'a>) -> BoxFuture<'a, CallToolResult> {
        Box::pin(async move {
            let Some(dispatch_id) = call.begin(CancellationToken::new()).await else {
                return call.session_gone();
            };
            let started_at = StdInstant::now();
            let session_id = call.session.id().as_str().to_string();
            let result = match parse_skill_name(call.args) {
                Ok(name) => {
                    // Skills are namespaced under neo-; accept a bare name too so a
                    // run is still recorded if the agent drops the prefix.
                    let name = crate::services::skills::neo_prefixed(&name);
                    match self.skill_runs.mark(&session_id, &name).await {
                        Ok(()) => ToolResult::text(format!("recording this run of /{name}"), None),
                        Err(error) => ToolResult::error(error.to_string()),
                    }
                }
                Err(message) => ToolResult::error(message),
            };
            call.record(
                MARK_SKILL_RUN_TOOL_NAME,
                call.args,
                &result,
                i64::try_from(started_at.elapsed().as_millis()).unwrap_or(i64::MAX),
                &dispatch_id,
            )
            .await;
            call.finish(&dispatch_id, result).await
        })
    }
}

/// Shared by both help tools: the live setting, the request registry, and the cockpit
/// projection that pins a request to the agent's active tab.
pub(crate) struct HumanHelpTools {
    pub agent_settings: Arc<AgentSettingsStore>,
    pub help: Arc<HelpRegistry>,
    pub cockpit: Arc<CockpitQuery>,
}

pub(crate) struct RequestHumanHelpTool(pub Arc<HumanHelpTools>);
pub(crate) struct AwaitHumanHelpTool(pub Arc<HumanHelpTools>);

impl LocalTool for RequestHumanHelpTool {
    fn definition(&self) -> Tool {
        request_help_tool()
    }

    /// Listed only while human help is on; a cached caller still reaches the refusal.
    fn listed(&self) -> bool {
        self.0.agent_settings.human_help() == HumanHelp::On
    }

    fn call<'a>(&'a self, call: LocalToolCall<'a>) -> BoxFuture<'a, CallToolResult> {
        Box::pin(self.0.call_request_human_help(call))
    }
}

impl LocalTool for AwaitHumanHelpTool {
    fn definition(&self) -> Tool {
        await_help_tool()
    }

    fn listed(&self) -> bool {
        self.0.agent_settings.human_help() == HumanHelp::On
    }

    fn call<'a>(&'a self, call: LocalToolCall<'a>) -> BoxFuture<'a, CallToolResult> {
        Box::pin(self.0.call_await_human_help(call))
    }
}

impl HumanHelpTools {
    async fn call_request_human_help(&self, call: LocalToolCall<'_>) -> CallToolResult {
        let raw_args = call.args;
        // Only new asks are refused: a request opened before the switch flipped still resolves
        // through await_human_help, so no waiting agent is stranded.
        if self.agent_settings.human_help() == HumanHelp::Off {
            return CallToolResult::error(vec![ContentBlock::text(HUMAN_HELP_OFF_MESSAGE)]);
        }
        let Some(reason) = raw_args
            .get("reason")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            return CallToolResult::error(vec![ContentBlock::text(
                "request_human_help: \"reason\" is required (a short line saying what you need a human to do).",
            )]);
        };
        let string_arg = |key: &str| {
            raw_args
                .get(key)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        };
        // Pin the tab the agent is on right now, so takeover targets the blocked page even when the
        // agent has several tabs open and the poll-time attribution later drifts or empties.
        let (browser_tab_id, url, title) = self.resolve_active_tab(call.session.id()).await;
        let params = HelpOpenParams {
            request_id: format!("help-{}", Ulid::new()),
            reason: reason.to_string(),
            details: string_arg("details"),
            resume_hint: string_arg("resumeHint"),
            kind: raw_args
                .get("kind")
                .and_then(Value::as_str)
                .and_then(help_kind_from_str),
            browser_tab_id,
            url,
            title,
        };
        let entry = self.help.open(call.session.id(), params).await;
        self.run_help_wait(&call, &entry).await
    }

    /// The session's most-recently-active owned tab, as the live projection sees it now. Captured at
    /// request time so a help request pins the exact blocked tab. Returns all-None when the session
    /// has no attributed tab, in which case the snapshot falls back to the poll-time pick.
    async fn resolve_active_tab(
        &self,
        session_id: &SessionId,
    ) -> (Option<i64>, Option<String>, Option<String>) {
        let filters = LiveSessionFilters {
            profile_id: None,
            slug: None,
            site: None,
            search: None,
            since: None,
        };
        let Ok(sessions) = self.cockpit.list(&filters).await else {
            return (None, None, None);
        };
        let tab = sessions
            .into_iter()
            .find(|projection| projection.task.session_id == session_id.as_str())
            .and_then(|projection| {
                projection
                    .live
                    .browser_tabs
                    .into_iter()
                    .max_by_key(|tab| tab.last_activity_at.unwrap_or(0))
            });
        match tab {
            Some(tab) => (Some(tab.browser_tab_id), Some(tab.url), Some(tab.title)),
            None => (None, None, None),
        }
    }

    async fn call_await_human_help(&self, call: LocalToolCall<'_>) -> CallToolResult {
        match self.help.get(call.session.id()).await {
            Some(entry) => self.run_help_wait(&call, &entry).await,
            // No entry is the safe-terminal case, not a resume: the request may have
            // expired and been reaped before this reattach, so reporting "resolved"
            // could tell the agent a human helped when none did. "timed_out" never
            // falsely clears a block; the agent decides whether to ask again or stop.
            None => ToolResult::text(
                "No open human-help request is waiting (it was never opened or has already ended). A human did not hand control back here, so do not assume the block is cleared: decide whether to ask again with request_human_help or stop.",
                Some(json!({ "status": "timed_out" })),
            )
            .into_call_tool_result(),
        }
    }

    async fn run_help_wait(&self, call: &LocalToolCall<'_>, entry: &Arc<HelpEntry>) -> CallToolResult {
        let dispatch_cancel = CancellationToken::new();
        let Some(dispatch_id) = call.begin(dispatch_cancel.clone()).await else {
            return call.session_gone();
        };
        // A terminal cancel (cockpit Stop via the dispatch token, or session teardown via the
        // child token) discards the request; the per-call request token only ends this wait, so a
        // client that times out one chunk and re-calls keeps its request. The never-cancelled
        // placeholder keeps the request token out of the terminal path.
        let terminal = linked_cancel_token(
            call.session.child_token(),
            CancellationToken::new(),
            dispatch_cancel.clone(),
        );
        let outcome = self
            .help
            .wait_chunk(
                call.session.id(),
                entry,
                HELP_WAIT_CHUNK,
                &terminal,
                &call.request_cancel,
            )
            .await;
        let result = help_wait_result(entry.request_id(), outcome);
        call.finish(&dispatch_id, result).await
    }
}

fn save_skill_tool() -> Tool {
    let Value::Object(input_schema) = json!({
        "type": "object",
        "properties": {
            "name": { "type": "string", "pattern": "^[a-z0-9-]+$" },
            "description": { "type": "string" },
            "steps": { "type": "array", "items": { "type": "string" } },
            "learnedNotes": { "type": "array", "items": { "type": "string" } },
            "site": { "type": "string" }
        },
        "required": ["name", "description"]
    }) else {
        unreachable!();
    };
    Tool::new(SAVE_SKILL_TOOL_NAME, SAVE_SKILL_DESCRIPTION, input_schema).with_annotations(
        ToolAnnotations::with_title("Save skill")
            .read_only(false)
            .destructive(false)
            .idempotent(true),
    )
}

fn mark_skill_run_tool() -> Tool {
    let Value::Object(input_schema) = json!({
        "type": "object",
        "properties": {
            "name": { "type": "string", "pattern": "^[a-z0-9-]+$" }
        },
        "required": ["name"]
    }) else {
        unreachable!();
    };
    Tool::new(
        MARK_SKILL_RUN_TOOL_NAME,
        MARK_SKILL_RUN_DESCRIPTION,
        input_schema,
    )
    .with_annotations(
        ToolAnnotations::with_title("Mark skill run")
            .read_only(false)
            .destructive(false)
            .idempotent(true),
    )
}

fn request_help_tool() -> Tool {
    let Value::Object(input_schema) = json!({
        "type": "object",
        "properties": {
            "reason": {
                "type": "string",
                "description": "A short line saying what you need a human to do, e.g. \"Enter the LinkedIn verification code\"."
            },
            "details": {
                "type": "string",
                "description": "What the human should know to help, e.g. where the code was sent."
            },
            "resumeHint": {
                "type": "string",
                "description": "What you will do after, so the human knows the task continues, e.g. \"I'll pick up at profile 15 of 22\"."
            },
            "kind": {
                "type": "string",
                "enum": ["login", "captcha", "approval", "other"],
                "description": "The kind of help, used for the cockpit icon."
            }
        },
        "required": ["reason"]
    }) else {
        unreachable!();
    };
    Tool::new(
        REQUEST_HELP_TOOL_NAME,
        REQUEST_HELP_DESCRIPTION,
        input_schema,
    )
    .with_annotations(
        ToolAnnotations::with_title("Request human help")
            .read_only(false)
            .destructive(false)
            .idempotent(true),
    )
}

fn await_help_tool() -> Tool {
    let Value::Object(input_schema) = json!({
        "type": "object",
        "properties": {
            "requestId": {
                "type": "string",
                "description": "The requestId returned by request_human_help. Optional; the server keys the wait by session."
            }
        }
    }) else {
        unreachable!();
    };
    Tool::new(AWAIT_HELP_TOOL_NAME, AWAIT_HELP_DESCRIPTION, input_schema).with_annotations(
        ToolAnnotations::with_title("Await human help")
            .read_only(true)
            .destructive(false)
            .idempotent(true),
    )
}

fn help_kind_from_str(value: &str) -> Option<HelpRequestKind> {
    match value {
        "login" => Some(HelpRequestKind::Login),
        "captcha" => Some(HelpRequestKind::Captcha),
        "approval" => Some(HelpRequestKind::Approval),
        "other" => Some(HelpRequestKind::Other),
        _ => None,
    }
}

fn help_wait_result(request_id: &str, outcome: HelpWaitOutcome) -> ToolResult {
    match outcome {
        HelpWaitOutcome::Resolved { note } => {
            let text = match note.as_deref() {
                Some(note) if !note.is_empty() => format!(
                    "A human finished and handed control back. Note: {note}. Continue the task."
                ),
                _ => "A human finished and handed control back. Continue the task.".to_string(),
            };
            ToolResult::text(
                text,
                Some(json!({ "status": "resolved", "requestId": request_id, "note": note })),
            )
        }
        HelpWaitOutcome::Waiting { elapsed_seconds } => ToolResult::text(
            format!(
                "Still waiting for a human ({elapsed_seconds}s so far). Call await_human_help to keep waiting; do nothing else on the page."
            ),
            Some(
                json!({ "status": "waiting", "requestId": request_id, "elapsedSeconds": elapsed_seconds }),
            ),
        ),
        HelpWaitOutcome::Cancelled => ToolResult::text(
            "The human-help wait was stopped. Do not continue this task.",
            Some(json!({ "status": "cancelled", "requestId": request_id })),
        ),
        // This wait call was interrupted but the request is still open; if the agent sees this, it
        // should reattach with await_human_help rather than act on the page.
        HelpWaitOutcome::Interrupted => ToolResult::text(
            "The wait call ended before a human responded. Call await_human_help to keep waiting; do nothing else on the page.",
            Some(json!({ "status": "waiting", "requestId": request_id })),
        ),
        HelpWaitOutcome::TimedOut => ToolResult::text(
            "No human responded in time. Decide whether to ask again with request_human_help or stop.",
            Some(json!({ "status": "timed_out", "requestId": request_id })),
        ),
    }
}

fn parse_skill_name(raw_args: &Value) -> Result<String, String> {
    raw_args
        .get("name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| "name must be a non-empty string".to_string())
}

fn parse_save_skill(raw_args: &Value, session_id: String) -> Result<CreateSkill, String> {
    let name = raw_args
        .get("name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or("name must be a non-empty string")?
        .to_string();
    let description = raw_args
        .get("description")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or("description must be a non-empty string")?
        .to_string();
    let steps = parse_string_array(raw_args.get("steps"), "steps")?;
    let learned_notes = parse_string_array(raw_args.get("learnedNotes"), "learnedNotes")?;
    let site = raw_args
        .get("site")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    Ok(CreateSkill {
        name,
        description,
        site,
        steps,
        learned_notes,
        origin: SkillOrigin::Agent,
        source_session_id: Some(session_id),
    })
}

fn parse_string_array(value: Option<&Value>, field: &str) -> Result<Vec<String>, String> {
    match value {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| {
                item.as_str()
                    .map(str::to_string)
                    .ok_or_else(|| format!("{field} entries must be strings"))
            })
            .collect(),
        Some(_) => Err(format!("{field} must be an array of strings")),
    }
}

