//! Complete catalog and normalization rules for BrowserClaw product analytics.
//!
//! Producers can select one of these opaque definitions, but cannot construct a
//! new wire event or widen its property schema. Free-form input is reduced to
//! fixed tokens here before the delivery service ever sees it.

use serde_json::{Map, Value};
use std::{
    collections::{HashMap, HashSet},
    sync::OnceLock,
};

const CLIENT_NAME: &str = "client_name";
const HARNESS: &str = "harness";
const KIND: &str = "kind";
const TOOL_NAME: &str = "tool_name";
const DISPATCH_COUNT: &str = "dispatch_count";
const DISTINCT_TOOL_COUNT: &str = "distinct_tool_count";
const MAX_CONCURRENT_USED_SESSIONS: &str = "max_concurrent_used_sessions";
const TOTAL_DURATION_MS: &str = "total_duration_ms";
const MAX_DURATION_MS: &str = "max_duration_ms";

pub(crate) const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;

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

pub(crate) const END_KIND_VALUES: [&str; 3] = ["closed", "errored", "cancelled"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PropertyKind {
    ClientName,
    Harness,
    EndKind,
    ToolName,
    UnsignedInteger,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PropertyDefinition {
    name: &'static str,
    kind: PropertyKind,
}

impl PropertyDefinition {
    const fn new(name: &'static str, kind: PropertyKind) -> Self {
        Self { name, kind }
    }

    fn normalize(self, value: &Value) -> Option<Value> {
        match self.kind {
            PropertyKind::ClientName => Some(Value::String(bucket_client_name(value.as_str()?))),
            PropertyKind::Harness => normalize_token(value, &HARNESS_VALUES),
            PropertyKind::EndKind => normalize_token(value, &END_KIND_VALUES),
            PropertyKind::ToolName => {
                let raw = value.as_str()?;
                known_tool_names()
                    .contains(raw)
                    .then(|| Value::String(raw.to_string()))
            }
            PropertyKind::UnsignedInteger => value
                .as_u64()
                .filter(|value| *value <= MAX_SAFE_INTEGER)
                .map(Value::from),
        }
    }
}

fn normalize_token(value: &Value, accepted: &[&str]) -> Option<Value> {
    let raw = value.as_str()?;
    accepted
        .contains(&raw)
        .then(|| Value::String(raw.to_string()))
}

fn known_tool_names() -> &'static HashSet<&'static str> {
    static KNOWN_TOOL_NAMES: OnceLock<HashSet<&'static str>> = OnceLock::new();
    KNOWN_TOOL_NAMES.get_or_init(|| {
        browseros_mcp::catalog()
            .into_iter()
            .map(|tool| tool.name)
            .chain(std::iter::once("name_session"))
            .collect()
    })
}

/// One catalog entry. Its private fields and constructor prevent producers from
/// inventing wire names or schemas while keeping call sites as small as constants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EventDefinition {
    name: &'static str,
    properties: &'static [PropertyDefinition],
}

impl EventDefinition {
    const fn new(name: &'static str, properties: &'static [PropertyDefinition]) -> Self {
        Self { name, properties }
    }

    #[must_use]
    pub const fn name(self) -> &'static str {
        self.name
    }

    #[must_use]
    pub fn property_names(self) -> Vec<&'static str> {
        self.properties
            .iter()
            .map(|property| property.name)
            .collect()
    }

    pub(crate) fn sanitize(self, properties: &Value) -> Option<Value> {
        let input = properties.as_object()?;
        let mut output = Map::new();
        for property in self.properties {
            let value = input.get(property.name)?;
            output.insert(property.name.to_string(), property.normalize(value)?);
        }
        Some(Value::Object(output))
    }

    pub(crate) fn required_values_are_normalized(
        self,
        properties: &HashMap<String, Value>,
    ) -> bool {
        self.properties.iter().all(|property| {
            let Some(current) = properties.get(property.name) else {
                return false;
            };
            property.normalize(current).as_ref() == Some(current)
        })
    }

    pub(crate) fn allows_property(self, key: &str) -> bool {
        self.properties.iter().any(|property| property.name == key)
    }
}

