//! `POST /mcp/{task}`: the private MCP endpoint a coding agent reaches the browser through.
//! It answers `initialize`, `tools/list` and `tools/call` for an armed task only, refuses any
//! request that carries `Origin` or `Sec-Fetch-Site` (a web page must never reach it), and
//! forwards a call into `Browser::call` under the scope the turn armed. Chunk `browser` fills
//! this file.

use super::Browser;

/// Mounted by `http` under `/mcp`. Until chunk `browser` lands it has no routes, so every
/// request is a `404`.
pub(super) fn router(_browser: Browser) -> axum::Router {
    axum::Router::new()
}
