//! MCP tool sources: an rmcp streamable-HTTP client per endpoint (the internal
//! `http://127.0.0.1:<port>/mcp?structured=1[&read_only=1]` with the lease header, and custom
//! servers), plus transport detection with a legacy-SSE fallback (`mcp-transport-detect.ts`).

use super::{ToolSource, ToolSourceError};
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum McpTransport {
    StreamableHttp,
    /// `mcp-transport-detect.ts` (1 h cache), falling back to legacy SSE.
    Detect,
}

#[derive(Debug, Clone)]
pub struct McpEndpoint {
    pub label: String,
    pub url: url::Url,
    pub headers: Vec<(String, String)>,
    pub transport: McpTransport,
}

/// Connects and lists tools within 15 s. S0 stub: always unavailable.
pub async fn connect_mcp(endpoint: McpEndpoint) -> Result<Arc<dyn ToolSource>, ToolSourceError> {
    Err(ToolSourceError::Unavailable {
        label: endpoint.label,
        message: "MCP client tool source is not implemented yet".to_owned(),
    })
}