pub const SERVER_STARTED: EventDefinition = EventDefinition::new("server_started", &[]);
pub const AGENT_SESSION_STARTED: EventDefinition = EventDefinition::new(
    "agent_session_started",
    &[PropertyDefinition::new(
        CLIENT_NAME,
        PropertyKind::ClientName,
    )],
);
pub const AGENT_SESSION_ENDED: EventDefinition = EventDefinition::new(
    "agent_session_ended",
    &[
        PropertyDefinition::new(KIND, PropertyKind::EndKind),
        PropertyDefinition::new(CLIENT_NAME, PropertyKind::ClientName),
        PropertyDefinition::new(DISPATCH_COUNT, PropertyKind::UnsignedInteger),
        PropertyDefinition::new(DISTINCT_TOOL_COUNT, PropertyKind::UnsignedInteger),
        PropertyDefinition::new(MAX_CONCURRENT_USED_SESSIONS, PropertyKind::UnsignedInteger),
    ],
);
pub const HARNESS_CONNECTED: EventDefinition = EventDefinition::new(
    "harness_connected",
    &[PropertyDefinition::new(HARNESS, PropertyKind::Harness)],
);
pub const HARNESS_DISCONNECTED: EventDefinition = EventDefinition::new(
    "harness_disconnected",
    &[PropertyDefinition::new(HARNESS, PropertyKind::Harness)],
);
pub const AGENT_SESSION_TOOL_USAGE: EventDefinition = EventDefinition::new(
    "agent_session_tool_usage",
    &[
        PropertyDefinition::new(CLIENT_NAME, PropertyKind::ClientName),
        PropertyDefinition::new(TOOL_NAME, PropertyKind::ToolName),
        PropertyDefinition::new(DISPATCH_COUNT, PropertyKind::UnsignedInteger),
        PropertyDefinition::new(TOTAL_DURATION_MS, PropertyKind::UnsignedInteger),
        PropertyDefinition::new(MAX_DURATION_MS, PropertyKind::UnsignedInteger),
    ],
);

