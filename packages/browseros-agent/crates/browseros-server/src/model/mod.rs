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
// The sign-in's skeleton temporarily expects endpoint() to be unused until dispatch exists.
#[allow(
    unfulfilled_lint_expectations,
    reason = "the sign-in implementation lands separately"
)]
mod chatgpt;
mod errors;
mod responses;
mod scripted;
mod sse;

use std::sync::Arc;
use std::sync::atomic::AtomicUsize;

use base64::Engine;
use http::header::{AUTHORIZATION, HeaderMap, HeaderName, HeaderValue};
use responses::{ResponsesDialect, ResponsesEndpoint};
use serde_json::Value;
use url::Url;

pub use chatgpt::{SignIn, SignInError, SignInStatus};
pub use scripted::{Script, ScriptedCall, ScriptedError, ScriptedStep, ScriptedUsage};

use crate::store::{Credentials, ProviderConfig, ProviderKind, Store};
use crate::turn::{
    ChatError, ConversationId, EventSink, FinishReason, ToolCallId, ToolOwner, TurnEvent,
};

/// The model side of the server. HTTP connection pooling is shared, while the route and
/// credentials are resolved per turn.
#[derive(Clone)]
pub struct Models {
    http: Result<reqwest::Client, Arc<reqwest::Error>>,
    sign_in: Arc<SignIn>,
    /// `Some` when `BROWSEROS_USE_MOCK_LLM` is on: every provider resolves to this script.
    script: Option<Arc<Script>>,
}

impl Models {
    pub fn new(store: Store, install_id: Option<String>) -> Self {
        Self {
            // Redirects could carry a saved custom credential to a different destination.
            // Retries belong to Model, which knows whether any output reached the turn.
            http: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .retry(reqwest::retry::never())
                .connect_timeout(std::time::Duration::from_secs(15))
                .build()
                .map_err(Arc::new),
            sign_in: Arc::new(SignIn::new(store, install_id)),
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
        let kind = provider.kind;
        let http = self.http.clone().map_err(|_| errors::disconnected())?;
        let route = match kind {
            ProviderKind::Google
            | ProviderKind::Openrouter
            | ProviderKind::Fireworks
            | ProviderKind::Azure
            | ProviderKind::OpenaiCompatible
            | ProviderKind::Ollama
            | ProviderKind::Lmstudio
            | ProviderKind::Moonshot => {
                let endpoint = chat::endpoint(&provider)?;
                Route::Chat {
                    http,
                    endpoint,
                    model: provider.model,
                }
            }
            ProviderKind::Openai => {
                let base = provider
                    .base_url
                    .as_deref()
                    .filter(|value| !value.trim().is_empty())
                    .unwrap_or("https://api.openai.com/v1/");
                let url = endpoint(base, "responses")?;
                let mut headers = provider.headers.clone();
                // The configured API key owns the Authorization header for this route.
                headers.push((
                    "authorization".into(),
                    format!("Bearer {}", api_key(&provider)?),
                ));
                let endpoint = ResponsesEndpoint {
                    url,
                    headers,
                    dialect: ResponsesDialect::OpenAi,
                };
                Route::Responses {
                    http,
                    endpoint,
                    model: provider.model,
                }
            }
            ProviderKind::ChatgptPro => {
                if !matches!(provider.credentials, Credentials::SignIn) {
                    return Err(errors::config(
                        "ChatGPT Pro requires signing in to ChatGPT.",
                    ));
                }
                let mut endpoint = self.sign_in.endpoint().await?;
                let mut headers = provider.headers;
                headers.extend(endpoint.headers);
                endpoint.headers = headers;
                Route::Responses {
                    http,
                    endpoint,
                    model: provider.model,
                }
            }
            ProviderKind::Anthropic => {
                let endpoint = anthropic::endpoint(&provider)?;
                Route::Anthropic {
                    http,
                    endpoint,
                    model: provider.model,
                }
            }
            ProviderKind::Bedrock => {
                return Err(ChatError::not_implemented("Calling this provider"));
            }
        };
        Ok(Model {
            route,
            window: provider.context_window,
            supports_images: provider.supports_images,
            kind: Some(kind),
            reasoning: Reasoning {
                effort: provider.reasoning_effort,
                summary: provider.reasoning_summary,
            },
        })
    }

    /// `POST /test-provider`: resolve (with a fresh random conversation id), then one tiny step
    /// with no tools and a 15 s limit.
    pub async fn test(&self, provider: ProviderConfig) -> Result<std::time::Duration, ChatError> {
        let start = std::time::Instant::now();
        let probe = async {
            let model = self.resolve(provider, ConversationId::new_v4()).await?;
            let messages = [ModelMessage::User {
                text: "Respond with exactly: 'ok'".into(),
                images: Vec::new(),
            }];
            let turn = model
                .step_once(
                    &StepRequest {
                        system: "",
                        messages: &messages,
                        tools: &[],
                        effort: None,
                    },
                    &mut ProbeSink,
                )
                .await?;
            match turn.finish() {
                FinishReason::ContentFilter => Err(ChatError::new(
                    crate::turn::ChatErrorCode::ContentFilter,
                    "Content filtered",
                    "The provider blocked the connection test.",
                )),
                FinishReason::Error => Err(errors::protocol(
                    "The provider returned a failed connection test.",
                )),
                _ => Ok(start.elapsed()),
            }
        };
        tokio::time::timeout(std::time::Duration::from_secs(15), probe)
            .await
            .map_err(|_| {
                ChatError::new(
                    crate::turn::ChatErrorCode::ConnectionFailed,
                    "Connection test timed out",
                    "The provider did not respond within 15 seconds.",
                )
            })?
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
    kind: Option<ProviderKind>,
    reasoning: Reasoning,
}

/// Which wire a resolved model speaks. The route owns its request-scoped authentication.
enum Route {
    Chat {
        http: reqwest::Client,
        endpoint: chat::Endpoint,
        model: String,
    },
    Responses {
        http: reqwest::Client,
        endpoint: ResponsesEndpoint,
        model: String,
    },
    Anthropic {
        http: reqwest::Client,
        endpoint: anthropic::Endpoint,
        model: String,
    },
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
            kind: None,
            reasoning: Reasoning::default(),
        }
    }

