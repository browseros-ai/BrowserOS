//! Owns desktop MCP discovery and admission. The internal catalog stays intact for
//! pursue and host effects, but cached MCP callers must pass this same policy.

use super::{
    hosted,
    service::{AWAIT_HELP_TOOL_NAME, PURSUE_TOOL_NAME, REQUEST_HELP_TOOL_NAME},
};
use crate::{config::ServerMode, services::agent_settings::HumanHelp};

/// Set false to expose only the SDK's run, including to cached exception callers.
pub(super) const LIST_EXCEPTIONS: bool = true;
const EXCEPTIONS: [&str; 3] = [
    PURSUE_TOOL_NAME,
    REQUEST_HELP_TOOL_NAME,
    AWAIT_HELP_TOOL_NAME,
];
/// Internal handlers remain usable by host dispatch. Retired local tools stay
/// compiled for hosted compatibility but have no desktop MCP entry point.
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
pub(super) enum Admission {
    Call,
    Refuse(&'static str),
    PursueHandback,
}

/// One matcher for host-authored desktop prose. Ordinary English and SDK method
/// names remain valid; only references to the retired MCP tools are forbidden.
#[cfg(test)]
pub(super) fn removed_tool_reference(text: &str) -> Option<&'static str> {
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
pub(super) struct Surface {
    pub(super) mode: ServerMode,
    pub(super) list_exceptions: bool,
    pub(super) human_help: HumanHelp,
    pub(super) pursue_offered: bool,
}

impl Surface {
    pub(super) fn listed(&self, name: &str) -> bool {
        if self.mode.is_hosted() {
            return hosted::refusal(self.mode, name).is_none();
        }
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

    /// Hosted keeps its catalog-wide schema lookup. Desktop never leaks internal
    /// schemas through get_tool, even when the caller remembers an old tool name.
    pub(super) fn lookup(&self, name: &str) -> bool {
        self.mode.is_hosted() || self.listed(name)
    }

    pub(super) fn admission(&self, name: &str) -> Admission {
        if self.mode.is_hosted() {
            return hosted::refusal(self.mode, name).map_or(Admission::Call, Admission::Refuse);
        }
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

#[cfg(test)]
mod tests {
    use super::*;

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
                    mode: ServerMode::Desktop,
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
