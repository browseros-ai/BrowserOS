//! Side-panel association (TS effect `conversation-tabs`): a tab a conversation's tools
//! open joins that run's side-panel set, so the panel follows the agent into it.
//!
//! It rides the host's `TabJournal::claim_tab` seam rather than a dispatch effect. The host
//! claims a page the moment it is created, for `tabs new` and for every page a `run`
//! script opens mid-script; a dispatch effect would see only `tabs new`. That matches TS,
//! which published creation through a page proxy before load waits or the rest of a script.

use super::{leases::conversation_of_session, runs::RunRegistry};
use browseros_mcp_host::seams::{TabActivity, TabClaim, TabJournal};
use futures_util::future::BoxFuture;
use std::sync::Arc;

/// server-rust's tab journal: everything but creation claims is claw cockpit bookkeeping
/// this server has no surface for.
pub(super) struct PanelTabs(pub(super) Arc<dyn RunRegistry>);

impl TabJournal for PanelTabs {
    fn record_activity(&self, _activity: TabActivity) -> BoxFuture<'_, ()> {
        Box::pin(async {})
    }

    fn forget_incarnation<'a>(&'a self, _page_id: u32, _target_id: &'a str) -> BoxFuture<'a, ()> {
        Box::pin(async {})
    }

    fn claim_tab(&self, claim: TabClaim) {
        // Only lease sessions belong to a conversation; external MCP clients' tabs never
        // reach a side panel. Merely reading or navigating a user tab claims nothing.
        let Some(conversation_id) = conversation_of_session(&claim.session_id) else {
            return;
        };
        let Ok(tab_id) = u32::try_from(claim.tab_id) else {
            return;
        };
        if let Some(run) = self.0.active_run(conversation_id) {
            self.0.associate_tabs(&run, &[tab_id]);
        }
    }

    fn release_tab(&self, _tab_id: i64, _session_id: String) {}
}