    /// One provider call. Streams `Text`, `Reasoning` and, when a call first appears in the
    /// response, `ToolStarted`. It never emits `ToolInput` or `ToolDone`: the finished calls
    /// are returned in `AssistantTurn::tool_calls()` and the loop runs them.
    /// Cancellation: the caller drops this future and the HTTPS stream closes.
    pub async fn step(
        &self,
        request: &StepRequest<'_>,
        sink: &mut dyn EventSink,
    ) -> Result<AssistantTurn, ChatError> {
        let mut sink = ObservedSink {
            inner: sink,
            emitted: false,
        };
        for attempt in 0..2 {
            match self.step_once(request, &mut sink).await {
                Err(error) if !sink.emitted => match errors::retry_delay(&error, attempt) {
                    Some(delay) => tokio::time::sleep(delay).await,
                    None => return Err(error),
                },
                result => return result,
            }
        }
        self.step_once(request, &mut sink).await
    }

    async fn step_once(
        &self,
        request: &StepRequest<'_>,
        sink: &mut dyn EventSink,
    ) -> Result<AssistantTurn, ChatError> {
        let result = match &self.route {
            Route::Chat {
                http,
                endpoint,
                model,
            } => chat::step(http, endpoint, model, request, &self.reasoning, sink).await,
            Route::Responses {
                http,
                endpoint,
                model,
            } => {
                if self.reasoning.effort.is_none() && self.reasoning.summary.is_none() {
                    responses::step(http, endpoint, model, request, sink).await
                } else {
                    responses::step_configured(
                        http,
                        endpoint,
                        model,
                        request,
                        &self.reasoning,
                        sink,
                    )
                    .await
                }
            }
            Route::Anthropic {
                http,
                endpoint,
                model,
            } => anthropic::step(http, endpoint, model, request, &self.reasoning, sink).await,
            Route::Scripted { script, next } => scripted::step(script, next, sink).await,
        };
        result.map_err(|mut error| {
            if let Some(kind) = self.kind {
                error.provider.get_or_insert_with(|| kind.as_str().into());
            }
            error
        })
    }

    /// The context window from the provider row; compaction's budget.
    pub fn window(&self) -> usize {
        self.window
    }

    pub fn supports_images(&self) -> bool {
        self.supports_images
    }
}

/// Whether the provider has emitted anything the user can see. Replaying a partially
/// visible response would duplicate text or tool cards, so retries stop at this seam.
struct ObservedSink<'a> {
    inner: &'a mut dyn EventSink,
    emitted: bool,
}

#[async_trait::async_trait]
impl EventSink for ObservedSink<'_> {
    async fn emit(&mut self, event: TurnEvent) {
        self.emitted = true;
        self.inner.emit(event).await;
    }
}

/// The settings probe drives the same stream to completion without producing panel events.
struct ProbeSink;

#[async_trait::async_trait]
impl EventSink for ProbeSink {
    async fn emit(&mut self, _event: TurnEvent) {}
}

/// The row supplies exact wire levels; each step supplies the switch and coarse fallback.
#[derive(Default)]
struct Reasoning {
    effort: Option<String>,
    summary: Option<String>,
}

