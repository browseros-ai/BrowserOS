//! The agent loop: builds a merged tool set from its sources, renders the system prompt, then runs
//! turns as a step loop (model stream → tool calls in parallel → results → next step) that emits
//! `UiChunk`s in AI SDK order. Implemented by P7; S0 ships signatures with error-returning stubs.

use crate::{llm, prompt, tools};
use std::{collections::BTreeSet, sync::Arc};
use tokio_util::sync::CancellationToken;
use ui_message_stream::{ChunkStream, UiChunk, UiMessage};

/// What the loop needs to know about the model beyond the `LanguageModel` handle.
#[derive(Debug, Clone)]
pub struct ModelInfo {
    pub provider: llm::ProviderKind,
    pub model_id: String,
    /// `None` → 200 000 tokens, as in TS `computeBudget`.
    pub context_window: Option<f64>,
    pub supports_images: bool,
}

pub struct AgentConfig {
    pub model: Arc<dyn llm::LanguageModel>,
    pub info: ModelInfo,
    pub reasoning: llm::ReasoningSettings,
    pub prompt: prompt::PromptInputs,
    /// Merge order = precedence (later wins on name clash, except names owned by a `required`
    /// source: TS drops custom MCP tools that collide with internal ones).
    pub sources: Vec<tools::SourceSpec>,
    /// `stepCountIs(100)` in TS.
    pub max_steps: u32,
}

pub struct TurnInput {
    /// Display history; the last element is this turn's user message. Never rewritten here:
    /// the model-facing copy is derived per turn.
    pub history: Vec<UiMessage>,
    /// Model-facing replacement text for that last user message (context notices, browser
    /// context wrapper and `<USER_QUERY>`).
    pub prompt_text: String,
    /// `start.messageId` of the assistant message (fixes recon F8: TS persists `""`).
    pub message_id: String,
}

#[derive(Debug, thiserror::Error)]
pub enum AgentBuildError {
    #[error("Required MCP server \"{0}\" is unavailable.")]
    RequiredSourceUnavailable(String),
    #[error(transparent)]
    Source(#[from] tools::ToolSourceError),
}

/// A prepared agent for one conversation session: model, rendered system prompt, merged tool
/// set. Cheap to share; the engine caches one per conversation until its inputs change.
pub struct Agent {
    tool_names: BTreeSet<String>,
}

impl Agent {
    /// Connects every source (in parallel; a required failure aborts and closes the rest),
    /// merges tools, renders the prompt.
    pub async fn build(config: AgentConfig) -> Result<Agent, AgentBuildError> {
        let _ = config;
        Err(AgentBuildError::Source(
            tools::ToolSourceError::Unavailable {
                label: "browseros-agent".to_owned(),
                message: "the built-in agent loop is not implemented yet".to_owned(),
            },
        ))
    }

    pub fn tool_names(&self) -> &BTreeSet<String> {
        &self.tool_names
    }

    /// Runs one turn. Never errors: failures are `error` chunks; cancel → `abort` chunk then end.
    pub fn run_turn(&self, input: TurnInput, cancel: CancellationToken) -> ChunkStream {
        let _ = cancel;
        Box::pin(futures_util::stream::iter([
            UiChunk::Start {
                message_metadata: None,
                message_id: Some(input.message_id),
            },
            UiChunk::Error {
                error_text: "The built-in agent loop is not implemented yet.".to_owned(),
            },
        ]))
    }

    /// Closes MCP clients. Must run when the engine evicts a session, or loopback MCP sessions leak.
    pub async fn dispose(self) {}
}
