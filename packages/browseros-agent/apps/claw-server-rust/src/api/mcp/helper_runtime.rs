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
use serde_json::{Value, json};
use std::collections::BTreeSet;

const MS_PER_DAY: i64 = 86_400_000;

/// Discovery/staleness view of a helper: name, soft age signal, and whether it
/// is a distilled candidate. `ageDays` is null when never stamped.
#[must_use]
pub(crate) fn helper_info_json(meta: &HelperMeta, now: i64) -> Value {
    let age_days = (meta.last_verified > 0).then(|| (now - meta.last_verified).max(0) / MS_PER_DAY);
    json!({
        "name": meta.name,
        "ageDays": age_days,
        "candidate": meta.candidate,
        "agent": meta.agent,
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
        // Append alongside the run's { ok, value, logs }; leave text content as is.
        let mut result = context.result.clone();
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
            deps: String::new(),
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