impl Reasoning {
    fn effort<'a>(&'a self, fallback: Option<Effort>, levels: &[&str]) -> Option<&'a str> {
        let fallback = fallback?;
        Some(
            self.effort
                .as_deref()
                .filter(|level| levels.contains(level))
                .unwrap_or(match fallback {
                    Effort::Low => "low",
                    Effort::Medium => "medium",
                    Effort::High => "high",
                }),
        )
    }
}

/// The token budgets used by Claude models that implement manual extended thinking.
fn thinking_budget(effort: &str) -> i32 {
    match effort {
        "minimal" => 1024,
        "low" => 4096,
        "high" => 24_576,
        "xhigh" | "max" => 32_000,
        _ => 8192,
    }
}

fn endpoint(base: &str, operation: &str) -> Result<Url, ChatError> {
    let mut url =
        Url::parse(base).map_err(|_| errors::config("Enter a valid provider base URL."))?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return Err(errors::config(
            "Enter an HTTP or HTTPS base URL without credentials or a fragment.",
        ));
    }
    {
        let mut path = url
            .path_segments_mut()
            .map_err(|_| errors::config("Enter a valid provider base URL."))?;
        path.pop_if_empty().extend(operation.split('/'));
    }
    Ok(url)
}

fn headers(custom: &[(String, String)]) -> Result<HeaderMap, ChatError> {
    let mut headers = HeaderMap::new();
    for (name, value) in custom {
        let name = HeaderName::try_from(name.as_str())
            .map_err(|_| errors::config("A custom header has an invalid name."))?;
        // An overridden Host can route the request to a different tenant at the same URL.
        if matches!(
            name.as_str(),
            "host" | "content-length" | "transfer-encoding"
        ) {
            return Err(errors::config(
                "A custom header overrides HTTP routing or framing.",
            ));
        }
        set_header(&mut headers, name, value, true)?;
    }
    Ok(headers)
}

fn set_header(
    headers: &mut HeaderMap,
    name: HeaderName,
    value: &str,
    sensitive: bool,
) -> Result<(), ChatError> {
    let mut value = HeaderValue::from_str(value)
        .map_err(|_| errors::config("A credential or custom header has an invalid value."))?;
    value.set_sensitive(sensitive);
    headers.insert(name, value);
    Ok(())
}

fn api_key(provider: &ProviderConfig) -> Result<&str, ChatError> {
    match &provider.credentials {
        Credentials::ApiKey(key) if !key.expose().trim().is_empty() => Ok(key.expose()),
        _ => Err(errors::config(format!(
            "{} requires an API key.",
            provider.kind.as_str()
        ))),
    }
}

fn bearer(headers: &mut HeaderMap, key: &str) -> Result<(), ChatError> {
    set_header(headers, AUTHORIZATION, &format!("Bearer {key}"), true)
}

fn tool_call(id: &Value, name: &Value, input: Value) -> Result<ToolCall, ChatError> {
    let id = id
        .as_str()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| errors::protocol("A tool call arrived without an id."))?;
    let name = name
        .as_str()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| errors::protocol("A tool call arrived without a function name."))?;
    if !input.is_object() {
        return Err(errors::protocol(
            "A tool call's input must be a JSON object.",
        ));
    }
    Ok(ToolCall {
        id: ToolCallId(id.into()),
        name: name.into(),
        input,
    })
}

fn tool_input(arguments: &Value) -> Result<Value, ChatError> {
    arguments
        .as_str()
        .and_then(|input| serde_json::from_str(input).ok())
        .ok_or_else(|| errors::protocol("A tool call arrived without complete JSON arguments."))
}

async fn tool_started(sink: &mut dyn EventSink, id: &str, name: &str) {
    sink.emit(TurnEvent::ToolStarted {
        id: ToolCallId(id.into()),
        name: name.into(),
        owner: ToolOwner::Server,
    })
    .await;
}

fn data_url(image: &Image) -> String {
    format!(
        "data:{};base64,{}",
        image.media_type,
        base64::engine::general_purpose::STANDARD.encode(&image.bytes)
    )
}

fn usage(input: &Value, output: &Value) -> Option<Usage> {
    Some(Usage {
        input_tokens: input.as_u64()?,
        output_tokens: output.as_u64()?,
    })
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
    text: String,
    calls: Vec<ToolCall>,
    finish: FinishReason,
    usage: Option<Usage>,
    /// The provider's own assistant output, sent back untouched on the next step of the turn.
    native: NativeOutput,
}

/// What a provider returned for its turn, in its own shape: raw JSON for the hand-written wires
/// (so unknown fields such as signatures survive), the SDK's message for Bedrock.
enum NativeOutput {
    /// Earlier turns loaded from SQLite, and the scripted model: nothing to send back.
    None,
    /// The Chat Completions assistant message, the Responses output items, or the Anthropic
    /// content blocks, exactly as received.
    Chat(Value),
    Responses(Vec<Value>),
    Anthropic(Vec<Value>),
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
