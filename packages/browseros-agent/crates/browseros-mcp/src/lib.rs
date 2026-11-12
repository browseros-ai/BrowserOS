//! BrowserOS browser tools and a thin rmcp server matching `packages/browser-mcp`.
//!
//! Hosts that own dispatch policy can implement `ServerHandler` directly over [`catalog`] and
//! [`execute_tool`], as `apps/claw-server/src/api/mcp` does.

pub mod constants;
pub mod format;
pub mod framework;
pub mod output_file;
pub mod output_store;
pub mod response;
pub mod run_policy;
pub mod service;
mod settle;
pub mod token_estimate;
pub mod tools;
pub mod trust_boundary;

#[cfg(test)]
mod evaluate_in_page_tests;
#[cfg(test)]
mod targeting_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod viewport_tests;

pub use run_policy::RunPolicy;

pub use framework::{
    AuditHints, BrowserToolDefaults, BrowserToolOptions, FeatureNote, InnerCallHook,
    InnerCallRecord, OutputFileAccess, StaticTargetStore, TargetStore, ToolCtx, ToolDef,
    ToolExecution, ToolMetadata, ToolResult, catalog, execute_tool, extract_page_id,
    result_page_id, unapplied_target_note,
};
pub use service::{
    BrowserMcpService, BrowserMcpServiceOptions, BrowserSessionProvider,
    BrowserToolExecutedCallback, BrowserToolExecutionEvent, BrowserToolLifecycleCallback,
    BrowserToolLifecycleEvent, browser_mcp_instructions,
};
