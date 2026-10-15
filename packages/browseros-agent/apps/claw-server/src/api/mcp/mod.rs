//! claw's `/mcp`: the shared host layer from `browseros_mcp_host`, branded "BrowserOS
//! neo" and wired to claw's audit timeline, cockpit, skills and human-help surfaces in
//! `host`. Session, ownership, grouping and dispatch logic all live in the crate.

pub mod distill;
pub mod host;
mod local_tools;
pub mod observers;
mod prompt;

#[cfg(test)]
mod pipeline_tests;
#[cfg(test)]
mod tab_journal_tests;
#[cfg(test)]
pub mod test_support;

use crate::AppState;
use browseros_mcp_host::mcp::session_arg_description;
use rmcp::transport::streamable_http_server::{
    session::local::LocalSessionManager,
    tower::{StreamableHttpServerConfig, StreamableHttpService},
};
use std::sync::LazyLock;

pub use browseros_mcp_host::HostMcpService as ClawMcpService;
// The shared layer's modules, at the paths claw's code and tests have always used.
pub use browseros_mcp_host::mcp::{dispatch, effects, guards, helper_runtime, naming, script_hook};

/// The `session` argument description claw serves. Re-exported so the route tests
/// assert against the real text instead of keeping a copy of it, which is what let the
/// two drift apart.
pub static SESSION_ARG_DESCRIPTION: LazyLock<String> =
    LazyLock::new(|| session_arg_description(host::NEO_SESSION_META_KEY));

/// Builds the shared MCP service used by both streamable HTTP and stdio.
#[must_use]
pub fn browser_mcp_service(state: AppState) -> ClawMcpService {
    browseros_mcp_host::mcp_service(state.mcp)
}

/// Builds the rmcp streamable HTTP service mounted at `/mcp`.
#[must_use]
pub fn streamable_http_service(
    state: AppState,
) -> StreamableHttpService<ClawMcpService, LocalSessionManager> {
    browseros_mcp_host::streamable_http_service(state.mcp, StreamableHttpServerConfig::default())
}
