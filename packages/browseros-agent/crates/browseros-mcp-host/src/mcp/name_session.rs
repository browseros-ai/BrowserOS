//! `name_session`: the one local tool every host serves. It labels the session's tab
//! group `<client>/<name>`, declares the task category to analytics once, and stores a
//! PII-scrubbed summary through the session journal for later search.

use crate::{
    mcp::{
        effects::tab_groups::apply_agent_tab_group_title,
        naming::{normalize_small_name, session_group_title},
    },
    seams::{HostEvent, LocalTool, LocalToolCall, TASK_CATEGORY_VALUES},
    sessions::Session,
};
use browseros_mcp::ToolResult;
use futures_util::future::BoxFuture;
use rmcp::model::{CallToolResult, Tool, ToolAnnotations};
use serde_json::{Value, json};
use std::time::Instant as StdInstant;
use tokio_util::sync::CancellationToken;
use tracing::warn;

pub const NAME_SESSION_TOOL_NAME: &str = "name_session";
const NAME_SESSION_DESCRIPTION: &str = "Name this browser session at the start of a task: a small lowercase 2-3 word label for what it is doing, e.g. \"invoice processing\", a `category` for the kind of task, and a short `summary`. Tabs are grouped as <client>/<name>; the label stays on this machine, the summary powers audit search and is also recorded for analytics, and the category is used for anonymous aggregate analytics. Call again to update.";
const NAME_SESSION_CATEGORY_DESCRIPTION: &str = "The kind of task, for anonymous aggregate analytics only; the free-form name is never sent. Pick the closest fit from the list.";
const NAME_SESSION_SUMMARY_DESCRIPTION: &str = "One or two short lines saying what this task is, phrased so you can find it again by searching later. No names, emails, URLs, file paths, or account numbers.";
pub(crate) const SUMMARY_MAX_LEN: usize = 200;
const NAME_SESSION_INPUT_MAX_LEN: usize = 64;

/// Registered first by every `HostCtx`.
pub(crate) struct NameSessionTool;

impl LocalTool for NameSessionTool {
    fn definition(&self) -> Tool {
        name_session_tool()
    }

    fn call<'a>(&'a self, call: LocalToolCall<'a>) -> BoxFuture<'a, CallToolResult> {
        Box::pin(call_name_session(call))
    }
}

async fn call_name_session(call: LocalToolCall<'_>) -> CallToolResult {
    let Some(dispatch_id) = call.begin(CancellationToken::new()).await else {
        return call.session_gone();
    };
    let session = call.session;
    let raw_args = call.args;
    let started_at = StdInstant::now();
    let rename = match rename_session(Some(session.as_ref()), raw_args).await {
        Ok(rename) => rename,
        Err(message) => return call.finish(&dispatch_id, ToolResult::error(message)).await,
    };
    // Scrub structural PII from any provided summary before it is stored, indexed for
    // search, or recorded on the task-declared analytics event. Last write wins locally.
    let scrubbed_summary = raw_args
        .get("summary")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|summary| !summary.is_empty())
        .map(scrub_summary);
    if let Some(category) = raw_args
        .get("category")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|category| !category.is_empty())
    {
        // At most once per session: a later name_session rename must not
        // re-declare and overcount the category mix or the declaration rate.
        if session.try_mark_task_declared() {
            let mut properties = json!({
                "task_category": category,
                "client_name": session.client_name(),
            });
            // The scrubbed summary rides along with the category declaration; the
            // analytics layer bounds it defensively before it leaves the machine.
            if let Some(summary) = scrubbed_summary
                .as_deref()
                .filter(|summary| !summary.is_empty())
            {
                properties["task_summary"] = Value::String(summary.to_string());
            }
            call.host
                .analytics()
                .capture(HostEvent::SessionTaskDeclared, properties);
        }
    }
    if let Some(clean) = scrubbed_summary.as_deref()
        && !clean.is_empty()
        && let Err(error) = call
            .host
            .journal()
            .set_task_summary(session.id().as_str(), clean)
            .await
    {
        warn!(error = %error, "failed to store task summary");
    }
    let browser = call.host.browser.session().await;
    apply_agent_tab_group_title(
        browser.as_ref(),
        &call.host.sessions.ownership(),
        session.convo_id(),
        session.as_ref(),
        session.child_token(),
    )
    .await;
    let result = ToolResult::text(rename.response, None);
    // The audit dispatch persists the raw tool arguments; substitute the scrubbed
    // summary so the unsanitized text never reaches the audit detail timeline.
    let dispatch_args = match scrubbed_summary.as_deref() {
        Some(clean) => with_scrubbed_summary(raw_args, clean),
        None => raw_args.clone(),
    };
    call.record(
        NAME_SESSION_TOOL_NAME,
        &dispatch_args,
        &result,
        i64::try_from(started_at.elapsed().as_millis()).unwrap_or(i64::MAX),
        &dispatch_id,
    )
    .await;
    call.finish(&dispatch_id, result).await
}

