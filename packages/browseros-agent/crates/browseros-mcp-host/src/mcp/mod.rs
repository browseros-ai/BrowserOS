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

use crate::{admission::Admission, ctx::HostCtx};
use axum::{
    Router,
    extract::Request,
    middleware::{self, Next},
    response::Response,
};
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
    /// 2026-07-28 path, a loopback-only `Host` allowlist, and no `Origin` check.
    pub http: StreamableHttpServerConfig,
    /// Resolves each request's caller before rmcp parses it; `None` admits every request
    /// as an anonymous client, which is claw's model.
    pub admission: Option<Arc<dyn Admission>>,
}

impl McpRouteConfig {
    /// The TS BrowserOS server's transport: every request stateless, single-JSON replies
    /// for 2025-era clients (its internal ACP client depends on them), any `Host` (the
    /// server is reachable on the LAN), and no `Origin` validation.
    #[must_use]
    pub fn stateless_json() -> Self {
        let mut http = StreamableHttpServerConfig::default().disable_allowed_hosts();
        http.legacy_session_mode = false;
        http.json_response = true;
        http.allowed_origins = Vec::new();
        Self {
            http,
            admission: None,
        }
    }

    #[must_use]
    pub fn with_admission(mut self, admission: Arc<dyn Admission>) -> Self {
        self.admission = Some(admission);
        self
    }
}

/// Mounts the MCP endpoint at `/mcp`. Request hygiene (Origin and browser-fetch
/// rejection, JSON content type) is the host's policy; layer it on the returned router.
pub fn mcp_router(host: Arc<HostCtx>, config: McpRouteConfig) -> Router {
    let router = Router::new().nest_service("/mcp", streamable_http_service(host, config.http));
    match config.admission {
        Some(admission) => with_admission(router, admission),
        None => router,
    }
}

/// Runs `admission` on every request reaching `router` and attaches the admitted caller
/// to the request's extensions, where the MCP service reads it. For hosts that mount
/// `streamable_http_service` behind their own routing instead of `mcp_router`.
pub fn with_admission(router: Router, admission: Arc<dyn Admission>) -> Router {
    router.layer(middleware::from_fn(move |request: Request, next: Next| {
        let admission = admission.clone();
        async move { admit(admission.as_ref(), request, next).await }
    }))
}

async fn admit(admission: &dyn Admission, request: Request, next: Next) -> Response {
    let (mut parts, body) = request.into_parts();
    match admission.admit(&parts) {
        Ok(caller) => {
            if let Some(caller) = caller {
                parts.extensions.insert(caller);
            }
            next.run(Request::from_parts(parts, body)).await
        }
        Err(response) => *response,
    }
}
