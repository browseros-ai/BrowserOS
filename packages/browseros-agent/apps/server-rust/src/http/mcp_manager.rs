//! `/mcp-manager/*`: the BrowserOS MCP tool list and per-agent install/uninstall over
//! `mcp_manager::McpManagerService`. Owned by P12.

use axum::response::Response;

pub async fn tools() -> Response {
    super::not_implemented()
}

pub async fn agents() -> Response {
    super::not_implemented()
}

pub async fn install() -> Response {
    super::not_implemented()
}

pub async fn uninstall() -> Response {
    super::not_implemented()
}
