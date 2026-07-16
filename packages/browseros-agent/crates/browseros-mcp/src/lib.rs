//! BrowserOS browser tools and a thin rmcp server matching `packages/browser-mcp`.
//!
//! Hosts that own dispatch policy can implement `ServerHandler` directly over [`catalog`] and
//! [`execute_tool`], as `apps/claw-server-rust/src/mcp` does.

pub mod constants;
pub mod format;
pub mod framework;
pub mod output_file;
pub mod response;
pub mod service;
pub mod tools;
pub mod trust_boundary;

#[cfg(test)]
mod tests;

pub use framework::{
    BrowserToolDefaults, BrowserToolOptions, OutputFileAccess, ToolCtx, ToolDef, ToolMetadata,
    ToolResult, catalog, execute_tool, extract_page_id, result_page_id,
};
pub use service::{
    BROWSER_MCP_INSTRUCTIONS, BrowserMcpService, BrowserMcpServiceOptions, BrowserSessionProvider,
    BrowserToolExecutedCallback, BrowserToolExecutionEvent, BrowserToolLifecycleCallback,
    BrowserToolLifecycleEvent,
};