pub const ALL: [EventDefinition; 6] = [
    SERVER_STARTED,
    AGENT_SESSION_STARTED,
    AGENT_SESSION_ENDED,
    HARNESS_CONNECTED,
    HARNESS_DISCONNECTED,
    AGENT_SESSION_TOOL_USAGE,
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
        assert_eq!(ALL.len(), 6);
        assert_eq!(
            ALL.map(EventDefinition::name),
            [
                SERVER_STARTED.name(),
                AGENT_SESSION_STARTED.name(),
                AGENT_SESSION_ENDED.name(),
                HARNESS_CONNECTED.name(),
                HARNESS_DISCONNECTED.name(),
                AGENT_SESSION_TOOL_USAGE.name(),
            ]
        );
        assert_eq!(
            AGENT_SESSION_ENDED.property_names(),
            vec![
                "kind",
                "client_name",
                "dispatch_count",
                "distinct_tool_count",
                "max_concurrent_used_sessions",
            ]
        );
        assert_eq!(
            AGENT_SESSION_TOOL_USAGE.property_names(),
            vec![
                "client_name",
                "tool_name",
                "dispatch_count",
                "total_duration_ms",
                "max_duration_ms",
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
                AGENT_SESSION_ENDED.sanitize(&json!({
                    "kind": kind,
                    "client_name": "Codex",
                    "dispatch_count": 0,
                    "distinct_tool_count": 0,
                    "max_concurrent_used_sessions": 0,
                })),
                Some(json!({
                    "kind": kind,
                    "client_name": "codex",
                    "dispatch_count": 0,
                    "distinct_tool_count": 0,
                    "max_concurrent_used_sessions": 0,
                }))
            );
        }

        for invalid in [json!(null), json!(42), json!("custom")] {
            assert_eq!(
                HARNESS_CONNECTED.sanitize(&json!({ "harness": invalid })),
                None
            );
            assert_eq!(
                AGENT_SESSION_ENDED.sanitize(&json!({
                    "kind": invalid,
                    "client_name": "Codex",
                    "dispatch_count": 0,
                    "distinct_tool_count": 0,
                    "max_concurrent_used_sessions": 0,
                })),
                None
            );
        }
    }

    #[test]
    fn cancelled_session_end_kind_is_preserved() {
        assert_eq!(
            AGENT_SESSION_ENDED.sanitize(&json!({
                "kind": "cancelled",
                "client_name": "Codex",
                "dispatch_count": 1,
                "distinct_tool_count": 1,
                "max_concurrent_used_sessions": 1,
            })),
            Some(json!({
                "kind": "cancelled",
                "client_name": "codex",
                "dispatch_count": 1,
                "distinct_tool_count": 1,
                "max_concurrent_used_sessions": 1,
            }))
        );
    }

    #[test]
    fn session_usage_schemas_accept_only_known_tools_and_safe_aggregates() {
        assert_eq!(
            AGENT_SESSION_TOOL_USAGE.sanitize(&json!({
                "client_name": "Claude Code",
                "tool_name": "navigate",
                "dispatch_count": 3,
                "total_duration_ms": 810,
                "max_duration_ms": 420,
                "url": "https://private.example",
                "arguments": { "prompt": "private" },
                "result": "private",
            })),
            Some(json!({
                "client_name": "claude-code",
                "tool_name": "navigate",
                "dispatch_count": 3,
                "total_duration_ms": 810,
                "max_duration_ms": 420,
            }))
        );
        assert_eq!(
            AGENT_SESSION_TOOL_USAGE.sanitize(&json!({
                "client_name": "Codex",
                "tool_name": "name_session",
                "dispatch_count": 1,
                "total_duration_ms": 12,
                "max_duration_ms": 12,
            })),
            Some(json!({
                "client_name": "codex",
                "tool_name": "name_session",
                "dispatch_count": 1,
                "total_duration_ms": 12,
                "max_duration_ms": 12,
            }))
        );

        for tool_name in ["unknown", "https://private.example", "user@example.com"] {
            assert_eq!(
                AGENT_SESSION_TOOL_USAGE.sanitize(&json!({
                    "client_name": "Codex",
                    "tool_name": tool_name,
                    "dispatch_count": 1,
                    "total_duration_ms": 12,
                    "max_duration_ms": 12,
                })),
                None
            );
        }

        for invalid in [json!(-1), json!(1.5), json!(MAX_SAFE_INTEGER + 1)] {
            assert_eq!(
                AGENT_SESSION_TOOL_USAGE.sanitize(&json!({
                    "client_name": "Codex",
                    "tool_name": "navigate",
                    "dispatch_count": invalid,
                    "total_duration_ms": 12,
                    "max_duration_ms": 12,
                })),
                None
            );
        }
        assert_eq!(
            AGENT_SESSION_TOOL_USAGE.sanitize(&json!({
                "client_name": "Codex",
                "tool_name": "navigate",
                "dispatch_count": 1,
                "total_duration_ms": MAX_SAFE_INTEGER,
                "max_duration_ms": MAX_SAFE_INTEGER,
            })),
            Some(json!({
                "client_name": "codex",
                "tool_name": "navigate",
                "dispatch_count": 1,
                "total_duration_ms": MAX_SAFE_INTEGER,
                "max_duration_ms": MAX_SAFE_INTEGER,
            }))
        );
        assert_eq!(
            AGENT_SESSION_TOOL_USAGE.sanitize(&json!({
                "client_name": "Codex",
                "tool_name": "navigate",
                "dispatch_count": 1,
                "total_duration_ms": 12,
            })),
            None
        );
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
