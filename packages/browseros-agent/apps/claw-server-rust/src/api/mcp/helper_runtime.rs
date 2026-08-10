//! Host side of code-mode helper self-healing: loads the helpers a script's
//! owned tabs make relevant so the runtime can expose them by name, and
//! surfaces what is available for discovery.

use crate::{
    AppState,
    api::mcp::dispatch::{ARBITRARY_SCRIPT_TOOLS, ToolEffect, ToolEffectContext},
    clock::now_epoch_ms,
    ids::ConvoId,
    services::helpers::{self, HelperMeta},
};
use browseros_core::BrowserSession;
use browseros_mcp::{ToolResult, framework::HelperSource};
use futures_util::future::BoxFuture;
use rmcp::model::ContentBlock;
use serde_json::{Value, json};
use std::collections::BTreeSet;

const MS_PER_DAY: i64 = 86_400_000;

/// Discovery/staleness view of a helper: name, soft age signal, whether it is a
/// distilled candidate, a description, and the exact copy-paste call form so the
/// agent can reuse it without guessing the signature. `ageDays` is null when
/// never stamped.
#[must_use]
pub(crate) fn helper_info_json(meta: &HelperMeta, now: i64) -> Value {
    let age_days = (meta.last_verified > 0).then(|| (now - meta.last_verified).max(0) / MS_PER_DAY);
    json!({
        "name": meta.name,
        "ageDays": age_days,
        "candidate": meta.candidate,
        "agent": meta.agent,
        "description": meta.description,
        "opensPage": meta.opens_page,
        "inputs": meta.inputs,
        "call": helpers::call_example(meta),
    })
}

/// Collects the helpers to hot-load for a script run: every saved helper for the
/// hosts of the agent's currently owned tabs. Cheap-gated so a browser with no
/// helpers pays nothing (no page scan).
pub(crate) async fn preload_helpers(
    state: &AppState,
    caller: &ConvoId,
    session: &BrowserSession,
) -> Vec<HelperSource> {
    let dir = &state.config.browserclaw_dir;
    if !helpers::has_any_helpers(dir) {
        return Vec::new();
    }
    let mut sources = Vec::new();
    for host in owned_tab_hosts(state, caller, session).await {
        for meta in helpers::list_helper_meta(dir, &host) {
            if let Some(source) = helpers::read_helper_source(dir, &host, &meta.name) {
                sources.push(HelperSource {
                    name: meta.name,
                    source,
                });
            }
        }
    }
    sources
}

/// The distinct host buckets of the tabs this agent owns.
pub(crate) async fn owned_tab_hosts(
    state: &AppState,
    caller: &ConvoId,
    session: &BrowserSession,
) -> BTreeSet<String> {
    let ownership = state.sessions.ownership();
    let mut hosts = BTreeSet::new();
    for page in session.pages.list().await.unwrap_or_default() {
        if ownership.owner_of_page(&page.page_id).await.as_ref() != Some(caller) {
            continue;
        }
        if let Some(host) = helpers::host_bucket(&page.url) {
            hosts.insert(host);
        }
    }
    hosts
}

/// Effect that appends `helpersAvailable` to a successful script run's result so
/// the agent notices the helpers its owned-tab hosts offer without having to
/// ask. Cheap-gated when no helpers exist.
pub fn discovery(
    context: ToolEffectContext<'_>,
) -> BoxFuture<'_, anyhow::Result<Option<ToolResult>>> {
    Box::pin(async move {
        if context.result.is_error
            || context.cancelled
            || !ARBITRARY_SCRIPT_TOOLS.contains(&context.call.tool().name)
        {
            return Ok(None);
        }
        let (Some(identity), Some(session)) =
            (&context.call.identity, &context.call.browser_session)
        else {
            return Ok(None);
        };
        let dir = &context.call.state.config.browserclaw_dir;
        if !helpers::has_any_helpers(dir) {
            return Ok(None);
        }
        let hosts = owned_tab_hosts(&context.call.state, &identity.ownership_key, session).await;
        let available = available_for_hosts(dir, &hosts, now_epoch_ms());
        if available.is_empty() {
            return Ok(None);
        }
        let mut result = context.result.clone();
        // Deliver discovery through a text block: structured content is not
        // serialized over the MCP wire (the script tool has no output schema),
        // so a helpersAvailable structured field alone never reaches the agent.
        result
            .content
            .push(ContentBlock::text(discovery_note(&available)));
        // Also keep the structured field for any client that does read it.
        result.structured_content = Some(match result.structured_content.take() {
            Some(Value::Object(mut map)) => {
                map.insert("helpersAvailable".to_string(), Value::Array(available));
                Value::Object(map)
            }
            _ => json!({ "helpersAvailable": available }),
        });
        Ok(Some(result))
    })
}

const _: ToolEffect = discovery;

