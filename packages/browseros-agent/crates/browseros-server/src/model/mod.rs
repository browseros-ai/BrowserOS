//! `model`: one model call per agent step, for every provider type. The server owns the
//! loop (`agent`); this module owns everything about talking to a model once: the provider
//! wires, credentials, reasoning options, retries before first output, and turning provider
//! errors into the extension's `ChatError`.
//!
//! ```text
//!   Models::resolve(ProviderConfig, conversation) ──▶ Model ──step(request, sink)──▶ AssistantTurn
//!                                                      │      streams Text / Reasoning / ToolStarted;
//!                                                      │      the finished calls are returned, and
//!                                                      │      the loop emits ToolInput as it runs each
//!                                                      └─ chat · responses · anthropic · Bedrock · scripted
//! ```
//!
//! Three wires are written by hand over one shared `reqwest` client (OpenAI Chat Completions,
//! OpenAI Responses, Anthropic Messages) and Bedrock goes through the AWS SDK; no model library
//! is used, because the one picked (genai) changed `serde_json` for every crate in the build
//! (CORRECTIONS-2). Provider wire types never leave this module (design rule 4).

mod anthropic;
mod bedrock;
mod chat;
mod chatgpt;
mod errors;
mod responses;
mod scripted;
mod sse;

use std::sync::Arc;
use std::sync::atomic::AtomicUsize;

pub use chatgpt::{SignIn, SignInError, SignInStatus};
pub use scripted::{Script, ScriptedCall, ScriptedError, ScriptedStep, ScriptedUsage};

use crate::store::{ProviderConfig, ProviderKind, Store};
use crate::turn::{ChatError, ConversationId, EventSink, FinishReason, ToolCallId};

/// The model side of the server, shared by every turn: one shared `reqwest` client (chunk
/// `models` adds it), the store (for the ChatGPT tokens), and the sign-in. Cheap to clone.
#[derive(Clone)]
pub struct Models {
    #[expect(
        dead_code,
        reason = "read once chunk `models` writes the provider routes"
    )]
    store: Store,
    sign_in: Arc<SignIn>,
    /// `Some` when `BROWSEROS_USE_MOCK_LLM` is on: every provider resolves to this script.
    script: Option<Arc<Script>>,
}

impl Models {
    pub fn new(store: Store, install_id: Option<String>) -> Self {
        Self {
            sign_in: Arc::new(SignIn::new(store.clone(), install_id)),
            store,
            script: None,
        }
    }

    /// Every provider resolves to `script`, so tests and the end-to-end proof need no key.
    /// `app` picks this when `BROWSEROS_USE_MOCK_LLM=true`.
    pub fn scripted(store: Store, install_id: Option<String>, script: Script) -> Self {
        Self {
            script: Some(Arc::new(script)),
            ..Self::new(store, install_id)
        }
    }

    /// Builds the route for a provider row. Replaces the literal `{{conversationId}}` in the
    /// row's header values with `conversation`, once, here. For ChatGPT it refreshes the token
    /// first.
    pub async fn resolve(
        &self,
        mut provider: ProviderConfig,
        conversation: ConversationId,
    ) -> Result<Model, ChatError> {
        let conversation = conversation.to_string();
        for (_, value) in &mut provider.headers {
            *value = value.replace(CONVERSATION_ID_PLACEHOLDER, &conversation);
        }
        if let Some(script) = &self.script {
            return Ok(Model::scripted(
                Script::clone(script),
                provider.context_window,
                provider.supports_images,
            ));
        }
        // One arm per provider type, decided by the row's type and never by the model name.
        // Chunk `models` fills the first eleven; chunk `chatgpt` owns the last.
        match provider.kind {
            ProviderKind::Anthropic
            | ProviderKind::Openai
            | ProviderKind::Google
            | ProviderKind::Openrouter
            | ProviderKind::Fireworks
            | ProviderKind::Azure
            | ProviderKind::OpenaiCompatible
            | ProviderKind::Ollama
            | ProviderKind::Lmstudio
            | ProviderKind::Moonshot => Err(ChatError::not_implemented("Calling this provider")),
            ProviderKind::Bedrock => Err(ChatError::not_implemented("Calling AWS Bedrock")),
            ProviderKind::ChatgptPro => Err(ChatError::not_implemented(
                "Calling ChatGPT through sign-in",
            )),
        }
    }

    /// `POST /test-provider`: resolve (with a fresh random conversation id), then one tiny step
    /// with no tools and a 15 s limit.
    pub async fn test(&self, _provider: ProviderConfig) -> Result<std::time::Duration, ChatError> {
        Err(ChatError::not_implemented("Testing a provider"))
    }

    pub fn sign_in(&self) -> &SignIn {
        &self.sign_in
    }
}

/// The literal a provider row's header value may hold, replaced by the conversation id.
const CONVERSATION_ID_PLACEHOLDER: &str = "{{conversationId}}";

