//! What `BROWSERCLAW_HOSTED_MODE=1` changes about the MCP tool surface.
//!
//! A hosted server is BrowserOS neo run as infrastructure and driven by a remote agent that
//! brings its own harness. Two groups of server tools assume things that are not true there:
//! `save_skill` and `mark_skill_run` link skills into agents installed beside the browser, and
//! `request_human_help`/`await_human_help` wait on a person watching the cockpit. A hosted
//! server does not offer them. Hosted agents also manage their own tabs and task labels:
//! the server neither groups their tabs nor asks them to name tasks.
//!
//! Both modes strip legacy `taskName` before a catalog tool sees its strict arguments.
//! Desktop keeps accepting it; hosted ignores it, including malformed values.

use super::{
    naming::{SESSION_NAME_INPUT_MAX_LEN, normalize_small_name},
    service::{
        AWAIT_HELP_TOOL_NAME, MARK_SKILL_RUN_TOOL_NAME, NAME_SESSION_TOOL_NAME,
        REQUEST_HELP_TOOL_NAME, SAVE_SKILL_TOOL_NAME,
    },
};
use crate::config::ServerMode;
use rmcp::model::Tool;
use serde_json::Value;
use std::sync::Arc;

pub const TASK_NAME_ARG: &str = "taskName";
const NAME_SESSION_REFUSAL: &str = "name_session is not available on this hosted BrowserOS neo server. Continue with the browser tools; task naming is not required.";
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

/// Hosted tool guidance describes explicit grouping without requiring task groups.
#[must_use]
pub fn with_tool_guidance(mut tool: Tool) -> Tool {
    let mut schema = tool.input_schema.as_ref().clone();
    if let Some(Value::Object(properties)) = schema.get_mut("properties") {
        if let Some(Value::Object(session)) = properties.get_mut("session") {
            session.insert("description".into(), Value::String(
                "Opaque logical task handle, including its browser_target binding. If returned under com.browseros.neo/session in a tool result, pass it as session on later calls. Omit on the first call or to start a separate task. Stopped handles stay stopped; stale browser targets require explicit reselection or clear.".into()
            ));
        }
        if tool.name == "tabs" {
            if let Some(Value::Object(group)) = properties.get_mut("groupId") {
                group.insert("description".into(), Value::String(
                    "Optional destination tab group for action=\"new\". Omit to open an ungrouped tab.".into()
                ));
            }
            let mut description = "Manage browser tabs: list open pages and their page ids, show the active page, open a new background page, or close one. New pages are ready once they stop changing; do not pause afterwards. Use returned page ids with snapshot/act/navigate. Tabs stay ungrouped unless you explicitly request a group.".to_string();
            // The catalog appends native capability guidance after the base description.
            if let Some((_, note)) = tool
                .description
                .as_deref()
                .and_then(|text| text.split_once("\n\n"))
            {
                description.push_str("\n\n");
                description.push_str(note);
            }
            tool.description = Some(description.into());
        }
    }
    if tool.name == "tab_groups"
        && let Some(description) = tool.description.as_deref()
    {
        tool.description = Some(description.replace(
            " To resume earlier work after a reconnect, pass the remembered group id as groupId on tabs action=\"new\" instead, which also makes that group's tabs read as yours again.",
            "",
        ).into());
    }
    tool.input_schema = Arc::new(schema);
    tool
}

/// Removes `taskName` from a call's arguments and returns the session label it asks for.
///
/// Hosted ignores the legacy argument. On desktop a missing or blank value is no request.
/// A desktop value that cannot become a label is an error,
/// returned before the tool runs, the same as any other invalid argument.
pub fn take_task_name(args: &mut Value, mode: ServerMode) -> Result<Option<String>, &'static str> {
    let Some(raw) = args
        .as_object_mut()
        .and_then(|map| map.remove(TASK_NAME_ARG))
    else {
        return Ok(None);
    };
    if mode.is_hosted() {
        return Ok(None);
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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
    fn tests_that_hosted_name_refusal_does_not_request_a_name() {
        let message = refusal(ServerMode::Hosted, NAME_SESSION_TOOL_NAME).unwrap_or_default();
        assert!(!message.contains(TASK_NAME_ARG), "{message}");
    }

    #[test]
    fn task_name_is_stripped_and_normalized() {
        let mut args = json!({ "action": "list", "taskName": "  Invoice Processing! " });
        assert_eq!(
            take_task_name(&mut args, ServerMode::Desktop),
            Ok(Some("invoice-processing".to_string()))
        );
        assert_eq!(args, json!({ "action": "list" }));
    }

    #[test]
    fn absent_or_blank_task_name_is_no_request_and_still_stripped() {
        let mut args = json!({ "action": "list" });
        assert_eq!(take_task_name(&mut args, ServerMode::Desktop), Ok(None));
        let mut blank = json!({ "taskName": "   " });
        assert_eq!(take_task_name(&mut blank, ServerMode::Desktop), Ok(None));
        assert_eq!(blank, json!({}));
        let mut not_an_object = json!(null);
        assert_eq!(
            take_task_name(&mut not_an_object, ServerMode::Desktop),
            Ok(None)
        );
    }

    #[test]
    fn unusable_task_name_is_an_error_and_still_stripped() {
        for bad in [json!(7), json!("!!!"), json!("x".repeat(65))] {
            let mut args = json!({ "taskName": bad });
            assert!(
                take_task_name(&mut args, ServerMode::Desktop).is_err(),
                "{bad}"
            );
            assert_eq!(args, json!({}));
        }
    }

    #[test]
    fn tests_that_hosted_task_name_is_ignored_and_stripped() {
        for old in [
            json!("Invoice Processing"),
            json!(7),
            json!("!!!"),
            json!("x".repeat(65)),
        ] {
            let mut args = json!({"action":"list", "taskName":old});
            assert_eq!(take_task_name(&mut args, ServerMode::Hosted), Ok(None));
            assert_eq!(args, json!({"action":"list"}));
        }
    }
}
