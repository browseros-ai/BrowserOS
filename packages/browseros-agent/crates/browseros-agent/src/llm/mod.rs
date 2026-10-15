//! The provider layer: `LanguageModel` is the seam the loop calls; `RigModel` (production, over
//! rig-core 0.44) and [`ScriptedModel`] (tests, `BROWSEROS_USE_MOCK_LLM`) are its adapters.
//! Also owns credential-shaped config (`LlmConfig`), the reasoning-options table (TS
//! `reasoning-config.ts`), the `ChatError` classifier and one-shot `generate_text` used by
//! test-provider and refine-prompt. Implemented by P5; S0 ships signatures with stubs.

mod chat_error;
mod scripted;

pub use chat_error::{ChatError, ChatErrorCode, chat_error_text, redact, to_chat_error};
pub use scripted::ScriptedModel;

use crate::messages::ModelMessage;
use futures_core::Stream;
use serde_json::{Map, Value};
use std::{collections::BTreeMap, pin::Pin, str::FromStr, sync::Arc};
use tokio_util::sync::CancellationToken;

/// The 13 `LLM_PROVIDERS` ids of `@browseros/shared/schemas/llm`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProviderKind {
    Anthropic,
    OpenAi,
    Google,
    OpenRouter,
    Azure,
    Ollama,
    LmStudio,
    Bedrock,
    OpenAiCompatible,
    Moonshot,
    ChatGptPro,
    GitHubCopilot,
    QwenCode,
}

impl ProviderKind {
    pub const ALL: [ProviderKind; 13] = [
        Self::Anthropic,
        Self::OpenAi,
        Self::Google,
        Self::OpenRouter,
        Self::Azure,
        Self::Ollama,
        Self::LmStudio,
        Self::Bedrock,
        Self::OpenAiCompatible,
        Self::Moonshot,
        Self::ChatGptPro,
        Self::GitHubCopilot,
        Self::QwenCode,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Anthropic => "anthropic",
            Self::OpenAi => "openai",
            Self::Google => "google",
            Self::OpenRouter => "openrouter",
            Self::Azure => "azure",
            Self::Ollama => "ollama",
            Self::LmStudio => "lmstudio",
            Self::Bedrock => "bedrock",
            Self::OpenAiCompatible => "openai-compatible",
            Self::Moonshot => "moonshot",
            Self::ChatGptPro => "chatgpt-pro",
            Self::GitHubCopilot => "github-copilot",
            Self::QwenCode => "qwen-code",
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("unknown LLM provider: {0}")]
pub struct UnknownProvider(pub String);

impl FromStr for ProviderKind {
    type Err = UnknownProvider;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|kind| kind.as_str() == value)
            .ok_or_else(|| UnknownProvider(value.to_owned()))
    }
}

