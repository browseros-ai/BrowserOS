use std::sync::Arc;

use axum::{Router, middleware};
use rmcp::transport::streamable_http_server::session::never::NeverSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};

use super::request_hygiene::request_hygiene;
use super::system_routes::system_routes;
use crate::browser::BrowserLink;
use crate::config::Defaults;
use crate::mcp::McpServer;

/// Every route on the agent listener: `/mcp` behind the request hygiene, and `/system/*` for
/// Chromium's supervisor. Chromium's proxy forwards its public port here, so this router is
/// the whole surface an agent can reach.
///
/// `/mcp` is stateless: no `Mcp-Session-Id` on any protocol version (`legacy_session_mode:
/// false`, and a session manager that refuses sessions). Its SSE keep-alive is lowered to
/// `Defaults::SSE_KEEP_ALIVE`, because a gone client is noticed only on the next write, and
/// noticing it is what stops its run.
pub(crate) fn agent_router(mcp: McpServer, browser: Arc<BrowserLink>) -> Router {
    let config = StreamableHttpServerConfig::default()
        .with_legacy_session_mode(false)
        .with_sse_keep_alive(Some(Defaults::SSE_KEEP_ALIVE));
    let service = StreamableHttpService::new(
        move || Ok(mcp.clone()),
        Arc::new(NeverSessionManager::default()),
        config,
    );
    Router::new()
        .nest_service(
            "/mcp",
            Router::new()
                .fallback_service(service)
                .layer(middleware::from_fn(request_hygiene)),
        )
        .merge(system_routes(browser))
}
