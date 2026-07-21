//! Complete catalog and normalization rules for BrowserClaw product analytics.
//!
//! Producers can select one of these opaque definitions, but cannot construct a
//! new wire event or widen its property schema. Free-form input is reduced to
//! fixed tokens here before the delivery service ever sees it.

use serde_json::{Map, Value};
use std::collections::HashMap;

const CLIENT_NAME: &str = "client_name";
const HARNESS: &str = "harness";
const KIND: &str = "kind";

const KNOWN_CLIENTS: [&str; 14] = [
    "claude-desktop",
    "claude-code",
    "claude-ai",
    "cursor",
    "vscode",
    "vscode-insiders",
    "codex",
    "zed",
    "opencode",
    "antigravity",
    "windsurf",
    "cline",
    "continue",
    "goose",
];

pub(crate) const HARNESS_VALUES: [&str; 7] = [
    "Claude Code",
    "Codex",
    "Cursor",
    "OpenCode",
    "Antigravity",
    "VS Code",
    "Zed",
];

pub(crate) const END_KIND_VALUES: [&str; 2] = ["closed", "errored"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RequiredProperty {
    ClientName,
    Harness,
    Kind,
}

impl RequiredProperty {
    const fn name(self) -> &'static str {
        match self {
            Self::ClientName => CLIENT_NAME,
            Self::Harness => HARNESS,
            Self::Kind => KIND,
        }
    }

    fn normalize(self, value: &Value) -> Option<Value> {
        let raw = value.as_str()?;
        match self {
            Self::ClientName => Some(Value::String(bucket_client_name(raw))),
            Self::Harness if HARNESS_VALUES.contains(&raw) => Some(Value::String(raw.to_string())),
            Self::Kind if END_KIND_VALUES.contains(&raw) => Some(Value::String(raw.to_string())),
            Self::Harness | Self::Kind => None,
        }
    }
}

/// One catalog entry. Its private fields and constructor prevent producers from
/// inventing wire names or schemas while keeping call sites as small as constants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EventDefinition {
    name: &'static str,
    required: &'static [RequiredProperty],
}

impl EventDefinition {
    const fn new(name: &'static str, required: &'static [RequiredProperty]) -> Self {
        Self { name, required }
    }

    #[must_use]
    pub const fn name(self) -> &'static str {
        self.name
    }

    #[must_use]
    pub fn required_property_names(self) -> &'static [&'static str] {
        match self.required {
            [] => &[],
            [RequiredProperty::ClientName] => &[CLIENT_NAME],
            [RequiredProperty::Harness] => &[HARNESS],
            [RequiredProperty::Kind] => &[KIND],
            _ => &[],
        }
    }

    pub(crate) fn sanitize(self, properties: &Value) -> Option<Value> {
        let input = properties.as_object()?;
        let mut output = Map::new();
        for required in self.required {
            let value = input.get(required.name())?;
            output.insert(required.name().to_string(), required.normalize(value)?);
        }
        Some(Value::Object(output))
    }

    pub(crate) fn required_values_are_normalized(
        self,
        properties: &HashMap<String, Value>,
    ) -> bool {
        self.required.iter().all(|required| {
            let Some(current) = properties.get(required.name()) else {
                return false;
            };
            required.normalize(current).as_ref() == Some(current)
        })
    }

    pub(crate) fn allows_property(self, key: &str) -> bool {
        self.required.iter().any(|required| required.name() == key)
    }
}

pub const SERVER_STARTED: EventDefinition = EventDefinition::new("server_started", &[]);
pub const AGENT_SESSION_STARTED: EventDefinition =
    EventDefinition::new("agent_session_started", &[RequiredProperty::ClientName]);
pub const AGENT_SESSION_ENDED: EventDefinition =
    EventDefinition::new("agent_session_ended", &[RequiredProperty::Kind]);
pub const HARNESS_CONNECTED: EventDefinition =
    EventDefinition::new("harness_connected", &[RequiredProperty::Harness]);
pub const HARNESS_DISCONNECTED: EventDefinition =
    EventDefinition::new("harness_disconnected", &[RequiredProperty::Harness]);

pub const ALL: [EventDefinition; 5] = [
    SERVER_STARTED,
    AGENT_SESSION_STARTED,
    AGENT_SESSION_ENDED,
    HARNESS_CONNECTED,
    HARNESS_DISCONNECTED,
];

pub(crate) fn by_wire_name(name: &str) -> Option<EventDefinition> {
    ALL.into_iter().find(|definition| definition.name == name)
}