/// A resolved provider configuration: stored row merged with inline request fields and OAuth
/// credentials by `apps/server-rust` (`chat::request`) before it reaches this crate.
#[derive(Debug, Clone)]
pub struct LlmConfig {
    pub provider: ProviderKind,
    pub model: String,
    pub api_key: Option<String>,
    pub base_url: Option<String>,
    /// Already validated (`LLMHeadersSchema`) and `{{conversationId}}`-substituted.
    pub headers: Option<BTreeMap<String, String>>,
    pub resource_name: Option<String>,
    pub region: Option<String>,
    pub access_key_id: Option<String>,
    pub secret_access_key: Option<String>,
    pub session_token: Option<String>,
    pub account_id: Option<String>,
    pub upstream_provider: Option<String>,
    pub conversation_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReasoningEffort {
    None,
    Low,
    Medium,
    High,
    XHigh,
    Max,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ReasoningSummary {
    #[default]
    Auto,
    Concise,
    Detailed,
}

#[derive(Debug, Clone, Default)]
pub struct ReasoningSettings {
    pub supports_reasoning: bool,
    pub effort: Option<ReasoningEffort>,
    pub summary: ReasoningSummary,
}

/// Construction failures, with the exact TS texts (e.g. `Anthropic provider requires apiKey`).
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct LlmConfigError(pub String);

/// The shared HTTP client stack every `RigModel` is built on (one connection pool; per-model
/// header middleware for Copilot/Codex/OpenRouter quirks). Cheap to clone.
#[derive(Debug, Clone, Default)]
pub struct HttpStack {
    _private: (),
}

impl HttpStack {
    pub fn new() -> Self {
        Self::default()
    }
}

pub fn create_language_model(
    config: &LlmConfig,
    http: &HttpStack,
) -> Result<Arc<dyn LanguageModel>, LlmConfigError> {
    let _ = http;
    Err(LlmConfigError(format!(
        "{} provider is not implemented yet",
        config.provider.as_str()
    )))
}

/// The provider seam. Adapters: `RigModel` (production), [`ScriptedModel`] (tests,
/// `BROWSEROS_USE_MOCK_LLM`). One call = one model step; the loop owns retries and tool calls.
#[async_trait::async_trait]
pub trait LanguageModel: Send + Sync {
    fn provider(&self) -> ProviderKind;
    fn model_id(&self) -> &str;
    /// Whether tool results may carry media (`anthropic`, `openai`, `azure`, `bedrock`,
    /// `google` + gemini-3); otherwise `messages::normalize_for_model` hoists it.
    fn supports_media_in_tool_results(&self) -> bool;
    /// Cancelling the token must end the stream promptly and drop the HTTP response.
    async fn stream(
        &self,
        request: ModelRequest,
        cancel: CancellationToken,
    ) -> Result<ModelStream, ModelError>;
}

#[derive(Debug, Clone)]
pub struct ModelRequest {
    pub system: Option<String>,
    pub messages: Vec<ModelMessage>,
    pub tools: Vec<ToolDefinition>,
    pub options: ProviderOptions,
}

#[derive(Debug, Clone)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

/// The AI SDK `providerOptions` / top-level reasoning params, rendered into rig request params by
/// the adapter.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ProviderOptions(pub Map<String, Value>);

/// Port of `reasoning-config.ts:102-205` (recon A3). S0 stub: no options.
pub fn provider_options(
    provider: ProviderKind,
    model_id: &str,
    settings: &ReasoningSettings,
) -> ProviderOptions {
    let _ = (provider, model_id, settings);
    ProviderOptions::default()
}

pub type ModelStream = Pin<Box<dyn Stream<Item = Result<ModelEvent, ModelError>> + Send>>;

/// One model step's events, provider-neutral. The loop maps these 1:1 onto `UiChunk`s.
#[derive(Debug, Clone, PartialEq)]
pub enum ModelEvent {
    TextStart {
        id: String,
        provider_metadata: Option<Value>,
    },
    TextDelta {
        id: String,
        delta: String,
    },
    TextEnd {
        id: String,
        provider_metadata: Option<Value>,
    },
    ReasoningStart {
        id: String,
        provider_metadata: Option<Value>,
    },
    ReasoningDelta {
        id: String,
        delta: String,
        provider_metadata: Option<Value>,
    },
    ReasoningEnd {
        id: String,
        provider_metadata: Option<Value>,
    },
    ToolInputStart {
        id: String,
        tool_name: String,
    },
    ToolInputDelta {
        id: String,
        delta: String,
    },
    ToolCall {
        id: String,
        provider_call_id: String,
        tool_name: String,
        input_json: String,
        provider_metadata: Option<Value>,
    },
    Finish {
        reason: ModelFinishReason,
        usage: Usage,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelFinishReason {
    Stop,
    Length,
    ContentFilter,
    ToolCalls,
    Error,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Usage {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub cached_input_tokens: Option<u64>,
    pub reasoning_tokens: Option<u64>,
}

#[derive(Debug, thiserror::Error)]
pub enum ModelError {
    #[error("{message}")]
    Api {
        status: u16,
        message: String,
        response_body: Option<String>,
        response_headers: Vec<(String, String)>,
        data: Option<Value>,
        retryable: bool,
    },
    /// The "fetch failed" family: DNS, refused, reset, TLS.
    #[error("{0}")]
    Connection(String),
    #[error("{0}")]
    InvalidPrompt(String),
    #[error("{0}")]
    Config(String),
    #[error("cancelled")]
    Cancelled,
}

/// One-shot text generation for `/test-provider` and `/refine-prompt`.
pub async fn generate_text(
    model: &dyn LanguageModel,
    system: Option<&str>,
    prompt: &str,
    cancel: CancellationToken,
) -> Result<String, ModelError> {
    let _ = (system, prompt, cancel);
    Err(ModelError::Config(format!(
        "generate_text is not implemented yet for {}",
        model.provider().as_str()
    )))
}
