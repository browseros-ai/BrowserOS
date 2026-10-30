//! Refuses an agent's first page-changing call on a page the person took over from it, so the
//! agent stops and checks with the user instead of taking the page straight back. Reading stays
//! allowed: looking at a page disturbs nothing.

use crate::api::mcp::dispatch::{ToolCall, ToolGuard, extract_page_id};
use browseros_core::PageId;
use browseros_mcp::ToolResult;
use futures_util::future::BoxFuture;

const READING_TOOLS: [&str; 8] = [
    "snapshot",
    "diff",
    "read",
    "grep",
    "screenshot",
    "pdf",
    "wait",
    "history",
];

pub fn guard(call: &ToolCall) -> BoxFuture<'_, Option<ToolResult>> {
    Box::pin(async move {
        if READING_TOOLS.contains(&call.tool().name) {
            return None;
        }
        let page = extract_page_id(call)?;
        let identity = call.identity.as_ref()?;
        call.state
            .page_hold
            .take_notice(&PageId(page), identity.session.id())
            .await
            .map(ToolResult::error)
    })
}

const _: ToolGuard = guard;
