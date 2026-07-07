use crate::framework::{OutputFileAccess, ToolCallHooks, ToolResult};
use browseros_core::BrowserSession;
use futures_util::future::BoxFuture;
use serde_json::Value;
use std::{fmt, sync::Arc};
use tokio_util::sync::CancellationToken;

pub type McpHookResult<T> = Result<T, McpHookError>;

#[derive(Debug, Clone)]
pub struct McpClientInfo {
    pub name: String,
    pub version: String,
    pub title: Option<String>,
}

#[derive(Debug, Clone)]
pub struct McpSessionStarted {
    pub session_id: String,
    pub client_info: McpClientInfo,
}

#[derive(Debug, Clone)]
pub struct McpSessionClosed {
    pub session_id: String,
    pub reason: String,
}

#[derive(Clone)]
pub struct McpToolCall {
    pub session_id: String,
    pub dispatch_id: String,
    pub tool_name: &'static str,
    pub raw_args: Value,
    pub hooks: ToolCallHooks,
    pub browser_session: Option<Arc<BrowserSession>>,
    pub cancel: CancellationToken,
    pub output_files: OutputFileAccess,
}

#[derive(Debug, Clone, Copy)]
pub struct McpToolTiming {
    pub duration_ms: i64,
}

pub struct McpBeforeToolResult {
    pub cancel: CancellationToken,
    pub result: Option<ToolResult>,
}

#[derive(Debug, Clone)]
pub struct McpHookError {
    message: String,
}

impl McpHookError {
    #[must_use]
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for McpHookError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for McpHookError {}

/// Host boundary for MCP session lifecycle and tool side effects.
pub trait McpHooks: Send + Sync + 'static {
    fn session_started(&self, event: McpSessionStarted) -> BoxFuture<'_, McpHookResult<()>> {
        Box::pin(async move {
            drop(event);
            Ok(())
        })
    }

    fn session_closed(&self, event: McpSessionClosed) -> BoxFuture<'_, McpHookResult<()>> {
        Box::pin(async move {
            drop(event);
            Ok(())
        })
    }

    fn before_tool(&self, call: McpToolCall) -> BoxFuture<'_, McpHookResult<McpBeforeToolResult>> {
        Box::pin(async move {
            Ok(McpBeforeToolResult {
                cancel: call.cancel,
                result: None,
            })
        })
    }

    fn after_tool<'a>(
        &'a self,
        call: McpToolCall,
        result: &'a mut ToolResult,
        timing: McpToolTiming,
    ) -> BoxFuture<'a, McpHookResult<()>> {
        Box::pin(async move {
            let _ = result;
            drop((call, timing));
            Ok(())
        })
    }
}

#[derive(Clone, Default)]
pub struct NoopMcpHooks;

impl McpHooks for NoopMcpHooks {}
