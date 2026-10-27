//! What this particular server offers, and what neo does not do at all.
//!
//! The tool surface is conditional in five independent ways. `setViewport`
//! appears only when hosted; `pursue` only when the server holds a
//! decision-model credential; `solve_captcha` only with a provider
//! configured; the human-help pair only with the setting on; and
//! `browser_target`, `browser_profiles`, `credentials` and `payment_cards`
//! only on a native that implements the matching feature. A static command
//! table would promise commands this server refuses, so the CLI asks once and
//! says what it found.

/// An optional tool, with the reason it is usually missing.
///
/// The reason is the whole value: "not offered" sends an agent looking for a
/// flag, while "no decision-model credential on the server" sends it to the
/// settings page or to `run`.
pub struct Optional {
    pub command: &'static str,
    pub tool: &'static str,
    pub absent_because: &'static str,
}

pub const OPTIONAL: &[Optional] = &[
    Optional {
        command: "pursue",
        tool: "pursue",
        absent_because: "the server has no decision-model credential",
    },
    Optional {
        command: "help",
        tool: "request_human_help",
        absent_because: "human help is turned off in BrowserOS neo settings",
    },
    Optional {
        command: "profile",
        tool: "browser_profiles",
        absent_because: "this BrowserOS build does not implement profile targeting",
    },
    Optional {
        command: "target",
        tool: "browser_target",
        absent_because: "this BrowserOS build does not implement profile targeting",
    },
    Optional {
        command: "skill",
        tool: "save_skill",
        absent_because: "a hosted server cannot link skills into an agent beside it",
    },
    Optional {
        command: "credentials",
        tool: "credentials",
        absent_because: "this BrowserOS build does not expose its password store",
    },
    Optional {
        command: "cards",
        tool: "payment_cards",
        absent_because: "this BrowserOS build does not expose its payment cards",
    },
];

/// Why a command the CLI knows is not available on this server, if so.
#[must_use]
pub fn unavailable(tool: &str, offered: &[String]) -> Option<&'static str> {
    if offered.is_empty() {
        // Nothing has been asked yet, which must read as offered rather than
        // as absent: refusing a call on an empty cache would break the first
        // command of every session.
        return None;
    }
    if offered.iter().any(|name| name == tool) {
        return None;
    }
    OPTIONAL
        .iter()
        .find(|entry| entry.tool == tool)
        .map(|entry| entry.absent_because)
}

/// A capability agent-browser has that neo does not, and the nearest thing here.
pub struct Gap {
    pub command: &'static str,
    pub missing: &'static str,
    pub nearest: Option<&'static str>,
}

/// The fixed list, so an unknown-but-expected command is a teaching moment
/// rather than a usage error. An agent that assumes one of these exists
/// otherwise wastes turns discovering it does not.
pub const GAPS: &[Gap] = &[
    Gap {
        command: "cookies",
        missing: "neo has no cookie tool.",
        nearest: Some("browseros run \"return await page.cdp('Network.getCookies')\""),
    },
    Gap {
        command: "storage",
        missing: "neo has no storage tool.",
        nearest: Some("browseros eval \"return JSON.stringify(localStorage)\""),
    },
    Gap {
        command: "state",
        missing: "neo has no saved auth state: it drives the user's real profile, already signed in.",
        nearest: Some("browseros target set \"<profile name>\""),
    },
    Gap {
        command: "auth",
        missing: "neo has no auth vault. A human signs in, in the real browser.",
        nearest: Some("browseros help request \"sign in to <site>\" --kind login"),
    },
    Gap {
        command: "network",
        missing: "neo has no network interception, HAR capture or request inspection.",
        nearest: Some(
            "call the site's own API from inside: browseros eval \"return await (await fetch('/api')).json()\"",
        ),
    },
    Gap {
        command: "route",
        missing: "neo has no request routing or aborting.",
        nearest: None,
    },
    Gap {
        command: "device",
        missing: "neo has no device emulation on desktop; setViewport exists only on a hosted server.",
        nearest: None,
    },
    Gap {
        command: "viewport",
        missing: "setViewport is offered only by a hosted server, where the windows are infrastructure.",
        nearest: None,
    },
    Gap {
        command: "geolocation",
        missing: "neo has no geolocation override.",
        nearest: None,
    },
    Gap {
        command: "locator",
        missing: "there are no locators or selectors for acting. Elements are addressed by snapshot ref.",
        nearest: Some("browseros snapshot -i, then browseros click @eN"),
    },
    Gap {
        command: "get",
        missing: "there is no get html|value|attr|box|styles.",
        nearest: Some("browseros eval \"return document.querySelector('#x').value\""),
    },
    Gap {
        command: "record",
        missing: "neo has no video or trace recording from the CLI.",
        nearest: Some("the cockpit records sessions already"),
    },
    Gap {
        command: "profiler",
        missing: "neo has no profiler.",
        nearest: None,
    },
    Gap {
        command: "frame",
        missing: "frames are not addressable. Iframe content is stitched into the snapshot inline.",
        nearest: Some("browseros snapshot, and use the refs inside the iframe directly"),
    },
    Gap {
        command: "clipboard",
        missing: "neo has no clipboard tool.",
        nearest: Some("browseros eval \"return await navigator.clipboard.readText()\""),
    },
    Gap {
        command: "highlight",
        missing: "neo has no highlight tool.",
        nearest: Some("browseros screenshot --annotate"),
    },
    Gap {
        command: "batch",
        missing: "there is no static command list, because run is strictly better.",
        nearest: Some("browseros run - <<'JS' … JS"),
    },
];

/// The message for a command that looks like an agent-browser habit.
#[must_use]
pub fn teach(command: &str) -> Option<String> {
    let gap = GAPS.iter().find(|gap| gap.command == command)?;
    let mut message = format!("no {command} command. {}", gap.missing);
    if let Some(nearest) = gap.nearest {
        message.push_str(&format!("\n  Closest: {nearest}"));
    }
    message.push_str(
        "\n  See: browseros skills get core --full  (section: gaps versus agent-browser)",
    );
    Some(message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_cache_never_blocks_a_call() {
        assert_eq!(unavailable("pursue", &[]), None);
    }

    #[test]
    fn a_withheld_optional_tool_is_explained_rather_than_attempted() {
        let offered = vec!["tabs".to_string(), "run".to_string()];
        let reason = unavailable("pursue", &offered).unwrap_or_default();
        assert!(reason.contains("decision-model credential"), "{reason}");
    }

    #[test]
    fn a_tool_this_server_lists_is_simply_available() {
        let offered = vec!["pursue".to_string()];
        assert_eq!(unavailable("pursue", &offered), None);
    }

    #[test]
    fn a_tool_with_no_optional_entry_is_not_invented() {
        let offered = vec!["tabs".to_string()];
        assert_eq!(unavailable("navigate", &offered), None);
    }

    #[test]
    fn an_agent_browser_habit_gets_the_nearest_neo_command() {
        let message = teach("cookies").unwrap_or_default();
        assert!(message.contains("Network.getCookies"), "{message}");
        assert!(message.contains("skills get core --full"), "{message}");
    }

    #[test]
    fn a_gap_with_no_workaround_still_says_so_plainly() {
        let message = teach("profiler").unwrap_or_default();
        assert!(message.contains("no profiler"), "{message}");
        assert!(!message.contains("Closest:"), "{message}");
    }

    #[test]
    fn an_unrelated_typo_is_left_to_clap() {
        assert_eq!(teach("clickk"), None);
    }
}
