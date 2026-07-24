//! Host side of code-mode helper self-healing: loads the helpers a script's
//! owned tabs make relevant so the runtime can expose them by name, and
//! surfaces what is available for discovery.

use crate::{AppState, ids::ConvoId, services::helpers};
use browseros_core::BrowserSession;
use browseros_mcp::framework::HelperSource;
use std::collections::BTreeSet;

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
