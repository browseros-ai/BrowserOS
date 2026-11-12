//! The MCP face: one tool, `run { code, timeout? }`, served stateless over streamable HTTP
//! (rmcp 3.2 with `legacy_session_mode: false`), so no `Mcp-Session-Id` is issued on any
//! protocol version and nothing is kept between requests (design D3, D5).
//!
//! ```text
//!  rmcp ─▶ McpServer::call_tool ─ RunMeta::parse (_meta, the host's) + RunArgs::parse (the
//!          model's) ─▶ RunService::run ─▶ RunReply ─▶ RunTool::result ─▶ CallToolResult
//!          a refusal at the boundary ─▶ RunService::refuse (recorded, then the same rendering)
//! ```
//!
//! Cancel is a dropped response. Stateless rmcp answers `notifications/cancelled` with 202 and
//! drops it; the request's token fires only when the response stream goes away, which hyper
//! notices on the next SSE keep-alive write, so `Defaults::SSE_KEEP_ALIVE` bounds how long an
//! abandoned run keeps going. `call_tool` turns that token into dropping the run's future.

mod mcp_server;
mod run_args;
mod run_description;
mod run_tool;

pub(crate) use mcp_server::McpServer;
use run_args::RunArgs;
use run_tool::RunTool;
#[cfg(feature = "test-support")]
pub use run_tool::{ReplyText, parse_text};
