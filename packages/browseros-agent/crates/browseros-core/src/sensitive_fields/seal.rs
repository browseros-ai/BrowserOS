//! The sealed isolated world: what Rust needs to install `seal.js` and read what it reports.
//!
//! ```text
//!  session attach ── Runtime.addBinding {name: JOURNAL_BINDING, executionContextName: SEALED_WORLD}
//!  first use in a document ── Page.createIsolatedWorld {worldName: SEALED_WORLD}
//!                          └─ Runtime.evaluate(seal_source()) in that context, before any engine
//!  typing in a sensitive field ── Runtime.bindingCalled {name: JOURNAL_BINDING, payload}
//!                                  ├─ JournalEntry::parse → sdk-traces' Privacy (secret set)
//!                                  └─ takeover's journal: JournalEntry::parse_with_trust →
//!                                     the secret set, as the owner's value only if trusted
//!  typing in an unwatched closed root ── Runtime.bindingCalled {payload: HIDDEN_ROOT_REPORT}
//!                                  └─ sdk-playwright: __clawSeal.takeHost(), scan, watch, sweep
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

/// The journal payload that says typing reached the seal at a shadow host whose root the world
/// cannot see: a closed root no one handed to `watch` yet. The seal holds the host until Rust
/// takes it with `__clawSeal.takeHost()`, so it can be scanned even after the page removed it.
/// It is no [`JournalEntry`], so a journal that reads only entries drops it.
pub const HIDDEN_ROOT_REPORT: &str = r#"{"hiddenRoot":true}"#;

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

    /// [`Self::parse`], and whether the owner entered the value. `seal.js` reports `trusted`
    /// when the browser dispatched the entry's event (`event.isTrusted`) and a trusted `keydown`,
    /// `paste` or `drop` reached the field since its last trusted entry, so each key, paste or
    /// drop makes at most one entry trusted. A page's script can dispatch none of those three
    /// trusted, and its `document.execCommand` edits fire a trusted `input` event but none of
    /// them (measured on Chromium 156), so its entries read untrusted.
    ///
    /// Anything but `"trusted": true`, a payload without the field included, reads as
    /// untrusted. The trust is read apart from the entry, so every entry `parse` keeps is kept
    /// here too, and an odd trust never drops a value the journal must mask.
    #[must_use]
    pub fn parse_with_trust(payload: &str) -> Option<(Self, bool)> {
        let entry = Self::parse(payload)?;
        let trusted = serde_json::from_str::<Trust>(payload).is_ok_and(|trust| trust.trusted);
        Some((entry, trusted))
    }
}

/// The trust half of a journal payload, for [`JournalEntry::parse_with_trust`].
#[derive(Deserialize)]
struct Trust {
    #[serde(default)]
    trusted: bool,
}

#[cfg(test)]
mod tests {
    use super::{
        FRAME_CROSSING_TOKENS, HIDDEN_ROOT_REPORT, JournalEntry, RULES_PLACEHOLDER, seal_source,
    };
    use crate::sensitive_fields::SensitiveKind;

    #[test]
    fn the_template_has_one_rules_placeholder() {
        assert_eq!(super::SEAL_TEMPLATE.matches(RULES_PLACEHOLDER).count(), 1);
    }

    #[test]
    fn the_seal_reports_a_hidden_root_as_rust_reads_it_and_never_as_an_entry() {
        assert_eq!(
            super::SEAL_TEMPLATE
                .matches(&format!("journal('{HIDDEN_ROOT_REPORT}')"))
                .count(),
            1,
            "seal.js sends the report as this exact payload"
        );
        assert!(
            JournalEntry::parse(HIDDEN_ROOT_REPORT).is_none(),
            "a journal that reads only entries, takeover's, drops it"
        );
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

    /// One entry with each `trusted` a payload may hold: `true` and `false`, as `seal.js` sends
    /// them, none, and a value that is no boolean.
    const BY_TRUST: [(&str, bool); 4] = [
        (
            r#"{"kind":"password","value":"hunter2pass","trusted":true}"#,
            true,
        ),
        (
            r#"{"kind":"password","value":"hunter2pass","trusted":false}"#,
            false,
        ),
        (r#"{"kind":"password","value":"hunter2pass"}"#, false),
        (
            r#"{"kind":"password","value":"hunter2pass","trusted":"true"}"#,
            false,
        ),
    ];

    #[test]
    fn the_seal_reports_each_entry_with_its_trust_as_rust_reads_it() {
        assert_eq!(
            super::SEAL_TEMPLATE
                .matches("var trusted = event.isTrusted && granted.has(el);")
                .count(),
            1,
            "seal.js decides trust as parse_with_trust's doc says"
        );
        assert_eq!(
            super::SEAL_TEMPLATE
                .matches("journal(raw.stringify({ kind: k, value: value, trusted: trusted }))")
                .count(),
            1,
            "seal.js sends each entry as this exact object"
        );
        assert!(
            JournalEntry::parse_with_trust(BY_TRUST[0].0).is_some_and(|(_, trusted)| trusted),
            "Rust reads the key seal.js sends"
        );
    }

    #[test]
    fn an_entry_is_trusted_only_when_its_payload_says_true() {
        for (payload, trusted) in BY_TRUST {
            let Some((entry, read)) = JournalEntry::parse_with_trust(payload) else {
                panic!("{payload} did not parse");
            };
            assert_eq!(entry.kind, SensitiveKind::Password, "{payload}");
            assert_eq!(entry.value.as_str(), "hunter2pass", "{payload}");
            assert_eq!(read, trusted, "{payload}");
        }
        for malformed in [
            r#"{"kind":"card","value":"","trusted":true}"#,
            r#"{"kind":"pin","value":"1234","trusted":true}"#,
            "not json",
        ] {
            assert!(
                JournalEntry::parse_with_trust(malformed).is_none(),
                "{malformed}"
            );
        }
    }

    /// sdk-playwright's journal reads entries with `parse`. The page's own scripts enter values
    /// there too, so dropping an untrusted entry would unmask its value.
    #[test]
    fn parse_keeps_an_entry_whatever_its_trust() {
        for (payload, _) in BY_TRUST {
            assert!(
                JournalEntry::parse(payload)
                    .is_some_and(|entry| entry.value.as_str() == "hunter2pass"),
                "{payload}"
            );
        }
    }
}