/// A concise, agent-readable summary of the helpers available on the tabs' hosts:
/// per helper a freshness signal, a description, and the exact call form to copy.
#[must_use]
fn discovery_note(available: &[Value]) -> String {
    let mut lines = vec![
        "Reusable helpers for your tabs. Call the one you need with the form shown, or read its full doc with browser.readHelper(name, { host }):".to_string(),
    ];
    for entry in available {
        let host = entry
            .get("host")
            .and_then(Value::as_str)
            .unwrap_or_default();
        lines.push(format!("- {host}:"));
        if let Some(items) = entry.get("helpers").and_then(Value::as_array) {
            for helper in items {
                lines.push(format_helper_ref(helper));
            }
        }
    }
    lines.join("\n")
}

fn format_helper_ref(helper: &Value) -> String {
    let name = helper
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let age = helper
        .get("ageDays")
        .and_then(Value::as_i64)
        .map_or_else(|| "unstamped".to_string(), |days| format!("{days}d"));
    let candidate = if helper
        .get("candidate")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        ", candidate"
    } else {
        ""
    };
    let description = helper
        .get("description")
        .and_then(Value::as_str)
        .filter(|desc| !desc.is_empty())
        .map(|desc| format!(": {desc}"))
        .unwrap_or_default();
    let call = helper
        .get("call")
        .and_then(Value::as_str)
        .unwrap_or_default();
    format!("  - {name} ({age}{candidate}){description}\n    {call}")
}

/// Builds the `[{ host, helpers: [...] }]` discovery list for a set of hosts,
/// omitting hosts with no helpers.
#[must_use]
pub(crate) fn available_for_hosts(
    browserclaw_dir: &std::path::Path,
    hosts: &BTreeSet<String>,
    now: i64,
) -> Vec<Value> {
    let mut available = Vec::new();
    for host in hosts {
        let list: Vec<Value> = helpers::list_helper_meta(browserclaw_dir, host)
            .iter()
            .map(|meta| helper_info_json(meta, now))
            .collect();
        if !list.is_empty() {
            available.push(json!({ "host": host, "helpers": list }));
        }
    }
    available
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn meta(name: &str, host: &str, last_verified: i64, candidate: bool) -> HelperMeta {
        HelperMeta {
            name: name.to_string(),
            host: host.to_string(),
            last_verified,
            agent: "codex".to_string(),
            candidate,
            opens_page: true,
            inputs: std::collections::BTreeMap::from([(
                "field0".to_string(),
                "search query".to_string(),
            )]),
            description: format!("Opens {host} search for a query"),
            session: String::new(),
        }
    }

    #[test]
    fn helper_info_reports_a_soft_age_and_null_when_unstamped() {
        let now = 10 * MS_PER_DAY;
        let fresh = helper_info_json(&meta("a", "h", 8 * MS_PER_DAY, true), now);
        assert_eq!(fresh["ageDays"], json!(2));
        assert_eq!(fresh["candidate"], json!(true));
        let unstamped = helper_info_json(&meta("b", "h", 0, false), now);
        assert_eq!(unstamped["ageDays"], Value::Null);
    }

    #[test]
    fn discovery_note_renders_description_and_correct_call_form() {
        let now = MS_PER_DAY;
        let entry = helper_info_json(&meta("search-amazon", "amazon.in", MS_PER_DAY, true), now);
        let available = vec![json!({ "host": "amazon.in", "helpers": [entry] })];
        let note = discovery_note(&available);
        assert!(note.contains("browser.readHelper"));
        assert!(note.contains("- amazon.in:"));
        // Freshness, candidate flag, and the description ride the helper line.
        assert!(
            note.contains("- search-amazon (0d, candidate): Opens amazon.in search for a query"),
            "note: {note}"
        );
        // The exact, copy-paste call form: bracket access, inputs object, no page.
        assert!(
            note.contains("helpers[\"search-amazon\"](browser, { field0: \"<search query>\" })"),
            "note: {note}"
        );
        // The wrong dotted form is gone.
        assert!(!note.contains("helpers.<name>"));
    }

    #[test]
    fn available_for_hosts_lists_only_hosts_with_helpers() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let now = 5 * MS_PER_DAY;
        helpers::save_helper(
            dir.path(),
            &meta("greet", "example.com", 3 * MS_PER_DAY, true),
            "async () => 1",
        )?;
        let hosts = BTreeSet::from(["example.com".to_string(), "empty.com".to_string()]);
        let available = available_for_hosts(dir.path(), &hosts, now);
        assert_eq!(available.len(), 1);
        assert_eq!(available[0]["host"], json!("example.com"));
        assert_eq!(available[0]["helpers"][0]["name"], json!("greet"));
        assert_eq!(available[0]["helpers"][0]["ageDays"], json!(2));
        assert_eq!(available[0]["helpers"][0]["candidate"], json!(true));
        Ok(())
    }
}
