//! The provider layer: `LanguageModel` is the seam the loop calls; `RigModel` (production, over
//! rig-core 0.44, private module `rig`) and [`ScriptedModel`] (tests, `BROWSEROS_USE_MOCK_LLM`)
//! are its adapters. Also owns credential-shaped config (`LlmConfig`), the reasoning-options
//! table (TS `reasoning-config.ts`), the `ChatError` classifier, the retry rule the loop applies
//! before a step's first event, and one-shot `generate_text` for test-provider and refine-prompt.
//!
//! ```text
//!  LlmConfig ──create_language_model──► Arc<dyn LanguageModel>   (RigModel | ScriptedModel::mock)
//!  ReasoningSettings ──provider_options──► ProviderOptions ─┐
//!  ModelRequest { system, messages, tools, options } ───────┴─► stream() ─► ModelStream
//!  ModelError ──to_chat_error──► ChatError ──chat_error_text──► errorText / HTTP 500 body
//! ```

mod chat_error;
mod reasoning;
mod rig;
mod scripted;

pub use chat_error::{ChatError, ChatErrorCode, chat_error_text, redact, to_chat_error};
pub use reasoning::provider_options;
pub use scripted::{MOCK_LLM_RESPONSE_TEXT, ScriptedModel};

use crate::messages::{ModelMessage, UserPart};
use futures_core::Stream;
use futures_util::StreamExt;
use serde_json::{Map, Value};
use std::{collections::BTreeMap, pin::Pin, str::FromStr, sync::Arc, time::Duration};
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

/// `Unknown provider: ${provider}` (`provider-factory.ts:231`).
#[derive(Debug, thiserror::Error)]
#[error("Unknown provider: {0}")]
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

impl From<LlmConfigError> for ModelError {
    /// A thrown construction `Error` in TS: classified by its message (`requires apiKey` →
    /// `provider_config`, anything else → `unknown`).
    fn from(error: LlmConfigError) -> Self {
        ModelError::Other(error.0)
    }
}

/// `CONVERSATION_ID_PLACEHOLDER` of `@browseros/shared/schemas/llm`.
pub const CONVERSATION_ID_PLACEHOLDER: &str = "{{conversationId}}";

/// `LLMHeadersSchema`: token-character names, tab/printable/Latin-1 values, no case-insensitive
/// duplicates. `create_language_model` re-checks what `chat::request` validated.
pub fn validate_headers(headers: &BTreeMap<String, String>) -> Result<(), LlmConfigError> {
    const NAME_SYMBOLS: &str = "!#$%&'*+.^_`|~-";
    let mut seen = std::collections::BTreeSet::new();
    for (name, value) in headers {
        let name_ok = !name.is_empty()
            && name
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || NAME_SYMBOLS.contains(ch));
        if !name_ok {
            return Err(LlmConfigError(format!("Invalid header name: {name}")));
        }
        let value_ok = value.chars().all(|ch| {
            ch == '\t' || ('\u{20}'..='\u{7e}').contains(&ch) || ('\u{80}'..='\u{ff}').contains(&ch)
        });
        if !value_ok {
            return Err(LlmConfigError(format!("Invalid header value for {name}")));
        }
        if !seen.insert(name.to_ascii_lowercase()) {
            return Err(LlmConfigError(format!("Duplicate header name: {name}")));
        }
    }
    Ok(())
}

/// The shared HTTP client stack every `RigModel` is built on: one reqwest connection pool (the
/// process-wide rig client; no request timeout, TS parity) under a per-model request policy (TS
/// endpoint, headers, Codex/Copilot rewrites). Cheap to clone.
#[derive(Debug, Clone)]
pub struct HttpStack {
    client: ::rig_core::http_client::DynHttpClient,
    test_origin: Option<http::Uri>,
}

impl Default for HttpStack {
    fn default() -> Self {
        Self {
            client: rig_reqwest::shared(),
            test_origin: None,
        }
    }
}

impl HttpStack {
    pub fn new() -> Self {
        Self::default()
    }

