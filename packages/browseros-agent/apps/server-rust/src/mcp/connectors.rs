//! Managed connector tools (Klavis `connector_mcp_servers` + Strata proxies) on `/mcp`.
//! The host calls runtime providers outside its catalog pipeline, so the two endpoint-wide
//! guards are repeated here at that seam, as TS `authorizeCall` did
//! (`browser-mcp-module.ts:167-186`): a lease needs an active run, then read-only refuses.

use super::runs::{RunRegistry, run_refusal};
use browseros_mcp_host::{AdmittedCaller, LocalToolProvider, seams::LocalToolCall};
use futures_util::future::BoxFuture;
use rmcp::model::{CallToolResult, ContentBlock, Tool};
use std::sync::Arc;

pub const CONNECTORS_READ_ONLY: &str = "Managed connector tools are unavailable in read-only mode.";

/// Wraps the Klavis provider with server-rust's call policy. Listing is unchanged: TS
/// registered connector tools for every caller, read-only included, and refused at call.
pub(super) struct GatedConnectors {
    pub(super) inner: Arc<dyn LocalToolProvider>,
    pub(super) runs: Arc<dyn RunRegistry>,
}

impl LocalToolProvider for GatedConnectors {
    fn tools<'a>(&'a self, caller: Option<&'a AdmittedCaller>) -> BoxFuture<'a, Vec<Tool>> {
        self.inner.tools(caller)
    }

    fn call<'a>(&'a self, name: &'a str, call: LocalToolCall<'a>) -> BoxFuture<'a, CallToolResult> {
        if let Some(refusal) = run_refusal(self.runs.as_ref(), call.caller) {
            return Box::pin(async move { refusal.into_call_tool_result() });
        }
        if call.caller.is_some_and(|caller| caller.read_only) {
            return Box::pin(async {
                CallToolResult::error(vec![ContentBlock::text(CONNECTORS_READ_ONLY)])
            });
        }
        self.inner.call(name, call)
    }
}
