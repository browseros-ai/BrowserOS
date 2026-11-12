pub mod captcha;
pub mod dispatch;
pub mod distill;
pub mod effects;
pub mod guards;
pub mod helper_runtime;
pub mod hosted;
pub mod naming;
pub mod observers;
mod prompt;
pub mod script_hook;
mod service;
mod session_handle;
mod tab_grouping;
mod timeouts;

#[cfg(test)]
pub mod test_support;

use crate::{AppState, services::sessions::RetainedGroupAction};
use rmcp::transport::streamable_http_server::{
    session::local::LocalSessionManager,
    tower::{StreamableHttpServerConfig, StreamableHttpService},
};
use std::{sync::Arc, time::Duration};

pub use service::ClawMcpService;
// Re-exported so the route tests assert against the real text instead of keeping a
// copy of it, which is what let the two drift apart.
pub use session_handle::SESSION_ARG_DESCRIPTION;

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
    let transport_sessions = transport_session_manager(state.config.session_idle);
    StreamableHttpService::new(
        move || Ok(browser_mcp_service(state.clone())),
        Arc::new(transport_sessions),
        StreamableHttpServerConfig::default(),
    )
}

// rmcp drops a transport session after its own 5-minute keep-alive, which 404s a
// legacy client long before our session idle sweep would retire it.
fn transport_session_manager(session_idle: Duration) -> LocalSessionManager {
    let mut sessions = LocalSessionManager::default();
    sessions.session_config.keep_alive = Some(session_idle);
    sessions
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transport_keep_alive_follows_configured_session_idle() {
        let idle = Duration::from_secs(1017);
        let sessions = transport_session_manager(idle);
        assert_eq!(sessions.session_config.keep_alive, Some(idle));
    }
}
