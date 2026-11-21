//! Governs discovery and cached MCP calls on the Playwright tool list.
//! Pursue still dispatches catalog tools inside the host.

use super::super::{
    AWAIT_HELP_TOOL_NAME, ClawMcpService, PURSUE_TOOL_NAME, PageNaming, REQUEST_HELP_TOOL_NAME,
    pursue,
};
use crate::services::agent_settings::HumanHelp;
use browseros_mcp::ToolResult;
use serde_json::Value;

/// Hides the shared tools when false. Cached waits can still drain and cached
/// pursue calls receive a handback.
pub(super) const LIST_EXCEPTIONS: bool = true;
const EXCEPTIONS: [&str; 3] = [
    PURSUE_TOOL_NAME,
    REQUEST_HELP_TOOL_NAME,
    AWAIT_HELP_TOOL_NAME,
];
#[derive(Clone, Copy)]
enum Exposure {
    Run,
    Exception,
    Retired,
    Internal,
}

const REMOVED_TOOLS: &[(&str, Exposure)] = &[
    ("navigate", Exposure::Internal),
    ("snapshot", Exposure::Internal),
    ("diff", Exposure::Internal),
    ("act", Exposure::Internal),
    ("wait", Exposure::Internal),
    ("read", Exposure::Internal),
    ("grep", Exposure::Internal),
    ("download", Exposure::Internal),
    ("upload", Exposure::Internal),
    ("screenshot", Exposure::Internal),
    ("pdf", Exposure::Internal),
    ("tabs", Exposure::Internal),
    ("windows", Exposure::Internal),
    ("history", Exposure::Internal),
    ("credentials", Exposure::Internal),
    ("evaluate_in_page", Exposure::Internal),
    ("read_output", Exposure::Internal),
    ("tab_groups", Exposure::Internal),
    ("browser_profiles", Exposure::Internal),
    ("browser_target", Exposure::Internal),
    ("payment_cards", Exposure::Internal),
    ("setViewport", Exposure::Internal),
    ("name_session", Exposure::Retired),
    ("save_skill", Exposure::Retired),
    ("mark_skill_run", Exposure::Retired),
    ("run_skill", Exposure::Retired),
    ("solve_captcha", Exposure::Retired),
];

fn exposure(name: &str) -> Exposure {
    if name == "run" {
        Exposure::Run
    } else if EXCEPTIONS.contains(&name) {
        Exposure::Exception
    } else {
        REMOVED_TOOLS
            .iter()
            .find_map(|(removed, exposure)| (*removed == name).then_some(*exposure))
            .unwrap_or(Exposure::Internal)
    }
}

/// Cached exception calls need terminal answers even when the tool is no longer listed.
#[derive(Debug, PartialEq, Eq)]
pub(in crate::api::mcp::service) enum Admission {
    Call,
    Refuse(&'static str),
    PursueHandback,
}

/// One matcher for host-authored desktop prose. Ordinary English and SDK method
/// names remain valid; only references to the retired MCP tools are forbidden.
#[cfg(test)]
pub(in crate::api::mcp::service) fn removed_tool_reference(text: &str) -> Option<&'static str> {
    for &(name, _) in REMOVED_TOOLS {
        if name.contains('_') || name == "setViewport" {
            if text.contains(name) {
                return Some(name);
            }
        } else if text.contains(&format!("`{name}`"))
            || text
                .match_indices(&format!("{name} action="))
                .any(|(index, _)| {
                    index == 0
                        || text[..index]
                            .chars()
                            .next_back()
                            .is_none_or(|ch| !ch.is_alphanumeric() && ch != '_')
                })
        {
            return Some(name);
        }
    }
    ["snapshot and act", "from tabs or navigate", "tabs action="]
        .into_iter()
        .find(|phrase| text.contains(phrase))
}

const REMOVED_MESSAGE: &str =
    "This tool is no longer available over desktop MCP. Use run with Playwright instead.";
const HELP_OFF_MESSAGE: &str = "Human help is turned off in BrowserOS neo settings. Use run to continue only if no human is needed; otherwise stop and tell the user in your chat what you need.";

/// One policy snapshot per request. Listing gates can change while cached callers
/// still need a terminal response from an exception they already started.
pub(in crate::api::mcp::service) struct Surface {
    pub(in crate::api::mcp::service) list_exceptions: bool,
    pub(in crate::api::mcp::service) human_help: HumanHelp,
    pub(in crate::api::mcp::service) pursue_offered: bool,
}

impl Surface {
    pub(in crate::api::mcp::service) fn for_host(host: &ClawMcpService) -> Self {
        Self {
            list_exceptions: LIST_EXCEPTIONS,
            human_help: host.human_help(),
            pursue_offered: host.pursue_offered(),
        }
    }

