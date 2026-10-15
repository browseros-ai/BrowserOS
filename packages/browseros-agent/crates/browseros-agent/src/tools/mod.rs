//! Tool sources for the built-in agent. Every tool reaches the loop through one seam,
//! [`ToolSource`]: browser tools arrive as MCP over loopback `/mcp` with a lease header (never as
//! Rust calls into `browseros-mcp-host`), plus custom MCP servers, the filesystem tools and the
//! nudge tools. `Agent::build` merges them in TS order: internal MCP → custom MCP → filesystem →
//! nudges (`ai-sdk-agent.ts:268-281`).
//!
//! The split of responsibilities mirrors the AI SDK's `tool()` / `dynamicTool()`: a source owns a
//! tool's description, JSON schema, execution and `toModelOutput`; the loop owns parsing the
//! model's input, validation of static tools' inputs, ordering and error chunks.

mod filesystem;
mod mcp;
mod nudges;
mod schema;

pub use filesystem::{FsScope, OutputGrants, filesystem_tools};
pub use mcp::{McpEndpoint, McpTransport, connect_mcp};
pub use nudges::nudge_tools;

pub(crate) use nudges::{SUGGEST_APP_CONNECTION, SUGGEST_SCHEDULE};

pub(crate) use schema::validate_static_input;

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

/// `JSON.stringify` of an object whose keys must come out in a fixed order no matter whether
/// serde_json's `preserve_order` feature is on (feature unification makes it workspace-wide
/// whenever one crate enables it): the nudge sentinels are parsed by
/// the app, but their text is also model-visible and byte-compared by the compat harness.
pub(crate) fn ordered_json_object(entries: &[(&str, &Value)]) -> String {
    let body = entries
        .iter()
        .map(|(key, value)| format!("{}:{}", Value::from(*key), value))
        .collect::<Vec<_>>()
        .join(",");
    format!("{{{body}}}")
}
