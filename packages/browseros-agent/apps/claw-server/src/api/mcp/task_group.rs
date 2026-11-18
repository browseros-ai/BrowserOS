//! The tab group as the `browseros` command line's only task identity.
//!
//! A CLI is one process per command, so it cannot hold a connection the way an agent
//! harness does, and it is not asked to carry a session either. It carries the group its
//! task works in, which it owns anyway, and the server resolves the task from that.

use rmcp::{
    ErrorData as McpError,
    model::{CallToolResult, MetaObject},
};
use serde_json::Value;

/// The `clientInfo.name` the `browseros` command line sends on every call.
///
/// Every group-keyed behaviour is gated on this name, so an MCP client never reaches one.
pub const CLI_CLIENT_NAME: &str = "browseros-cli";

/// The argument a CLI call presents its group in.
pub(super) const FIELD: &str = "group";

/// What a CLI call gets on a computer without the ultrafast plan. The prefix is stable so the
/// CLI can tell this refusal from a tool's own error and exit with a code of its own.
pub const NOT_ON_PLAN: &str = "ultrafast_required: the browseros command line is part of the ultrafast plan, which this computer does not have. Open BrowserOS neo to get ultrafast.";

/// The end-of-task reminder a CLI task gets: the command the CLI has, not the tab group it
/// would otherwise be told to close by hand.
pub(super) const DONE_TIP: &str =
    "Tip: when the user says this task is done, run: browseros task done";

/// The `_meta` key a CLI result names its task's group under, namespaced like the handle's.
pub const META_KEY: &str = "com.browseros.neo/group";

#[must_use]
pub fn is_cli_client(name: &str) -> bool {
    name == CLI_CLIENT_NAME
}

/// The group a CLI call presented, removed from `args` so no tool receives an argument its
/// schema never declared. Only called for the CLI, so an MCP client's arguments reach its
/// tools exactly as sent.
pub(super) fn take(args: &mut Value) -> Option<String> {
    args.as_object_mut()?
        .remove(FIELD)?
        .as_str()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

/// The group a `tab_groups` close names, which for a CLI task is the user's "done".
pub(super) fn closing_group(tool: &str, args: &Value) -> Option<String> {
    if tool != "tab_groups" || args.get("action").and_then(Value::as_str) != Some("close") {
        return None;
    }
    args.get("groupId")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|group| !group.is_empty())
        .map(str::to_string)
}

/// The rename reminder a CLI task gets in place of the session one. It names the
/// command the CLI has, and says task, since the CLI has no session to rename.
#[must_use]
pub(super) fn rename_tip(title: &str) -> String {
    format!(
        "Tip: this task is \"{title}\". Name it with: browseros task name \"<2-3 word task label>\""
    )
}

/// `result` naming the group the CLI task works in now, in `_meta` only. The CLI reads it
/// there and remembers it for the next command. Nothing is added to the text the agent
/// reads: the point of the model is that the agent carries nothing. No group means the task
/// has none open, which is how the CLI learns a task it was in has ended.
pub(super) fn attach(
    result: Result<CallToolResult, McpError>,
    group: Option<&str>,
) -> Result<CallToolResult, McpError> {
    let Some(group) = group else {
        return result;
    };
    result.map(|mut call_result| {
        call_result
            .meta
            .get_or_insert_with(MetaObject::new)
            .insert(META_KEY.to_string(), Value::String(group.to_string()));
        call_result
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn only_the_cli_name_is_the_cli() {
        assert!(is_cli_client("browseros-cli"));
        assert!(!is_cli_client("Codex"));
        assert!(!is_cli_client("claude-code"));
        assert!(!is_cli_client(""));
    }

    #[test]
    fn the_group_is_stripped_and_trimmed() {
        let mut args = json!({ "group": "  G1 ", "url": "https://example.com" });
        assert_eq!(take(&mut args), Some("G1".to_string()));
        assert_eq!(args, json!({ "url": "https://example.com" }));
    }

    #[test]
    fn a_blank_or_absent_group_presents_nothing() {
        let mut blank = json!({ "group": "   " });
        assert_eq!(take(&mut blank), None);
        assert_eq!(blank, json!({}), "even a blank group never reaches a tool");

        let mut absent = json!({ "url": "https://example.com" });
        assert_eq!(take(&mut absent), None);
        assert_eq!(absent, json!({ "url": "https://example.com" }));
    }

    #[test]
    fn only_a_tab_groups_close_names_a_closing_group() {
        let close = json!({ "action": "close", "groupId": " G1 " });
        assert_eq!(closing_group("tab_groups", &close), Some("G1".to_string()));
        assert_eq!(
            closing_group(
                "tab_groups",
                &json!({ "action": "update", "groupId": "G1" })
            ),
            None
        );
        assert_eq!(
            closing_group("tab_groups", &json!({ "action": "close" })),
            None
        );
        assert_eq!(closing_group("tabs", &close), None);
    }

    #[test]
    fn the_group_rides_in_meta_and_never_in_the_text() {
        let result = CallToolResult::success(vec![rmcp::model::ContentBlock::text("opened")]);

        let attached = attach(Ok(result.clone()), Some("G1"))
            .unwrap_or_else(|error| panic!("attach failed: {error:?}"));

        assert_eq!(
            attached
                .meta
                .as_ref()
                .and_then(|meta| meta.get(META_KEY))
                .and_then(Value::as_str),
            Some("G1")
        );
        assert_eq!(
            attached.content, result.content,
            "the agent's text is untouched"
        );
    }

    #[test]
    fn no_open_group_leaves_the_result_as_it_was() {
        let result = CallToolResult::success(vec![rmcp::model::ContentBlock::text("done")]);
        let attached = attach(Ok(result.clone()), None)
            .unwrap_or_else(|error| panic!("attach failed: {error:?}"));
        assert_eq!(attached, result);
    }
}
