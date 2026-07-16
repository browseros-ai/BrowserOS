pub mod dispatch;
pub mod effects;
pub mod guards;
pub mod naming;
mod prompt;
mod service;
mod timeouts;

#[cfg(test)]
pub mod test_support;

use crate::AppState;
use rmcp::transport::streamable_http_server::{
    session::local::LocalSessionManager,
    tower::{StreamableHttpServerConfig, StreamableHttpService},
};
use std::sync::Arc;

pub use service::ClawMcpService;

/// Builds the shared MCP service used by both streamable HTTP and stdio.
#[must_use]
pub fn browser_mcp_service(state: AppState) -> ClawMcpService {
    let browser = Arc::downgrade(&state.browser);
    state
        .sessions
        .set_last_session_teardown_hook(Arc::new(move |ownership, key| {
            let browser = browser.clone();
            Box::pin(async move {
                let Some(browser) = browser.upgrade() else {
                    return;
                };
                let Some(session) = browser.session().await else {
                    return;
                };
                effects::tab_groups::collapse_agent_tab_group(&session, &ownership, &key).await;
            })
        }));
    ClawMcpService::new(state)
}

/// Builds the rmcp streamable HTTP service mounted at `/mcp`.
#[must_use]
pub fn streamable_http_service(
    state: AppState,
) -> StreamableHttpService<ClawMcpService, LocalSessionManager> {
    StreamableHttpService::new(
        move || Ok(browser_mcp_service(state.clone())),
        Arc::new(LocalSessionManager::default()),
        StreamableHttpServerConfig::default(),
    )
}
