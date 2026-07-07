pub mod hooks;

use crate::{AppState, mcp::hooks::ClawMcpHooks};
use browseros_mcp::{BrowserMcpService, BrowserMcpServiceOptions, BrowserSessionProvider};
use rmcp::transport::streamable_http_server::{
    session::local::LocalSessionManager,
    tower::{StreamableHttpServerConfig, StreamableHttpService},
};
use std::sync::Arc;

const SERVER_NAME: &str = "browseros-claw-server";
const SERVER_TITLE: &str = "BrowserOS";

/// Builds the shared MCP service used by both streamable HTTP and stdio.
#[must_use]
pub fn browser_mcp_service(state: AppState) -> BrowserMcpService {
    let browser = state.browser.clone();
    let browser_session_provider: BrowserSessionProvider = Arc::new(move || {
        let browser = browser.clone();
        Box::pin(async move { browser.session().await })
    });
    BrowserMcpService::new(BrowserMcpServiceOptions {
        name: SERVER_NAME.to_string(),
        title: SERVER_TITLE.to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        browser_session: None,
        browser_session_provider: Some(browser_session_provider),
        instructions: None,
        defaults: Default::default(),
        output_files: None,
        hooks: Some(Arc::new(ClawMcpHooks::new(state))),
    })
}

/// Builds the rmcp streamable HTTP service mounted at `/mcp`.
#[must_use]
pub fn streamable_http_service(
    state: AppState,
) -> StreamableHttpService<BrowserMcpService, LocalSessionManager> {
    StreamableHttpService::new(
        move || Ok(browser_mcp_service(state.clone())),
        Arc::new(LocalSessionManager::default()),
        StreamableHttpServerConfig::default(),
    )
}
