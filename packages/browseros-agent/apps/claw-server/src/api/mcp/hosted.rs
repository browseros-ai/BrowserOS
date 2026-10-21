//! What `BROWSERCLAW_MODE=hosted` changes about the MCP tool surface.
//!
//! A hosted server is BrowserOS neo run as infrastructure and driven by a remote agent that
//! brings its own harness. Two groups of server tools assume things that are not true there:
//! `save_skill` and `mark_skill_run` link skills into agents installed beside the browser, and
//! `request_human_help`/`await_human_help` wait on a person watching the cockpit. A hosted
//! server does not offer them. Naming moves off the `name_session` tool onto an optional
//! `taskName` argument that rides on any call, so naming a task costs no extra round trip.
//!
//! Desktop is untouched: it lists the same tools as before and never advertises `taskName`.
//! Both modes still accept and strip `taskName`, because a catalog tool rejects unknown fields
//! and a stray one would otherwise fail the call it rode on.

use super::{
    naming::{SESSION_NAME_INPUT_MAX_LEN, normalize_small_name},
    service::{
        AWAIT_HELP_TOOL_NAME, MARK_SKILL_RUN_TOOL_NAME, NAME_SESSION_TOOL_NAME,
        REQUEST_HELP_TOOL_NAME, SAVE_SKILL_TOOL_NAME,
    },
};
use crate::config::ServerMode;
use rmcp::model::{JsonObject, Tool};
use serde_json::{Value, json};
use std::sync::Arc;

pub const TASK_NAME_ARG: &str = "taskName";
// Repeated in every listed tool's schema, so it stays short; the instructions carry the rest.
const TASK_NAME_ARG_DESCRIPTION: &str = "Optional 2-3 word label for this task, e.g. \"invoice processing\". Send it once, early; your tabs are grouped as <client>/<label>.";

const NAME_SESSION_REFUSAL: &str = "name_session is not available on this hosted BrowserOS neo server. Pass taskName=\"<2-3 word task label>\" on any tool call instead.";
const SKILLS_REFUSAL: &str = "Skills are not available on this hosted BrowserOS neo server: nothing was saved or recorded. Keep repeatable workflows in your own agent instead.";
const HUMAN_HELP_REFUSAL: &str = "Human help is not available on this hosted BrowserOS neo server. Do not wait for a human here: stop and tell the user in your chat what you need.";

/// Why the server refuses `tool` in `mode`, or `None` when the mode offers it.
///
/// Listing filters on this, and calls are refused with the returned text. Hidden tools stay
/// callable by name so an agent working from a cached tool list or an old skill learns what
/// to do instead, rather than getting a bare method-not-found.
#[must_use]
pub fn refusal(mode: ServerMode, tool: &str) -> Option<&'static str> {
    if !mode.is_hosted() {
        return None;
    }
    match tool {
        NAME_SESSION_TOOL_NAME => Some(NAME_SESSION_REFUSAL),
        SAVE_SKILL_TOOL_NAME | MARK_SKILL_RUN_TOOL_NAME => Some(SKILLS_REFUSAL),
        REQUEST_HELP_TOOL_NAME | AWAIT_HELP_TOOL_NAME => Some(HUMAN_HELP_REFUSAL),
        _ => None,
    }
}

/// Adds the optional `taskName` property to a listed tool's input schema.
#[must_use]
pub fn with_task_name_arg(mut tool: Tool) -> Tool {
    let mut schema = tool.input_schema.as_ref().clone();
    let properties = schema
        .entry("properties")
        .or_insert_with(|| Value::Object(JsonObject::new()));
    if let Value::Object(properties) = properties {
        properties.insert(
            TASK_NAME_ARG.to_string(),
            json!({
                "type": "string",
                "maxLength": SESSION_NAME_INPUT_MAX_LEN,
                "description": TASK_NAME_ARG_DESCRIPTION,
            }),
        );
    }
    tool.input_schema = Arc::new(schema);
    tool
}

/// Removes `taskName` from a call's arguments and returns the session label it asks for.
///
/// A missing or blank value is no request. A value that cannot become a label is an error,
/// returned before the tool runs, the same as any other invalid argument.
pub fn take_task_name(args: &mut Value) -> Result<Option<String>, &'static str> {
    let Some(raw) = args
        .as_object_mut()
        .and_then(|map| map.remove(TASK_NAME_ARG))
    else {
        return Ok(None);
    };
    let Value::String(raw) = raw else {
        return Err("taskName must be a string");
    };
    if raw.trim().is_empty() {
        return Ok(None);
    }
    if raw.chars().count() > SESSION_NAME_INPUT_MAX_LEN {
        return Err("taskName must be at most 64 characters");
    }
    let label = normalize_small_name(&raw);
    if label.is_empty() {
        return Err("taskName must contain letters or digits, e.g. \"invoice processing\"");
    }
    Ok(Some(label))
}

