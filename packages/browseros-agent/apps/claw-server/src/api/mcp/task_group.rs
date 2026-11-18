//! The tab group as the `browseros` command line's only task identity.
//!
//! A CLI is one process per command, so it cannot hold a connection the way an agent
//! harness does, and it is not asked to carry a session either. It carries the group its
//! task works in, which it owns anyway, and the server resolves the task from that.

use serde_json::Value;

/// The `clientInfo.name` the `browseros` command line sends on every call.
///
/// Every group-keyed behaviour is gated on this name, so an MCP client never reaches one.
pub const CLI_CLIENT_NAME: &str = "browseros-cli";

/// The argument a CLI call presents its group in.
pub(super) const FIELD: &str = "group";

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
}
