pub mod dispatch;
pub mod effects;
pub mod guards;
pub mod naming;
pub mod observers;
mod prompt;
mod service;
mod timeouts;

#[cfg(test)]
pub mod test_support;

use crate::{AppState, services::sessions::RetainedGroupAction};
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
        .set_retained_group_hook(Arc::new(move |ownership, key, action| {
            let browser = browser.clone();
            Box::pin(async move {
                let session = match browser.upgrade() {
                    Some(browser) => browser.session().await,
                    None => None,
                };
                match action {
                    RetainedGroupAction::Collapse => {
                        effects::tab_groups::collapse_agent_tab_group(
                            session.as_ref(),
                            &ownership,
                            &key,
                        )
                        .await
                    }
                    RetainedGroupAction::Close => {
                        effects::tab_groups::close_agent_tab_group(
                            session.as_ref(),
                            &ownership,
                            &key,
                        )
                        .await
                    }
                }
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