#[derive(Debug, PartialEq, Eq)]
struct SessionRename {
    response: String,
}

async fn rename_session(
    session: Option<&Session>,
    raw_args: &Value,
) -> Result<SessionRename, &'static str> {
    let Some(session) = session else {
        return Err("unable to resolve this session");
    };
    let Some(raw_name) = raw_args.get("name").and_then(Value::as_str) else {
        return Err("name must be a string");
    };
    if raw_name.chars().count() > NAME_SESSION_INPUT_MAX_LEN {
        return Err("name must be at most 64 characters");
    }
    let label = normalize_small_name(raw_name);
    if label.is_empty() {
        return Err("name must contain a usable session name");
    }

    let old_label = session.rename(label.clone()).await;
    let old_title = session_group_title(session, &old_label);
    let new_title = session_group_title(session, &label);
    Ok(SessionRename {
        response: format!("renamed to {new_title} (was {old_title})"),
    })
}

/// Best-effort structural PII scrub for an agent-provided task summary before it is
/// stored and indexed for search: drops any whitespace token that looks like an email,
/// URL, file path, bare domain/filename, or a long digit run (phone / card / account
/// number). Collapses whitespace and caps the length. Free prose and names are kept;
/// the agent is instructed to omit those, and the summary never leaves this machine.
fn scrub_summary(raw: &str) -> String {
    let scrubbed = raw
        .split_whitespace()
        .filter(|token| !is_pii_token(token))
        .collect::<Vec<_>>()
        .join(" ");
    if scrubbed.chars().count() > SUMMARY_MAX_LEN {
        scrubbed
            .chars()
            .take(SUMMARY_MAX_LEN)
            .collect::<String>()
            .trim_end()
            .to_string()
    } else {
        scrubbed
    }
}

/// Clones the tool arguments with the `summary` field replaced by its already-scrubbed
/// form, so the audit dispatch timeline persists the sanitized summary rather than the raw
/// one the scrubber removed from `tasks.task_summary` and the search index.
fn with_scrubbed_summary(raw_args: &Value, clean: &str) -> Value {
    let mut owned = raw_args.clone();
    if let Some(object) = owned.as_object_mut() {
        object.insert("summary".to_string(), Value::String(clean.to_string()));
    }
    owned
}

fn is_pii_token(token: &str) -> bool {
    let lower = token.to_ascii_lowercase();
    if token.contains('@')
        || lower.contains("://")
        || lower.starts_with("www.")
        || token.contains('/')
        || token.contains('\\')
    {
        return true;
    }
    if token.chars().filter(|c| c.is_ascii_digit()).count() >= 7 {
        return true;
    }
    // bare domains / filenames: example.com, crm.internal.acme.com, report.pdf
    if let Some((prefix, suffix)) = lower.rsplit_once('.') {
        return !prefix.is_empty()
            && (2..=24).contains(&suffix.len())
            && suffix.chars().all(|c| c.is_ascii_alphabetic());
    }
    false
}

fn name_session_tool() -> Tool {
    let Value::Object(input_schema) = json!({
        "type": "object",
        "properties": {
            "name": { "type": "string", "maxLength": NAME_SESSION_INPUT_MAX_LEN },
            "category": {
                "type": "string",
                "enum": TASK_CATEGORY_VALUES,
                "description": NAME_SESSION_CATEGORY_DESCRIPTION
            },
            "summary": {
                "type": "string",
                "maxLength": SUMMARY_MAX_LEN,
                "description": NAME_SESSION_SUMMARY_DESCRIPTION
            }
        },
        "required": ["name"]
    }) else {
        unreachable!();
    };
    Tool::new(
        NAME_SESSION_TOOL_NAME,
        NAME_SESSION_DESCRIPTION,
        input_schema,
    )
    .with_annotations(
        ToolAnnotations::with_title("Name session")
            .read_only(false)
            .destructive(false)
            .idempotent(true),
    )
}