/// The reminder appended to tool results while a session still carries its generated name.
#[must_use]
pub fn rename_tip(mode: ServerMode, title: &str) -> String {
    match mode {
        ServerMode::Desktop => format!(
            "Tip: this session is \"{title}\" — rename it with name_session name=\"<2-3 word task label>\""
        ),
        ServerMode::Hosted => format!(
            "Tip: this session is \"{title}\" — name it by passing taskName=\"<2-3 word task label>\" on your next call"
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HIDDEN: [&str; 5] = [
        NAME_SESSION_TOOL_NAME,
        SAVE_SKILL_TOOL_NAME,
        MARK_SKILL_RUN_TOOL_NAME,
        REQUEST_HELP_TOOL_NAME,
        AWAIT_HELP_TOOL_NAME,
    ];

    #[test]
    fn desktop_refuses_nothing() {
        for tool in HIDDEN.into_iter().chain(["tabs", "run", "pursue"]) {
            assert_eq!(refusal(ServerMode::Desktop, tool), None, "{tool}");
        }
    }

    #[test]
    fn hosted_refuses_the_server_tools_and_keeps_the_browser_ones() {
        for tool in HIDDEN {
            assert!(refusal(ServerMode::Hosted, tool).is_some(), "{tool}");
        }
        // pursue keeps its own gate, the decision-model credential.
        for tool in ["tabs", "run", "act", "pursue"] {
            assert_eq!(refusal(ServerMode::Hosted, tool), None, "{tool}");
        }
    }

    #[test]
    fn a_refused_name_session_points_at_task_name() {
        let message = refusal(ServerMode::Hosted, NAME_SESSION_TOOL_NAME).unwrap_or_default();
        assert!(message.contains(TASK_NAME_ARG), "{message}");
    }

    #[test]
    fn task_name_is_stripped_and_normalized() {
        let mut args = json!({ "action": "list", "taskName": "  Invoice Processing! " });
        assert_eq!(
            take_task_name(&mut args),
            Ok(Some("invoice-processing".to_string()))
        );
        assert_eq!(args, json!({ "action": "list" }));
    }

    #[test]
    fn absent_or_blank_task_name_is_no_request_and_still_stripped() {
        let mut args = json!({ "action": "list" });
        assert_eq!(take_task_name(&mut args), Ok(None));
        let mut blank = json!({ "taskName": "   " });
        assert_eq!(take_task_name(&mut blank), Ok(None));
        assert_eq!(blank, json!({}));
        let mut not_an_object = json!(null);
        assert_eq!(take_task_name(&mut not_an_object), Ok(None));
    }

    #[test]
    fn unusable_task_name_is_an_error_and_still_stripped() {
        for bad in [json!(7), json!("!!!"), json!("x".repeat(65))] {
            let mut args = json!({ "taskName": bad });
            assert!(take_task_name(&mut args).is_err(), "{bad}");
            assert_eq!(args, json!({}));
        }
    }

    #[test]
    fn task_name_property_is_optional_and_bounded() {
        let tool = Tool::new(
            "tabs",
            "Manage tabs.",
            Arc::new(JsonObject::from_iter([(
                "type".to_string(),
                json!("object"),
            )])),
        );
        let schema = Value::Object(with_task_name_arg(tool).input_schema.as_ref().clone());
        assert_eq!(schema["properties"][TASK_NAME_ARG]["type"], "string");
        assert_eq!(
            schema["properties"][TASK_NAME_ARG]["maxLength"],
            SESSION_NAME_INPUT_MAX_LEN
        );
        assert_eq!(schema["required"], Value::Null);
    }

    #[test]
    fn the_tip_names_the_channel_each_mode_offers() {
        let desktop = rename_tip(ServerMode::Desktop, "codex/agile-alpaca");
        assert_eq!(
            desktop,
            "Tip: this session is \"codex/agile-alpaca\" — rename it with name_session name=\"<2-3 word task label>\""
        );
        let hosted = rename_tip(ServerMode::Hosted, "codex/agile-alpaca");
        assert!(hosted.contains("taskName=\""), "{hosted}");
        assert!(!hosted.contains("name_session"), "{hosted}");
    }
}