#[must_use]
pub(crate) fn platform_token_for(target_os: &str) -> &str {
    match target_os {
        "macos" => "darwin",
        "windows" => "win32",
        other => other,
    }
}

#[must_use]
pub(crate) fn platform_token() -> &'static str {
    platform_token_for(std::env::consts::OS)
}

fn bucket_client_name(raw: &str) -> String {
    let mut slug = String::with_capacity(raw.len());
    let mut separator_pending = false;
    for character in raw.chars() {
        if character.is_ascii_alphanumeric() {
            if separator_pending && !slug.is_empty() {
                slug.push('-');
            }
            slug.push(character.to_ascii_lowercase());
            separator_pending = false;
        } else {
            separator_pending = true;
        }
    }
    if KNOWN_CLIENTS.contains(&slug.as_str()) {
        slug
    } else {
        "other".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn catalog_pins_wire_names_and_required_properties() {
        assert_eq!(
            ALL.map(|definition| (definition.name(), definition.required_property_names())),
            [
                (SERVER_STARTED.name(), &[][..]),
                (AGENT_SESSION_STARTED.name(), &["client_name"][..]),
                (AGENT_SESSION_ENDED.name(), &["kind"][..]),
                (HARNESS_CONNECTED.name(), &["harness"][..]),
                (HARNESS_DISCONNECTED.name(), &["harness"][..]),
            ]
        );
    }

    #[test]
    fn client_names_are_slugged_into_the_archived_buckets() {
        let known = [
            "Claude Desktop",
            "Claude Code",
            "Claude AI",
            "Cursor",
            "VSCode",
            "VSCode Insiders",
            "Codex",
            "Zed",
            "OpenCode",
            "Antigravity",
            "Windsurf",
            "Cline",
            "Continue",
            "Goose",
        ];
        let expected = [
            "claude-desktop",
            "claude-code",
            "claude-ai",
            "cursor",
            "vscode",
            "vscode-insiders",
            "codex",
            "zed",
            "opencode",
            "antigravity",
            "windsurf",
            "cline",
            "continue",
            "goose",
        ];

        for (raw, expected) in known.into_iter().zip(expected) {
            assert_eq!(
                AGENT_SESSION_STARTED.sanitize(&json!({ "client_name": raw })),
                Some(json!({ "client_name": expected }))
            );
        }
    }

    #[test]
    fn unknown_or_content_shaped_client_names_become_other() {
        for raw in [
            "",
            "my-secret-internal-tool",
            "https://example.com",
            "user@example.com",
            "/home/user/secret",
            r"C:\Users\someone",
        ] {
            assert_eq!(
                AGENT_SESSION_STARTED.sanitize(&json!({ "client_name": raw })),
                Some(json!({ "client_name": "other" }))
            );
        }
    }

    #[test]
    fn harness_and_end_kind_are_closed_token_sets() {
        for harness in HARNESS_VALUES {
            assert_eq!(
                HARNESS_CONNECTED.sanitize(&json!({ "harness": harness })),
                Some(json!({ "harness": harness }))
            );
        }
        for kind in END_KIND_VALUES {
            assert_eq!(
                AGENT_SESSION_ENDED.sanitize(&json!({ "kind": kind })),
                Some(json!({ "kind": kind }))
            );
        }

        for invalid in [json!(null), json!(42), json!("custom")] {
            assert_eq!(
                HARNESS_CONNECTED.sanitize(&json!({ "harness": invalid })),
                None
            );
            assert_eq!(
                AGENT_SESSION_ENDED.sanitize(&json!({ "kind": invalid })),
                None
            );
        }
    }

    #[test]
    fn missing_required_properties_reject_and_extra_properties_never_survive() {
        assert_eq!(AGENT_SESSION_STARTED.sanitize(&json!({})), None);
        assert_eq!(HARNESS_CONNECTED.sanitize(&json!(null)), None);
        assert_eq!(
            HARNESS_DISCONNECTED.sanitize(&json!({
                "harness": "Zed",
                "url": "https://example.com",
                "path": "/private/data",
                "email": "person@example.com",
                "session_id": "secret",
                "nested": { "prompt": "private" }
            })),
            Some(json!({ "harness": "Zed" }))
        );
    }

    #[test]
    fn platform_tokens_match_historical_node_values() {
        assert_eq!(platform_token_for("macos"), "darwin");
        assert_eq!(platform_token_for("windows"), "win32");
        assert_eq!(platform_token_for("linux"), "linux");
        assert_eq!(platform_token_for("freebsd"), "freebsd");
    }
}
