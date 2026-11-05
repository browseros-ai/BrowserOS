//! The sealed isolated world: what Rust needs to install `seal.js` and read what it reports.
//!
//! ```text
//!  session attach ── Runtime.addBinding {name: JOURNAL_BINDING, executionContextName: SEALED_WORLD}
//!  first use in a document ── Page.createIsolatedWorld {worldName: SEALED_WORLD}
//!                          └─ Runtime.evaluate(seal_source()) in that context, before any engine
//!  typing in a sensitive field ── Runtime.bindingCalled {name: JOURNAL_BINDING, payload}
//!                                  └─ JournalEntry::parse → sdk-traces' Privacy (secret set)
//!  main-world evaluate ── Runtime.evaluate("__clawSeal.sensitiveValues()") before and after
//!                          └─ FieldValues::from_sweep → mask the evaluate's result
//! ```
//!
//! The world name is deliberately not `PageJs`'s `browseros`: core's own page scripts read raw
//! values to place the caret and verify fills (`prepare-fill.js`, `set-input-value.js`), and
//! sealing their world would break them. Nothing in core installs the seal; claw-sdk's engine
//! (sdk-playwright) does, so claw-server is unaffected.

use serde::Deserialize;
use std::fmt;
use std::sync::OnceLock;
use zeroize::Zeroizing;

use super::field_rules::{RULES_JSON, SensitiveKind};

/// The isolated world claw-sdk's engine and the seal share. Distinct from `PageJs`'s world.
pub const SEALED_WORLD: &str = "browseros-sdk";

/// The CDP binding the journal calls. Add it with `executionContextName = SEALED_WORLD` so it
/// exists only in the sealed world; the page can neither see nor call it.
pub const JOURNAL_BINDING: &str = "__neoJournal";

/// Code in the sealed world must not reach another frame's document through these: prototypes
/// are per frame, so it would read through getters that may not be sealed yet. `seal.js` nulls
/// `contentDocument` and `contentWindow`; the rest cannot be shadowed (`top` is unforgeable), so
/// the engine's sources are checked against this list instead (sdk-playwright's lint test).
pub const FRAME_CROSSING_TOKENS: &[&str] = &[
    "contentDocument",
    "contentWindow",
    "getSVGDocument",
    "window[",
    ".frames",
    "parent.",
    "top.",
    ".opener",
];

const SEAL_TEMPLATE: &str = include_str!("seal.js");
const RULES_PLACEHOLDER: &str = "__RULES__";

/// `seal.js` with the rule table injected: an expression that seals the world it runs in, once
/// per document (a second run returns the same version and changes nothing), and evaluates to
/// the seal's version.
#[must_use]
pub fn seal_source() -> &'static str {
    static SOURCE: OnceLock<String> = OnceLock::new();
    SOURCE.get_or_init(|| SEAL_TEMPLATE.replace(RULES_PLACEHOLDER, RULES_JSON.trim()))
}

/// One value the journal saw entered into a sensitive field. It has no `Serialize` or
/// `Display`, prints without its value under `Debug`, and is zeroed when dropped.
pub struct JournalEntry {
    /// What the field holds.
    pub kind: SensitiveKind,
    /// The value as the field holds it now.
    pub value: Zeroizing<String>,
}

impl fmt::Debug for JournalEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("JournalEntry")
            .field("kind", &self.kind)
            .field("value", &"[secret]")
            .finish()
    }
}

#[derive(Deserialize)]
struct Payload {
    kind: SensitiveKind,
    value: String,
}

impl JournalEntry {
    /// Parses a `Runtime.bindingCalled` payload for [`JOURNAL_BINDING`]. `None` for anything
    /// that is not a well-formed entry with a non-empty value.
    #[must_use]
    pub fn parse(payload: &str) -> Option<Self> {
        let Payload { kind, value } = serde_json::from_str(payload).ok()?;
        let value = Zeroizing::new(value);
        (!value.is_empty()).then_some(Self { kind, value })
    }
}

#[cfg(test)]
mod tests {
    use super::{FRAME_CROSSING_TOKENS, JournalEntry, RULES_PLACEHOLDER, seal_source};
    use crate::sensitive_fields::SensitiveKind;

    #[test]
    fn the_template_has_one_rules_placeholder() {
        assert_eq!(super::SEAL_TEMPLATE.matches(RULES_PLACEHOLDER).count(), 1);
    }

    #[test]
    fn the_source_carries_the_rule_table() {
        let source = seal_source();
        assert!(!source.contains(RULES_PLACEHOLDER));
        assert!(source.contains("\"cardTokenPrefixes\""));
        assert!(source.trim_end().ends_with("})"));
    }

    /// The seal itself must not cross frames, apart from the definitions that null them.
    #[test]
    fn the_seal_never_reaches_another_frame() {
        let code = seal_source()
            .lines()
            .filter(|line| !line.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        for token in FRAME_CROSSING_TOKENS
            .iter()
            .filter(|token| !["contentDocument", "contentWindow", "getSVGDocument"].contains(token))
        {
            assert!(!code.contains(token), "seal.js uses {token}");
        }
    }

    #[test]
    fn journal_payloads_parse_and_hide_their_value() {
        let entry = JournalEntry::parse(r#"{"kind":"one_time_code","value":"482913"}"#);
        let Some(entry) = entry else {
            panic!("payload did not parse");
        };
        assert_eq!(entry.kind, SensitiveKind::OneTimeCode);
        assert_eq!(entry.value.as_str(), "482913");
        assert!(!format!("{entry:?}").contains("482913"));
        assert!(JournalEntry::parse(r#"{"kind":"card","value":""}"#).is_none());
        assert!(JournalEntry::parse(r#"{"kind":"pin","value":"1234"}"#).is_none());
        assert!(JournalEntry::parse("not json").is_none());
    }
}
