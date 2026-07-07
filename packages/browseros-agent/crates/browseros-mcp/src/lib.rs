pub mod constants;
pub mod format;
pub mod framework;
pub mod hooks;
pub mod output_file;
pub mod response;
pub mod service;
pub mod tools;
pub mod trust_boundary;

#[cfg(test)]
mod tests;

pub use framework::{
    BrowserToolDefaults, BrowserToolOptions, OutputFileAccess, ToolCallHooks, ToolCtx, ToolDef,
    ToolMetadata, ToolResult, catalog, execute_tool,
};
pub use hooks::{
    McpBeforeToolResult, McpClientInfo, McpHookError, McpHookResult, McpHooks, McpSessionClosed,
    McpSessionStarted, McpToolCall, McpToolTiming, NoopMcpHooks,
};
pub use service::{
    BROWSER_MCP_INSTRUCTIONS, BrowserMcpService, BrowserMcpServiceOptions, BrowserSessionProvider,
    cancellation_result, extract_page_id, result_page_id,
};