    /// Test seam: send every request to `origin` (`scheme://host:port`) while keeping the
    /// provider's own path and query, so wire tests reach fixed endpoints (Codex, Copilot, Qwen,
    /// Bedrock) on a local fake server. Production never sets it.
    #[doc(hidden)]
    pub fn with_test_origin(mut self, origin: &str) -> Result<Self, LlmConfigError> {
        let uri = origin
            .parse::<http::Uri>()
            .ok()
            .filter(|uri| uri.scheme().is_some() && uri.authority().is_some())
            .ok_or_else(|| LlmConfigError(format!("Invalid test origin: {origin}")))?;
        self.test_origin = Some(uri);
        Ok(self)
    }

    pub(crate) fn client(&self) -> ::rig_core::http_client::DynHttpClient {
        self.client.clone()
    }

    pub(crate) fn test_origin(&self) -> Option<&http::Uri> {
        self.test_origin.as_ref()
    }
}

/// `shouldUseMockLLM()`: the harness switch, read from the environment the harness sets.
pub fn mock_llm_enabled() -> bool {
    std::env::var("BROWSEROS_USE_MOCK_LLM").is_ok_and(|value| value == "true")
}

/// `createLanguageModel` (chat) and `createLLMProvider` (test/refine) in one: the mock switch,
/// then the provider factory. `config.headers` get `{{conversationId}}` substituted with
/// `config.conversation_id` (test/refine pass a random id, as TS does).
pub fn create_language_model(
    config: &LlmConfig,
    http: &HttpStack,
) -> Result<Arc<dyn LanguageModel>, LlmConfigError> {
    if mock_llm_enabled() {
        return Ok(Arc::new(ScriptedModel::mock()));
    }
    Ok(Arc::new(rig::build(config, http)?))
}

