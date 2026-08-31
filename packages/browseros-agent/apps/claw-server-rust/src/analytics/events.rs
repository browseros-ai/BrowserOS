//! Complete catalog and normalization rules for BrowserClaw product analytics.
//!
//! Producers can select one of these opaque definitions, but cannot construct a
//! new wire event or widen its property schema. Free-form input is reduced to
//! fixed tokens here before the delivery service ever sees it. The sole
//! exception is the agent-declared `task_summary`, a free-text property that is
//! PII-scrubbed and length-capped upstream and only bounded defensively here.

use serde_json::{Map, Value};
use std::{
    collections::{HashMap, HashSet},
    sync::OnceLock,
};

const CLIENT_NAME: &str = "client_name";
const TASK_CATEGORY: &str = "task_category";
const TASK_SUMMARY: &str = "task_summary";
const HARNESS: &str = "harness";
const KIND: &str = "kind";
const TOOL_NAME: &str = "tool_name";
const DISPATCH_COUNT: &str = "dispatch_count";
const DISTINCT_TOOL_COUNT: &str = "distinct_tool_count";
const MAX_CONCURRENT_USED_SESSIONS: &str = "max_concurrent_used_sessions";
const TOTAL_DURATION_MS: &str = "total_duration_ms";
const MAX_DURATION_MS: &str = "max_duration_ms";
const ACTIVE_DURATION_MS: &str = "active_duration_ms";
const TOOL_INPUT_TOKEN_ESTIMATE: &str = "tool_input_token_estimate";
const TOOL_OUTPUT_TOKEN_ESTIMATE: &str = "tool_output_token_estimate";
const BROWSERCLAW_TOKEN_ESTIMATE: &str = "browserclaw_token_estimate";
const SCREENSHOT_BASELINE_TOKEN_ESTIMATE: &str = "screenshot_baseline_token_estimate";
const SCREENSHOT_FIRST_TOKEN_ESTIMATE: &str = "screenshot_first_token_estimate";
const RAW_TOKEN_SAVINGS_ESTIMATE: &str = "raw_token_savings_estimate";
const EFFICIENCY_ESTIMATOR_VERSION: &str = "efficiency_estimator_version";
const SCREENSHOT_BASELINE_WIDTH: &str = "screenshot_baseline_width";
const SCREENSHOT_BASELINE_HEIGHT: &str = "screenshot_baseline_height";
const SCREENSHOT_TOKENS_PER_DISPATCH: &str = "screenshot_tokens_per_dispatch";

pub(crate) const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;

/// Hard cap on the free-text task summary before it leaves the machine. The
/// summary is already PII-scrubbed and length-capped upstream; this is the
/// final defensive bound at the analytics boundary.
pub(crate) const TASK_SUMMARY_MAX_CHARS: usize = 200;

const KNOWN_CLIENTS: [&str; 15] = [
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
    "browseros-cli",
];

const CLIENT_ALIASES: [(&str, &str); 4] = [
    ("codex-mcp-client", "codex"),
    ("codex-posthog-dashboard", "codex"),
    ("codex-browserclaw", "codex"),
    ("browserclaw-claude-desktop-wrapper", "claude-desktop"),
];

const UNRECOGNIZED_EMPTY: &str = "unrecognized-empty";

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

