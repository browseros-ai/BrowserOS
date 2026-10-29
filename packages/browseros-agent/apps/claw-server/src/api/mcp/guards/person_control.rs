//! Refuses page-changing calls on a page the person took over from the working overlay. Reading
//! stays allowed: the person is using the page, and looking at it disturbs nothing.

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
        call.state
            .page_hold
            .person_message(&PageId(page))
            .await
            .map(ToolResult::error)
    })
}

const _: ToolGuard = guard;
