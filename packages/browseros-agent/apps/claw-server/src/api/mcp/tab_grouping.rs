//! What explicit tab grouping changes about the text an agent reads.
//!
//! Text written for automatic grouping promises the server's own tab groups in several
//! places: the instructions, the `tabs`, `tab_groups` and `name_session` descriptions, the
//! `session`, `taskName` and `groupId` argument descriptions, and the note about a page an
//! earlier session of the same client opened. Under [`TabGrouping::Explicit`] each promise is
//! rewritten here, matched by the exact span its owner defines next to the text it belongs
//! to. Automatic grouping gets every text back untouched, so desktop reads the same bytes.
//!
//! The grouping itself is switched off in `effects::tab_groups`; this module only keeps the
//! words in step with it. A rewrite that stops matching its text fails the sentinel test in
//! `service`, rather than leaving a promise the server no longer keeps.

use crate::{
    api::mcp::{effects::page_ownership_notice, hosted, prompt, service, session_handle},
    config::TabGrouping,
};
use browseros_mcp::tools::{tab_groups, tabs};
use rmcp::model::Tool;
use serde_json::Value;
use std::{borrow::Cow, sync::Arc};

/// Each promise of a server-made group, and what an agent reads instead under explicit
/// grouping.
pub(super) const EXPLICIT_REWRITES: [(&str, &str); 10] = [
    (
        prompt::RECONNECT_GROUP_GUIDANCE,
        "anything. After a reconnect your earlier tabs read as another agent's;\n  continue in them by page id instead of opening them again.",
    ),
    (prompt::CLOSE_GROUP_CLAUSE, "."),
    (prompt::GROUP_TITLE_CLAUSE, "."),
    (tabs::GROUP_RECOVERY_GUIDANCE, ""),
    (
        tabs::OWN_GROUP_FALLBACK,
        "; omit it to open the page ungrouped.",
    ),
    (tab_groups::RESUME_GUIDANCE, ""),
    (session_handle::SESSION_GROUP_CONTINUITY, ""),
    (service::NAME_SESSION_GROUPING, "The label"),
    (hosted::TASK_NAME_GROUPING, "."),
    (
        page_ownership_notice::RECLAIM_ADVICE,
        "If that was you, carry on in it.",
    ),
];

/// `text` as an agent should read it under `grouping`.
pub(super) fn text(text: &str, grouping: TabGrouping) -> Cow<'_, str> {
    let mut text = Cow::Borrowed(text);
    if grouping.is_automatic() {
        return text;
    }
    for (promise, explicit) in EXPLICIT_REWRITES {
        if text.contains(promise) {
            text = Cow::Owned(text.replace(promise, explicit));
        }
    }
    text
}

/// A listed tool as an agent should read it under `grouping`: its description and the
/// description of each argument. Names and schemas are untouched, because clients validate
/// their calls against them.
pub(super) fn tool(mut tool: Tool, grouping: TabGrouping) -> Tool {
    if grouping.is_automatic() {
        return tool;
    }
    if let Some(description) = &tool.description
        && let Cow::Owned(rewritten) = text(description, grouping)
    {
        tool.description = Some(rewritten.into());
    }
    let mut schema = tool.input_schema.as_ref().clone();
    if let Some(Value::Object(properties)) = schema.get_mut("properties") {
        for property in properties.values_mut() {
            if let Some(Value::String(description)) = property.get_mut("description")
                && let Cow::Owned(rewritten) = text(description, grouping)
            {
                *description = rewritten;
            }
        }
    }
    tool.input_schema = Arc::new(schema);
    tool
}