/// Fixed set of task kinds the agent may declare. Only these tokens leave the
/// machine; the free-form session name never does. An unrecognized value is
/// coerced to `other` rather than dropped, so the declaration still counts and a
/// hot `other` signals a missing row.
pub(crate) const TASK_CATEGORY_VALUES: [&str; 11] = [
    "shopping",
    "research",
    "email-and-messaging",
    "form-filling",
    "data-extraction",
    "testing-and-qa",
    "dev-tools",
    "social-media",
    "finance-and-admin",
    "internal-tools",
    "other",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PropertyKind {
    ClientName,
    TaskCategory,
    TaskSummary,
    Harness,
    EndKind,
    ToolName,
    UnsignedInteger,
    SignedInteger,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PropertyDefinition {
    name: &'static str,
    kind: PropertyKind,
    /// When true, the event still validates and sends if this property is
    /// absent; when present it must still normalize.
    optional: bool,
}

impl PropertyDefinition {
    const fn new(name: &'static str, kind: PropertyKind) -> Self {
        Self {
            name,
            kind,
            optional: false,
        }
    }

    const fn optional(name: &'static str, kind: PropertyKind) -> Self {
        Self {
            name,
            kind,
            optional: true,
        }
    }

    fn normalize(self, value: &Value) -> Option<Value> {
        match self.kind {
            PropertyKind::ClientName => Some(Value::String(bucket_client_name(value.as_str()?))),
            PropertyKind::TaskCategory => {
                let raw = value.as_str()?;
                let category = if TASK_CATEGORY_VALUES.contains(&raw) {
                    raw
                } else {
                    "other"
                };
                Some(Value::String(category.to_string()))
            }
            PropertyKind::TaskSummary => {
                // The one free-text property: the agent-authored summary is already
                // PII-scrubbed and length-capped upstream; here it is only bounded
                // defensively and passed through as-is.
                let raw = value.as_str()?;
                Some(Value::String(
                    raw.chars().take(TASK_SUMMARY_MAX_CHARS).collect(),
                ))
            }
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
            PropertyKind::SignedInteger => value
                .as_i64()
                .filter(|value| value.unsigned_abs() <= MAX_SAFE_INTEGER)
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
            let Some(value) = input.get(property.name) else {
                if property.optional {
                    continue;
                }
                return None;
            };
            let Some(normalized) = property.normalize(value) else {
                if property.optional {
                    continue;
                }
                return None;
            };
            output.insert(property.name.to_string(), normalized);
        }
        Some(Value::Object(output))
    }

    pub(crate) fn required_values_are_normalized(
        self,
        properties: &HashMap<String, Value>,
    ) -> bool {
        self.properties.iter().all(|property| {
            match properties.get(property.name) {
                Some(current) => property.normalize(current).as_ref() == Some(current),
                // Absent is acceptable only for optional properties.
                None => property.optional,
            }
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
pub const AGENT_SESSION_TASK_DECLARED: EventDefinition = EventDefinition::new(
    "agent_session_task_declared",
    &[
        PropertyDefinition::new(TASK_CATEGORY, PropertyKind::TaskCategory),
        PropertyDefinition::new(CLIENT_NAME, PropertyKind::ClientName),
        PropertyDefinition::optional(TASK_SUMMARY, PropertyKind::TaskSummary),
    ],
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
pub const AGENT_SESSION_EFFICIENCY_COMPUTED: EventDefinition = EventDefinition::new(
    "agent_session_efficiency_computed",
    &[
        PropertyDefinition::new(KIND, PropertyKind::EndKind),
        PropertyDefinition::new(CLIENT_NAME, PropertyKind::ClientName),
        PropertyDefinition::new(DISPATCH_COUNT, PropertyKind::UnsignedInteger),
        PropertyDefinition::new(ACTIVE_DURATION_MS, PropertyKind::UnsignedInteger),
        PropertyDefinition::new(TOOL_INPUT_TOKEN_ESTIMATE, PropertyKind::UnsignedInteger),
        PropertyDefinition::new(TOOL_OUTPUT_TOKEN_ESTIMATE, PropertyKind::UnsignedInteger),
        PropertyDefinition::new(BROWSERCLAW_TOKEN_ESTIMATE, PropertyKind::UnsignedInteger),
        PropertyDefinition::new(
            SCREENSHOT_BASELINE_TOKEN_ESTIMATE,
            PropertyKind::UnsignedInteger,
        ),
        PropertyDefinition::new(
            SCREENSHOT_FIRST_TOKEN_ESTIMATE,
            PropertyKind::UnsignedInteger,
        ),
        PropertyDefinition::new(RAW_TOKEN_SAVINGS_ESTIMATE, PropertyKind::SignedInteger),
        PropertyDefinition::new(EFFICIENCY_ESTIMATOR_VERSION, PropertyKind::UnsignedInteger),
        PropertyDefinition::new(SCREENSHOT_BASELINE_WIDTH, PropertyKind::UnsignedInteger),
        PropertyDefinition::new(SCREENSHOT_BASELINE_HEIGHT, PropertyKind::UnsignedInteger),
        PropertyDefinition::new(
            SCREENSHOT_TOKENS_PER_DISPATCH,
            PropertyKind::UnsignedInteger,
        ),
    ],
);

pub const ALL: [EventDefinition; 8] = [
    SERVER_STARTED,
    AGENT_SESSION_STARTED,
    AGENT_SESSION_TASK_DECLARED,
    AGENT_SESSION_ENDED,
    HARNESS_CONNECTED,
    HARNESS_DISCONNECTED,
    AGENT_SESSION_TOOL_USAGE,
    AGENT_SESSION_EFFICIENCY_COMPUTED,
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
    if let Some((_, canonical)) = CLIENT_ALIASES
        .iter()
        .find(|(alias, _)| *alias == slug.as_str())
    {
        return (*canonical).to_string();
    }

    if KNOWN_CLIENTS.contains(&slug.as_str()) {
        return slug;
    }

    // Not allowlisted: record the client's own slug so the long tail is visible
    // instead of collapsed into one opaque bucket. A blank name has nothing to
    // record, so it is reported as empty.
    if slug.is_empty() {
        UNRECOGNIZED_EMPTY.to_string()
    } else {
        slug
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn catalog_pins_wire_names_and_required_properties() {
        assert_eq!(ALL.len(), 8);
        assert_eq!(
            ALL.map(EventDefinition::name),
            [
                SERVER_STARTED.name(),
                AGENT_SESSION_STARTED.name(),
                AGENT_SESSION_TASK_DECLARED.name(),
                AGENT_SESSION_ENDED.name(),
                HARNESS_CONNECTED.name(),
                HARNESS_DISCONNECTED.name(),
                AGENT_SESSION_TOOL_USAGE.name(),
                AGENT_SESSION_EFFICIENCY_COMPUTED.name(),
            ]
        );
        assert_eq!(
            AGENT_SESSION_TASK_DECLARED.property_names(),
            vec!["task_category", "client_name", "task_summary"]
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
        assert_eq!(
            AGENT_SESSION_EFFICIENCY_COMPUTED.property_names(),
            vec![
                "kind",
                "client_name",
                "dispatch_count",
                "active_duration_ms",
                "tool_input_token_estimate",
                "tool_output_token_estimate",
                "browserclaw_token_estimate",
                "screenshot_baseline_token_estimate",
                "screenshot_first_token_estimate",
                "raw_token_savings_estimate",
                "efficiency_estimator_version",
                "screenshot_baseline_width",
                "screenshot_baseline_height",
                "screenshot_tokens_per_dispatch",
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
            "browseros-cli",
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
            "browseros-cli",
        ];

        for (raw, expected) in known.into_iter().zip(expected) {
            assert_eq!(
                AGENT_SESSION_STARTED.sanitize(&json!({ "client_name": raw })),
                Some(json!({ "client_name": expected }))
            );
        }
    }

    #[test]
    fn known_mcp_aliases_collapse_to_stable_client_buckets() {
        for raw in [
            "codex-mcp-client",
            "codex-posthog-dashboard",
            "Codex BrowserClaw",
        ] {
            assert_eq!(
                AGENT_SESSION_STARTED.sanitize(&json!({ "client_name": raw })),
                Some(json!({ "client_name": "codex" }))
            );
        }
        assert_eq!(
            AGENT_SESSION_STARTED
                .sanitize(&json!({ "client_name": "browserclaw-claude-desktop-wrapper" })),
            Some(json!({ "client_name": "claude-desktop" }))
        );
    }

    #[test]
    fn blank_client_names_report_as_unrecognized_empty() {
        for raw in ["", "   ", "!!!", "…"] {
            assert_eq!(
                AGENT_SESSION_STARTED.sanitize(&json!({ "client_name": raw })),
                Some(json!({ "client_name": "unrecognized-empty" })),
                "{raw:?} should bucket as empty"
            );
        }
    }

    #[test]
    fn known_task_categories_pass_through_and_unknown_coerces_to_other() {
        for category in TASK_CATEGORY_VALUES {
            assert_eq!(
                AGENT_SESSION_TASK_DECLARED
                    .sanitize(&json!({ "task_category": category, "client_name": "cursor" })),
                Some(json!({ "task_category": category, "client_name": "cursor" }))
            );
        }
        // Anything off-enum is coerced to `other` so the declaration still counts.
        for raw in ["crypto-trading", "", "SHOPPING", "shopping ", "acme corp"] {
            assert_eq!(
                AGENT_SESSION_TASK_DECLARED
                    .sanitize(&json!({ "task_category": raw, "client_name": "codex" })),
                Some(json!({ "task_category": "other", "client_name": "codex" }))
            );
        }
    }

    #[test]
    fn task_declared_drops_when_category_is_not_a_string() {
        assert_eq!(
            AGENT_SESSION_TASK_DECLARED
                .sanitize(&json!({ "task_category": 7, "client_name": "cursor" })),
            None
        );
    }

    #[test]
    fn task_declared_passes_through_the_free_text_summary() {
        assert_eq!(
            AGENT_SESSION_TASK_DECLARED.sanitize(&json!({
                "task_category": "shopping",
                "client_name": "cursor",
                "task_summary": "Compared warranty terms across three retailers.",
            })),
            Some(json!({
                "task_category": "shopping",
                "client_name": "cursor",
                "task_summary": "Compared warranty terms across three retailers.",
            }))
        );
    }

    #[test]
    fn task_declared_still_sends_when_the_optional_summary_is_absent() {
        assert_eq!(
            AGENT_SESSION_TASK_DECLARED
                .sanitize(&json!({ "task_category": "research", "client_name": "codex" })),
            Some(json!({ "task_category": "research", "client_name": "codex" }))
        );
    }

    #[test]
    fn task_summary_is_capped_at_the_boundary() {
        let long = "x".repeat(TASK_SUMMARY_MAX_CHARS + 50);
        let sanitized = AGENT_SESSION_TASK_DECLARED.sanitize(&json!({
            "task_category": "other",
            "client_name": "codex",
            "task_summary": long,
        }));
        let summary = sanitized
            .as_ref()
            .and_then(|value| value.get("task_summary"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        assert_eq!(summary.chars().count(), TASK_SUMMARY_MAX_CHARS);
    }

    #[test]
    fn unlisted_client_names_surface_their_slug() {
        for (raw, expected) in [
            ("Roo Code", "roo-code"),
            ("LibreChat", "librechat"),
            ("5ire", "5ire"),
            ("Cherry Studio", "cherry-studio"),
            ("claude-code-router", "claude-code-router"),
        ] {
            assert_eq!(
                AGENT_SESSION_STARTED.sanitize(&json!({ "client_name": raw })),
                Some(json!({ "client_name": expected })),
                "{raw:?} should surface its slug"
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
    fn session_efficiency_preserves_signed_savings_and_rejects_content() {
        let properties = json!({
            "kind": "errored",
            "client_name": "Claude Code",
            "dispatch_count": 2,
            "active_duration_ms": 420,
            "tool_input_token_estimate": 30,
            "tool_output_token_estimate": 7_000,
            "browserclaw_token_estimate": 7_030,
            "screenshot_baseline_token_estimate": 6_000,
            "screenshot_first_token_estimate": 6_030,
            "raw_token_savings_estimate": -1_000,
            "efficiency_estimator_version": 2,
            "screenshot_baseline_width": 1_920,
            "screenshot_baseline_height": 1_080,
            "screenshot_tokens_per_dispatch": 3_000,
            "session_id": "secret",
            "url": "https://private.example",
            "prompt": "private",
            "arguments": { "label": "private" },
            "result": "private",
            "screenshot": "/private/screenshot.png",
            "path": "/private/data",
            "email": "person@example.com",
        });

        assert_eq!(
            AGENT_SESSION_EFFICIENCY_COMPUTED.sanitize(&properties),
            Some(json!({
                "kind": "errored",
                "client_name": "claude-code",
                "dispatch_count": 2,
                "active_duration_ms": 420,
                "tool_input_token_estimate": 30,
                "tool_output_token_estimate": 7_000,
                "browserclaw_token_estimate": 7_030,
                "screenshot_baseline_token_estimate": 6_000,
                "screenshot_first_token_estimate": 6_030,
                "raw_token_savings_estimate": -1_000,
                "efficiency_estimator_version": 2,
                "screenshot_baseline_width": 1_920,
                "screenshot_baseline_height": 1_080,
                "screenshot_tokens_per_dispatch": 3_000,
            }))
        );

        for invalid in [
            json!(-(MAX_SAFE_INTEGER as i64) - 1),
            json!(MAX_SAFE_INTEGER + 1),
            json!(1.5),
            json!("-1"),
        ] {
            let mut invalid_properties = properties.clone();
            invalid_properties["raw_token_savings_estimate"] = invalid;
            assert_eq!(
                AGENT_SESSION_EFFICIENCY_COMPUTED.sanitize(&invalid_properties),
                None
            );
        }

        let mut negative_unsigned = properties.clone();
        negative_unsigned["active_duration_ms"] = json!(-1);
        assert_eq!(
            AGENT_SESSION_EFFICIENCY_COMPUTED.sanitize(&negative_unsigned),
            None
        );
        let mut missing = properties;
        assert!(
            missing
                .as_object_mut()
                .and_then(|properties| properties.remove("dispatch_count"))
                .is_some()
        );
        assert_eq!(AGENT_SESSION_EFFICIENCY_COMPUTED.sanitize(&missing), None);
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