    pub(in crate::api::mcp::service) fn listed(&self, name: &str) -> bool {
        match exposure(name) {
            Exposure::Run => true,
            Exposure::Exception if self.list_exceptions => match name {
                PURSUE_TOOL_NAME => self.pursue_offered,
                REQUEST_HELP_TOOL_NAME | AWAIT_HELP_TOOL_NAME => self.human_help == HumanHelp::On,
                _ => unreachable!("exception names come from EXCEPTIONS"),
            },
            Exposure::Exception | Exposure::Retired | Exposure::Internal => false,
        }
    }

    pub(in crate::api::mcp::service) fn admission(&self, name: &str) -> Admission {
        match exposure(name) {
            Exposure::Run => Admission::Call,
            Exposure::Exception => match name {
                PURSUE_TOOL_NAME if !self.list_exceptions => Admission::PursueHandback,
                REQUEST_HELP_TOOL_NAME
                    if !self.list_exceptions || self.human_help == HumanHelp::Off =>
                {
                    Admission::Refuse(HELP_OFF_MESSAGE)
                }
                // Await drains a request opened before help was disabled. Pursue
                // rechecks its own plan and worker before starting any decisions.
                _ => Admission::Call,
            },
            Exposure::Internal | Exposure::Retired => Admission::Refuse(REMOVED_MESSAGE),
        }
    }
}

/// Answer a cached pursue caller without opening a session or contacting the worker.
pub(super) fn gate_off(raw_args: &Value) -> ToolResult {
    let Some(page) = PageNaming::NativeTab.pursue_page(raw_args) else {
        return ToolResult::error("pursue needs page.tabId from a run script");
    };
    pursue::not_started(page, "fast browsing is turned off")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gate_off_returns_a_native_tab_handback_without_work() {
        let result = gate_off(&serde_json::json!({"page": 5000000000i64}));
        assert!(!result.is_error);
        let body = result.structured_content.unwrap_or_default();
        assert_eq!(body["page"], 5000000000i64);
        assert_eq!(body["continueWith"], "run");
        assert_eq!(body["decisions"], 0);
        for reply in [
            pursue::not_started(7, "the worker is unavailable"),
            pursue::already_handed_back(7, PageNaming::NativeTab),
        ] {
            for block in reply.content {
                if let Some(text) = block.as_text() {
                    assert_eq!(removed_tool_reference(&text.text), None);
                }
            }
        }
    }

    #[test]
    fn wording_matcher_distinguishes_tool_references_from_ordinary_words() {
        for &(name, _) in REMOVED_TOOLS {
            assert!(
                removed_tool_reference(&format!("Use `{name}` here")).is_some(),
                "{name}"
            );
            assert!(
                removed_tool_reference(&format!("Use {name} action=list")).is_some(),
                "{name}"
            );
            if name.contains('_') || name == "setViewport" {
                assert!(
                    removed_tool_reference(&format!("Use {name} here")).is_some(),
                    "{name}"
                );
            } else {
                assert_eq!(
                    removed_tool_reference(&format!("You can {name} the page")),
                    None,
                    "{name}"
                );
            }
        }
        for phrase in ["snapshot and act", "from tabs or navigate", "tabs action="] {
            assert!(removed_tool_reference(phrase).is_some());
        }
        assert_eq!(
            removed_tool_reference(
                "Use page.screenshot() and page.ariaSnapshot() to inspect tabs."
            ),
            None
        );
    }

    #[test]
    fn exception_switch_controls_listing_and_cached_calls() {
        let names = [
            "run",
            REQUEST_HELP_TOOL_NAME,
            AWAIT_HELP_TOOL_NAME,
            PURSUE_TOOL_NAME,
            "snapshot",
        ];
        for exceptions in [false, true] {
            for pursue in [false, true] {
                let surface = Surface {
                    list_exceptions: exceptions,
                    human_help: HumanHelp::On,
                    pursue_offered: pursue,
                };
                let listed = names
                    .into_iter()
                    .filter(|name| surface.listed(name))
                    .collect::<Vec<_>>();
                let mut expected = vec!["run"];
                if exceptions {
                    expected.extend([REQUEST_HELP_TOOL_NAME, AWAIT_HELP_TOOL_NAME]);
                    if pursue {
                        expected.push(PURSUE_TOOL_NAME);
                    }
                }
                assert_eq!(listed, expected);
                assert!(
                    matches!(surface.admission("snapshot"), Admission::Refuse(message) if message.contains("run"))
                );
                assert_eq!(surface.admission(AWAIT_HELP_TOOL_NAME), Admission::Call);
                assert_eq!(
                    surface.admission(PURSUE_TOOL_NAME),
                    if exceptions {
                        Admission::Call
                    } else {
                        Admission::PursueHandback
                    }
                );
                assert_eq!(
                    surface.admission(REQUEST_HELP_TOOL_NAME),
                    if exceptions {
                        Admission::Call
                    } else {
                        Admission::Refuse(HELP_OFF_MESSAGE)
                    }
                );
            }
        }
    }
}
