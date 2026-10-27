//! The guides the CLI carries, so an agent can learn the API from the binary.
//!
//! Three layers, copied from agent-browser because the shape is proven: a tiny
//! installed skill that is always in context, this working guide pulled on
//! demand once a task actually touches a browser, and the complete reference
//! for the rare case that needs it. Connecting to neo over MCP instead loads
//! about 62 KB of tool schema into every session whether a browser is touched
//! or not; the CLI costs nothing until it is called.
//!
//! Embedded with `include_str!` rather than a crate, because two markdown
//! files do not justify a dependency and embedding is what makes
//! `skills get core` work with no network and no install step.

const CORE: &str = include_str!("skills/core.md");
const CORE_FULL: &str = include_str!("skills/core-full.md");

pub struct Guide {
    pub name: &'static str,
    pub summary: &'static str,
    pub body: &'static str,
    pub full: Option<&'static str>,
}

pub const GUIDES: &[Guide] = &[Guide {
    name: "core",
    summary: "The loop, refs, run, the daily commands and the exit codes.",
    body: CORE,
    full: Some(CORE_FULL),
}];

/// The guide by name, or nothing.
#[must_use]
pub fn get(name: &str, full: bool) -> Option<&'static str> {
    let guide = GUIDES.iter().find(|guide| guide.name == name)?;
    Some(match (full, guide.full) {
        (true, Some(body)) => body,
        _ => guide.body,
    })
}

/// What `skills list` prints.
#[must_use]
pub fn list() -> String {
    let mut lines = vec![
        "Guides carried by this binary. Read one with: browseros skills get <name>".to_string(),
        String::new(),
    ];
    for guide in GUIDES {
        lines.push(format!("  {:<8} {}", guide.name, guide.summary));
        if guide.full.is_some() {
            lines.push(format!(
                "  {:<8} --full adds the complete reference and the gaps versus agent-browser.",
                ""
            ));
        }
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_core_guide_is_there_and_is_not_empty() {
        let body = get("core", false).unwrap_or_default();
        assert!(body.len() > 1000, "{} bytes", body.len());
        assert!(body.contains("snapshot  ->  act"), "the loop must be in it");
    }

    #[test]
    fn full_returns_a_different_and_longer_document() {
        let short = get("core", false).unwrap_or_default();
        let long = get("core", true).unwrap_or_default();
        assert_ne!(short, long);
        assert!(long.len() > short.len());
        assert!(
            long.contains("Gaps versus agent-browser"),
            "the gaps belong in --full"
        );
    }

    #[test]
    fn the_short_guide_still_warns_about_the_cap_and_secrets() {
        let body = get("core", false).unwrap_or_default();
        assert!(body.contains("killed run is not a no-op"), "the cap hazard");
        assert!(body.contains("shell history"), "the secret hazard");
    }

    #[test]
    fn every_exit_code_is_documented_in_both_guides() {
        for full in [false, true] {
            let body = get("core", full).unwrap_or_default();
            for code in ["exit 0", "exit 5"].iter().map(|c| c.replace("exit ", "")) {
                assert!(body.contains(&code), "exit {code} missing (full={full})");
            }
        }
    }

    #[test]
    fn an_unknown_guide_is_not_invented() {
        assert_eq!(get("nope", false), None);
    }

    #[test]
    fn the_listing_names_every_guide() {
        let listing = list();
        for guide in GUIDES {
            assert!(listing.contains(guide.name), "{listing}");
        }
    }
}