/// A resolved provider, ready to step. Built per turn; holds request-scoped credentials.
pub struct Model {
    route: Route,
    window: usize,
    supports_images: bool,
}

/// Which wire a `Model` speaks. Chunk `models` adds genai and Bedrock, chunk `chatgpt` the
/// sign-in route.
enum Route {
    Scripted {
        script: Script,
        /// The next step to play: one `step` call plays one scripted step.
        next: AtomicUsize,
    },
}

impl Model {
    /// A model that plays `script`, one step per `step` call, whatever the request says.
    pub fn scripted(script: Script, window: usize, supports_images: bool) -> Self {
        Self {
            route: Route::Scripted {
                script,
                next: AtomicUsize::new(0),
            },
            window,
            supports_images,
        }
    }

    /// One provider call. Streams `Text`, `Reasoning` and, when a call first appears in the
    /// response, `ToolStarted`. It never emits `ToolInput` or `ToolDone`: the finished calls
    /// are returned in `AssistantTurn::tool_calls()` and the loop runs them.
    /// Cancellation: the caller drops this future and the HTTPS stream closes.
    pub async fn step(
        &self,
        _request: &StepRequest<'_>,
        sink: &mut dyn EventSink,
    ) -> Result<AssistantTurn, ChatError> {
        match &self.route {
            Route::Scripted { script, next } => scripted::step(script, next, sink).await,
        }
    }

    /// The context window from the provider row; compaction's budget.
    pub fn window(&self) -> usize {
        self.window
    }

    pub fn supports_images(&self) -> bool {
        self.supports_images
    }
}

/// One model call's input. Borrowed from the loop, which owns the messages for the turn.
pub struct StepRequest<'a> {
    pub system: &'a str,
    pub messages: &'a [ModelMessage],
    pub tools: &'a [ToolSpec],
    pub effort: Option<Effort>,
}

/// A tool as offered to a model: today the browser's `run`.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

/// The reasoning effort a step asks for, mapped per provider type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effort {
    Low,
    Medium,
    High,
}

/// The loop's provider-neutral context: built from the stored transcript at the start of a
/// turn, extended in memory during it, never stored.
pub enum ModelMessage {
    User {
        text: String,
        images: Vec<Image>,
    },
    Assistant(AssistantTurn),
    /// Text in the first version, for every wire.
    ToolResult {
        id: ToolCallId,
        text: String,
    },
}

/// A picture the model sees in a user message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Image {
    pub media_type: String,
    pub bytes: Vec<u8>,
}

/// One provider response. It carries the provider's own message (signed or encrypted
/// reasoning, call ids) so the next step of the same turn sends it back verbatim. Opaque
/// to the loop.
pub struct AssistantTurn {
    #[expect(
        dead_code,
        reason = "read once chunk `models` writes the provider routes"
    )]
    text: String,
    calls: Vec<ToolCall>,
    finish: FinishReason,
    usage: Option<Usage>,
    /// The provider's own assistant output, sent back untouched on the next step of the turn.
    #[expect(dead_code, reason = "read once chunk `models` writes the wires")]
    native: NativeOutput,
}

/// What a provider returned for its turn, in its own shape: raw JSON for the hand-written wires
/// (so unknown fields such as signatures survive), the SDK's message for Bedrock.
enum NativeOutput {
    /// Earlier turns loaded from SQLite, and the scripted model: nothing to send back.
    None,
    /// The Chat Completions assistant message, the Responses output items, or the Anthropic
    /// content blocks, exactly as received.
    #[expect(dead_code, reason = "built once chunk `models` writes the wires")]
    Json(serde_json::Value),
    #[expect(
        dead_code,
        reason = "built once chunk `models` writes the Bedrock route"
    )]
    Bedrock(Box<aws_sdk_bedrockruntime::types::Message>),
}

/// Token counts a provider reported for one call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
}

impl AssistantTurn {
    /// For history loaded from SQLite: earlier turns reach the model as text.
    pub fn text(text: String) -> Self {
        Self::plain(text, Vec::new(), FinishReason::Stop, None)
    }

    /// A turn with no provider-native record to send back (the scripted model's).
    fn plain(
        text: String,
        calls: Vec<ToolCall>,
        finish: FinishReason,
        usage: Option<Usage>,
    ) -> Self {
        Self {
            text,
            calls,
            finish,
            usage,
            native: NativeOutput::None,
        }
    }

    pub fn tool_calls(&self) -> &[ToolCall] {
        &self.calls
    }

    pub fn finish(&self) -> FinishReason {
        self.finish
    }

    /// What the provider reported for this call, when it reported anything. Compaction's first input.
    pub fn usage(&self) -> Option<Usage> {
        self.usage
    }
}

/// One tool call a model asked for, with its complete input.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolCall {
    pub id: ToolCallId,
    pub name: String,
    pub input: serde_json::Value,
}
