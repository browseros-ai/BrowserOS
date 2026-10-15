//! The MCP surface: one rmcp server per transport session over a shared `HostCtx`, the
//! catalog dispatch pipeline, the built-in effects and guards, and the code-mode hooks.

pub mod dispatch;
pub mod effects;
pub mod guards;
pub mod helper_runtime;
mod local;
pub mod name_session;
pub mod naming;
pub mod script_hook;
mod service;
mod timeouts;

use crate::ctx::HostCtx;
use axum::Router;
use rmcp::transport::streamable_http_server::{
    session::local::LocalSessionManager,
    tower::{StreamableHttpServerConfig, StreamableHttpService},
};
use std::sync::Arc;

pub use service::{HostMcpService, session_arg_description};

/// One MCP server for one transport session (an HTTP session or stdio). Dropping it
/// schedules teardown of the legacy session it started, if any.
#[must_use]
pub fn mcp_service(host: Arc<HostCtx>) -> HostMcpService {
    HostMcpService::new(host)
}

/// The rmcp streamable-HTTP service: one `HostMcpService` per transport session.
#[must_use]
pub fn streamable_http_service(
    host: Arc<HostCtx>,
    config: StreamableHttpServerConfig,
) -> StreamableHttpService<HostMcpService, LocalSessionManager> {
    StreamableHttpService::new(
        move || Ok(mcp_service(host.clone())),
        Arc::new(LocalSessionManager::default()),
        config,
    )
}

/// How `mcp_router` serves `/mcp`.
#[derive(Clone, Default)]
pub struct McpRouteConfig {
    /// rmcp's defaults are claw's: stateful legacy sessions with SSE, a stateless
    /// 2026-07-28 path, and a loopback-only `Host` allowlist.
    pub http: StreamableHttpServerConfig,
}

/// Mounts the MCP endpoint at `/mcp`. Request hygiene (Origin and browser-fetch
/// rejection, JSON content type) is the host's policy; layer it on the returned router.
#[must_use]
pub fn mcp_router(host: Arc<HostCtx>, config: McpRouteConfig) -> Router {
    Router::new().nest_service("/mcp", streamable_http_service(host, config.http))
}