/// The provider seam. Adapters: `RigModel` (production), [`ScriptedModel`] (tests,
/// `BROWSEROS_USE_MOCK_LLM`). One call = one model step; the loop owns retries and tool calls.
/// A failure before the stream exists is the `Err`; any later failure is the stream's last item.
#[async_trait::async_trait]
pub trait LanguageModel: Send + Sync {
    fn provider(&self) -> ProviderKind;
    fn model_id(&self) -> &str;
    /// Whether tool results may carry media (`anthropic`, `openai`, `azure`, `bedrock`,
    /// `google` + gemini-3); otherwise `messages::normalize_for_model` hoists it.
    fn supports_media_in_tool_results(&self) -> bool;
    /// Cancelling the token must end the stream promptly and drop the HTTP response: the next
    /// item is `Err(ModelError::Cancelled)` and the stream ends.
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

/// The TS `AgentReasoningConfig` object verbatim: an optional top-level `reasoning` level and/or
/// AI SDK `providerOptions` namespaces (`{"providerOptions":{"openai":{…}}}`). Built by
/// [`provider_options`]; the adapter renders it into the request body as each AI SDK provider
/// does. Empty means no reasoning options (test-provider, refine-prompt).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ProviderOptions(pub Map<String, Value>);

pub type ModelStream = Pin<Box<dyn Stream<Item = Result<ModelEvent, ModelError>> + Send>>;

/// One model step's events, provider-neutral. The loop maps these 1:1 onto `UiChunk`s.
///
/// - **Ids** are unique per call (so across a turn's steps). A tool call carries our `id` from
///   `ToolInputStart` on; `ToolCall.provider_call_id` is the provider's own id, which the
///   transcript must use as `tool_call_id` for the call and its result.
/// - **Order.** `*End` and `ToolCall` arrive in part-start order, so appending parts to the
///   step's assistant message as they end reproduces the provider's item order.
/// - **`provider_metadata`** on `*End`/`ToolCall` is opaque provider data (signatures, encrypted
///   reasoning). Store it verbatim as the matching `AssistantPart`'s `provider_options` (the AI SDK
///   providerMetadata → providerOptions convention); the adapter replays it on the next step when
///   the model is the one that produced it. It is JSON-safe to persist in UI messages.
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

#[derive(Debug, Clone, thiserror::Error)]
pub enum ModelError {
    /// An HTTP failure (AI SDK `APICallError`). `data` is boxed to keep `ModelError` small: it
    /// rides in every `Result` the stream yields.
    #[error("{message}")]
    Api {
        status: u16,
        message: String,
        response_body: Option<String>,
        response_headers: Vec<(String, String)>,
        data: Option<Box<Value>>,
        retryable: bool,
    },
    /// The "fetch failed" family: DNS, refused, reset, TLS.
    #[error("{0}")]
    Connection(String),
    #[error("{0}")]
    InvalidPrompt(String),
    /// `LoadAPIKeyError` / `NoSuchModelError` / an option the provider cannot honour.
    #[error("{0}")]
    Config(String),
    /// Any other failure (a malformed reply, an in-band provider error event): classified by its
    /// message, as TS classifies a plain thrown `Error`.
    #[error("{0}")]
    Other(String),
    #[error("cancelled")]
    Cancelled,
}

/// AI SDK `maxRetries`: two retries after the first attempt.
pub const MAX_RETRIES: u32 = 2;

impl ModelError {
    /// Whether the AI SDK would retry it: a retryable HTTP status (408/409/429/5xx) or a network
    /// failure (`Cannot connect to API` is retryable in the SDK).
    pub fn is_retryable(&self) -> bool {
        match self {
            Self::Api { retryable, .. } => *retryable,
            Self::Connection(_) => true,
            _ => false,
        }
    }
}

/// The wait before retry `attempt` (0-based) of a step that failed before its first event:
/// `retry-after-ms`, else `retry-after` seconds, when within 0–60 s; otherwise 2 s × 2ⁿ
/// (`retry-with-exponential-backoff.ts`).
pub fn retry_delay(error: &ModelError, attempt: u32) -> Duration {
    let backoff = Duration::from_millis(2_000u64.saturating_mul(2u64.saturating_pow(attempt)));
    let ModelError::Api {
        response_headers, ..
    } = error
    else {
        return backoff;
    };
    let header = |name: &str| {
        response_headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .and_then(|(_, value)| value.trim().parse::<f64>().ok())
    };
    let hinted_ms =
        header("retry-after-ms").or_else(|| header("retry-after").map(|seconds| seconds * 1000.0));
    match hinted_ms {
        Some(ms) if (0.0..=60_000.0).contains(&ms) => Duration::from_millis(ms as u64),
        _ => backoff,
    }
}

/// One-shot text generation for `/test-provider` and `/refine-prompt`: `streamText` with one user
/// message, no tools and no reasoning options, collecting the text deltas. A failure before the
/// first event is retried like a loop step; a failure after it ends the call with that error (TS
/// captures `onError` and rethrows once the text stream ends). Timeouts are the caller's
/// (`TIMEOUTS.TEST_PROVIDER` / `REFINE_PROMPT`), applied by cancelling `cancel`.
pub async fn generate_text(
    model: &dyn LanguageModel,
    system: Option<&str>,
    prompt: &str,
    cancel: CancellationToken,
) -> Result<String, ModelError> {
    let request = ModelRequest {
        system: system.map(str::to_owned),
        messages: vec![ModelMessage::User {
            content: vec![UserPart::Text {
                text: prompt.to_owned(),
                provider_options: None,
            }],
        }],
        tools: Vec::new(),
        options: ProviderOptions::default(),
    };
    let mut attempt = 0;
    let mut text = String::new();
    'attempts: loop {
        let mut stream = match model.stream(request.clone(), cancel.clone()).await {
            Ok(stream) => stream,
            Err(error) if error.is_retryable() && attempt < MAX_RETRIES => {
                wait_retry(&error, &mut attempt, &cancel).await?;
                continue;
            }
            Err(error) => return Err(error),
        };
        let mut received = false;
        while let Some(item) = stream.next().await {
            match item {
                Ok(ModelEvent::TextDelta { delta, .. }) => {
                    received = true;
                    text.push_str(&delta);
                }
                Ok(_) => received = true,
                Err(error) if !received && error.is_retryable() && attempt < MAX_RETRIES => {
                    wait_retry(&error, &mut attempt, &cancel).await?;
                    continue 'attempts;
                }
                Err(error) => return Err(error),
            }
        }
        return Ok(text);
    }
}

async fn wait_retry(
    error: &ModelError,
    attempt: &mut u32,
    cancel: &CancellationToken,
) -> Result<(), ModelError> {
    let delay = retry_delay(error, *attempt);
    *attempt += 1;
    tokio::select! {
        biased;
        _ = cancel.cancelled() => Err(ModelError::Cancelled),
        _ = tokio::time::sleep(delay) => Ok(()),
    }
}
