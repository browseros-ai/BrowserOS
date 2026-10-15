//! Tool sources for the built-in agent. Every tool reaches the loop through one seam,
//! [`ToolSource`]: browser tools arrive as MCP over loopback `/mcp` with a lease header (never as
//! Rust calls into `browseros-mcp-host`), plus custom MCP servers, the filesystem tools and the
//! nudge tools. Merged in TS order: internal MCP → custom MCP → filesystem → nudges.
//! Implemented by P7; S0 stubs fail to connect and refuse calls.

mod filesystem;
mod mcp;
mod nudges;

pub use filesystem::{FsScope, OutputGrants, filesystem_tools};
pub use mcp::{McpEndpoint, McpTransport, connect_mcp};
pub use nudges::nudge_tools;

use crate::messages::ToolResultOutput;
use serde_json::Value;
use std::{collections::BTreeMap, sync::Arc};
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone, PartialEq)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
    pub kind: ToolKind,
}

/// MCP tools are `Dynamic` (wire `dynamic: true` with `toolMetadata` + `title`); local tools are
/// `Static` (wire part type `tool-<name>`).
#[derive(Debug, Clone, PartialEq)]
pub enum ToolKind {
    Static,
    Dynamic {
        title: Option<String>,
        metadata: Value,
    },
}

/// A tool call's result in both forms: `ui` goes on the wire (`tool-output-available.output`;
/// the raw `CallToolResult` for MCP), `model` goes back into the transcript.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolOutput {
    pub ui: Value,
    pub model: ToolResultOutput,
}

/// Becomes a `tool-output-error` chunk; the turn continues.
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct ToolCallError(pub String);

#[derive(Debug, thiserror::Error)]
pub enum ToolSourceError {
    #[error("tool source \"{label}\" is unavailable: {message}")]
    Unavailable { label: String, message: String },
    #[error("tool source \"{label}\" timed out after {seconds}s")]
    Timeout { label: String, seconds: u64 },
}

/// One provider of tools. Implementations must be cancel-safe: `call` observes `cancel` and an
/// MCP source forwards it as `notifications/cancelled`.
#[async_trait::async_trait]
pub trait ToolSource: Send + Sync {
    fn label(&self) -> &str;
    async fn tools(&self) -> Result<Vec<ToolSpec>, ToolSourceError>;
    async fn call(
        &self,
        name: &str,
        input: Value,
        cancel: CancellationToken,
    ) -> Result<ToolOutput, ToolCallError>;
    /// Releases connections; called once by `Agent::dispose`.
    async fn close(&self);
}

pub struct SourceSpec {
    pub source: Arc<dyn ToolSource>,
    /// A required source failing to connect fails the whole build (the internal `/mcp`).
    pub required: bool,
}

/// Name → kind of every tool the agent was built with; `messages::ui_to_model` needs it to pick
/// `toModelOutput` per tool.
#[derive(Debug, Clone, Default)]
pub struct ToolCatalog {
    kinds: BTreeMap<String, ToolKind>,
}

impl ToolCatalog {
    pub fn new(specs: &[ToolSpec]) -> Self {
        Self {
            kinds: specs
                .iter()
                .map(|spec| (spec.name.clone(), spec.kind.clone()))
                .collect(),
        }
    }

    pub fn kind(&self, name: &str) -> Option<&ToolKind> {
        self.kinds.get(name)
    }
}

/// The S0 placeholder behind every constructor in this module until P7 lands real sources.
struct UnimplementedSource {
    label: String,
}

#[async_trait::async_trait]
impl ToolSource for UnimplementedSource {
    fn label(&self) -> &str {
        &self.label
    }

    async fn tools(&self) -> Result<Vec<ToolSpec>, ToolSourceError> {
        Err(ToolSourceError::Unavailable {
            label: self.label.clone(),
            message: "not implemented yet".to_owned(),
        })
    }

    async fn call(
        &self,
        name: &str,
        input: Value,
        cancel: CancellationToken,
    ) -> Result<ToolOutput, ToolCallError> {
        let _ = (input, cancel);
        Err(ToolCallError(format!(
            "{name}: tool source \"{}\" is not implemented yet",
            self.label
        )))
    }

    async fn close(&self) {}
}

fn unimplemented_source(label: &str) -> Arc<dyn ToolSource> {
    Arc::new(UnimplementedSource {
        label: label.to_owned(),
    })
}
