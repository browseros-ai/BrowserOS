//! `model`: one model call per agent step, for every provider type. The server owns the
//! loop (`agent`); this module owns everything about talking to a model once: which provider a
//! row reaches and with which credential, reasoning options, retries before first output, and
//! turning provider errors into the extension's `ChatError`.
//!
//! ```text
//!   Models::resolve(ProviderConfig, conversation) ──▶ Model ──step(request, sink)──▶ AssistantTurn
//!                          │                            │   streams Text / Reasoning / ToolStarted;
//!                  route::Route (Rig model)              │   the finished calls are returned, and
//!                  or the scripted model                 │   the loop emits ToolInput as it runs each
//!                                                        └─ history ──▶ Rig ──▶ stream fold ──▶ sink
//! ```
//!
//! Rig (`rig-core`, `rig-bedrock`) speaks the provider wires: Chat Completions for eight row
//! types, OpenAI Responses, the ChatGPT backend, Anthropic Messages and Bedrock ConverseStream.
//! The server keeps what is its own: the row's URL and headers (`route`), the sign-in
//! (`chatgpt`), the error cards and retry rule (`errors`), and the order the loop's context is
//! sent in (`history`). Rig types never leave this module (design rule 4).

mod bedrock;
mod chatgpt;
mod errors;
mod route;
mod scripted;

#[cfg(test)]
#[path = "../../tests/support/fake_provider.rs"]
mod fake_provider;

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::AtomicUsize;

use futures_util::StreamExt;
use http::header::{HeaderMap, HeaderName, HeaderValue};
use rig_core::completion::{self as rig, CompletionRequest, ToolDefinition};
use rig_core::message::{
    AssistantContent, AssistantMessage, CallId, ImageMediaType, Message, ToolName,
    ToolResultContent, UserContent,
};
use rig_core::streaming::{Item, PartKind, StreamEvent};

pub use chatgpt::{SignIn, SignInError, SignInStatus};
pub use scripted::{Script, ScriptedCall, ScriptedError, ScriptedStep, ScriptedUsage};

use crate::store::{Credentials, ProviderConfig, ProviderKind, Store};
use crate::turn::{
    ChatError, ConversationId, EventSink, FinishReason, ToolCallId, ToolOwner, TurnEvent,
};
use route::{Clients, Route};

/// The model side of the server. HTTP connection pooling is shared, while the route and
/// credentials are resolved per turn.
#[derive(Clone)]
pub struct Models {
    http: Result<Clients, Arc<reqwest_rig::Error>>,
    sign_in: Arc<SignIn>,
    /// `Some` when `BROWSEROS_USE_MOCK_LLM` is on: every provider resolves to this script.
    script: Option<Arc<Script>>,
}

impl Models {
    pub fn new(store: Store, install_id: Option<String>) -> Self {
        Self {
            http: Clients::new().map_err(Arc::new),
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
        let clients = self.http.as_ref().map_err(|_| errors::disconnected())?;
        let route = match provider.kind {
            ProviderKind::ChatgptPro => {
                if !matches!(provider.credentials, Credentials::SignIn) {
                    return Err(errors::config(
                        "ChatGPT Pro requires signing in to ChatGPT.",
                    ));
                }
                let session = self.sign_in.session().await?;
                Route::chatgpt(&provider, session, clients)?
            }
            _ => Route::new(&provider, clients)?,
        };
        Ok(Model {
            route: Backend::Rig(Box::new(route)),
            name: provider.model,
            window: provider.context_window,
            supports_images: provider.supports_images,
            kind: Some(provider.kind),
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
    route: Backend,
    /// The row's model id.
    name: String,
    window: usize,
    supports_images: bool,
    kind: Option<ProviderKind>,
    reasoning: Reasoning,
}

enum Backend {
    Rig(Box<Route>),
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
            route: Backend::Scripted {
                script,
                next: AtomicUsize::new(0),
            },
            name: String::new(),
            window,
            supports_images,
            kind: None,
            reasoning: Reasoning::default(),
        }
    }

    /// One provider call. Streams `Text`, `Reasoning` and, when a call first appears in the
    /// response, `ToolStarted`. It never emits `ToolInput` or `ToolDone`: the finished calls
    /// are returned in `AssistantTurn::tool_calls()` and the loop runs them.
    /// Cancellation: the caller drops this future, which drops Rig's stream and closes the
    /// HTTPS connection (or the Bedrock event stream).
    pub async fn step(
        &self,
        request: &StepRequest<'_>,
        sink: &mut dyn EventSink,
    ) -> Result<AssistantTurn, ChatError> {
        if matches!(self.route, Backend::Scripted { .. }) {
            return self.step_once(request, sink).await;
        }
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
            Backend::Rig(route) => self.call(route, request, sink).await,
            Backend::Scripted { script, next } => scripted::step(script, next, sink).await,
        };
        result.map_err(|mut error| {
            if let Some(kind) = self.kind {
                error.provider.get_or_insert_with(|| kind.as_str().into());
            }
            error
        })
    }

    /// One streamed Rig call, folded into the sink and an `AssistantTurn`.
    async fn call(
        &self,
        route: &Route,
        request: &StepRequest<'_>,
        sink: &mut dyn EventSink,
    ) -> Result<AssistantTurn, ChatError> {
        let mut messages = Vec::with_capacity(request.messages.len() + 1);
        let system = route.wire.system(request.system);
        if !system.is_empty() {
            messages.push(Message::system(system));
        }
        messages.extend(history(request.messages));
        let tools = request
            .tools
            .iter()
            .map(|tool| {
                let name = ToolName::new(&tool.name)
                    .map_err(|_| errors::protocol("A tool has no name."))?;
                Ok(ToolDefinition::new(
                    name,
                    &tool.description,
                    tool.input_schema.clone(),
                ))
            })
            .collect::<Result<Vec<_>, ChatError>>()?;
        let options = route
            .wire
            .options(&self.name, request.effort, &self.reasoning);
        let call = CompletionRequest::from(messages)
            .tools(tools)
            .additional_params(options.params)
            .max_tokens(options.max_tokens);

        let mut stream = route
            .model
            .stream(call)
            .map_err(|error| route.error(&error))?;
        let mut fold = Fold::default();
        while let Some(item) = stream.next().await {
            if let Item::Event(event) = item.map_err(|error| route.error(&error))? {
                fold.event(event, sink).await;
            }
        }
        let response = stream.finish().await.map_err(|error| route.error(&error))?;
        fold.complete(response, sink).await
    }

    /// The context window from the provider row; compaction's budget.
    pub fn window(&self) -> usize {
        self.window
    }

    pub fn supports_images(&self) -> bool {
        self.supports_images
    }
}

/// Rig's stream events as the panel's. Rig names a call when its part starts but gives the
/// provider's call id only when it ends, so the server issues its own id at the start: the
/// `ToolStarted` card and the finished call carry the same id, and the Rig call keeps the
/// provider's for the next step's tool result. The response's `choice` is in the order parts
/// ended, not started, so the finished calls are matched by the provider's id, not position.
#[derive(Default)]
struct Fold {
    text: String,
    /// Part index → the id issued when that tool call started.
    started: BTreeMap<usize, ToolCallId>,
    /// The provider's call id → the id issued for it, once its part ended.
    ended: BTreeMap<CallId, ToolCallId>,
}

impl Fold {
    async fn event(&mut self, event: StreamEvent, sink: &mut dyn EventSink) {
        match event {
            StreamEvent::Start {
                part,
                kind: PartKind::ToolCall,
                name: Some(name),
            } => {
                let id = self.started.entry(part.index()).or_insert_with(new_call_id);
                tool_started(sink, id, name.as_str()).await;
            }
            StreamEvent::Text { text, .. } if !text.is_empty() => {
                self.text.push_str(&text);
                sink.emit(TurnEvent::Text(text)).await;
            }
            StreamEvent::Reasoning { text, .. } if !text.is_empty() => {
                sink.emit(TurnEvent::Reasoning(text)).await;
            }
            StreamEvent::End {
                part,
                content: AssistantContent::ToolCall(call),
            } => {
                if let Some(id) = self.started.remove(&part.index()) {
                    self.ended.insert(call.id, id);
                }
            }
            _ => {}
        }
    }

    async fn complete(
        mut self,
        response: rig::CompletionResponse,
        sink: &mut dyn EventSink,
    ) -> Result<AssistantTurn, ChatError> {
        let finish = match response.finish_reason() {
            Some(rig::FinishReason::Stop) => FinishReason::Stop,
            Some(rig::FinishReason::ToolCalls) => FinishReason::ToolCalls,
            Some(rig::FinishReason::Length) => FinishReason::Length,
            Some(rig::FinishReason::ContentFilter) => FinishReason::ContentFilter,
            // Chat Completions reports `error`; Bedrock `malformed_model_output` and
            // `malformed_tool_use`.
            Some(rig::FinishReason::Other(reason))
                if reason == "error" || reason.starts_with("malformed") =>
            {
                FinishReason::Error
            }
            Some(_) => FinishReason::Other,
            None => return Err(errors::disconnected()),
        };
        let mut calls = Vec::new();
        let mut replay = Vec::new();
        // A response cut off or filtered mid-call runs nothing; its cards are sealed "Not run".
        if finish == FinishReason::ToolCalls {
            for call in response.tool_calls() {
                if call.function.invalid_arguments.is_some() {
                    return Err(errors::protocol(
                        "A tool call arrived without complete JSON arguments.",
                    ));
                }
                let id = match self.ended.remove(&call.id) {
                    Some(id) => id,
                    None => {
                        let id = new_call_id();
                        tool_started(sink, &id, call.function.name.as_str()).await;
                        id
                    }
                };
                calls.push(ToolCall {
                    id: id.clone(),
                    name: call.function.name.as_str().into(),
                    input: call.function.arguments_value(),
                });
                replay.push(RigCall {
                    id,
                    call: call.id.clone(),
                    name: call.function.name.clone(),
                });
            }
        }
        let usage = response
            .usage
            .input_tokens
            .zip(response.usage.output_tokens)
            .map(|(input_tokens, output_tokens)| Usage {
                input_tokens,
                output_tokens,
            });
        let native = match response.message() {
            Some(Message::Assistant(message)) => Some(Box::new(Native {
                message,
                calls: replay,
            })),
            _ => None,
        };
        Ok(AssistantTurn {
            text: self.text,
            calls,
            finish,
            usage,
            native,
        })
    }
}

fn new_call_id() -> ToolCallId {
    ToolCallId(format!("call_{}", uuid::Uuid::new_v4().simple()))
}

async fn tool_started(sink: &mut dyn EventSink, id: &ToolCallId, name: &str) {
    sink.emit(TurnEvent::ToolStarted {
        id: id.clone(),
        name: name.into(),
        owner: ToolOwner::Server,
    })
    .await;
}

/// The loop's context as Rig messages, in the order every wire accepts:
///
/// - neighbouring user messages are one message, their texts joined by a blank line and each
///   picture after its own text (compaction drops questions independently, so they are joined
///   here, at the last moment);
/// - every tool result of a step follows its assistant message before any picture, and when
///   several calls shared the step each picture is named with its call ("Tool call <id>: ").
///
/// The loop places each picture message right after its own result; Rig then sends the results
/// as the wire's tool results and the pictures as the user message after them.
fn history(messages: &[ModelMessage]) -> Vec<Message> {
    // Server call id → the Rig call, for the assistant turns seen so far.
    let mut known: BTreeMap<&str, &RigCall> = BTreeMap::new();
    let mut out = Vec::new();
    let mut rest = messages;
    while let Some((message, tail)) = rest.split_first() {
        match message {
            ModelMessage::Assistant(turn) => {
                rest = tail;
                if let Some(native) = &turn.native {
                    known.extend(native.calls.iter().map(|call| (call.id.0.as_str(), call)));
                    out.push(Message::Assistant(native.message.clone()));
                } else if !turn.text.is_empty() {
                    out.push(Message::assistant(&turn.text));
                }
            }
            ModelMessage::ToolResult { .. } => {
                let (results, pictures, tail) = tool_step(rest);
                rest = tail;
                let labelled = results.len() > 1;
                let results: Vec<_> = results
                    .into_iter()
                    .filter_map(|(id, text)| {
                        // Rig drops a result that answers no call it was given; so does this.
                        let call = known.get(id.0.as_str())?;
                        Some(UserContent::tool_result(
                            call.call.clone(),
                            call.name.clone(),
                            vec![ToolResultContent::text(text)],
                        ))
                    })
                    .collect();
                if !results.is_empty() {
                    out.push(Message::User { content: results });
                }
                let pictures: Vec<_> = pictures
                    .into_iter()
                    .map(|(id, text, images)| {
                        let label = labelled.then(|| {
                            known
                                .get(id.0.as_str())
                                .map_or_else(|| id.0.clone(), |call| call.call.wire().into_owned())
                        });
                        (label, text, images)
                    })
                    .collect();
                if let Some(user) = user(pictures.iter().map(|(label, text, images)| {
                    (label.as_deref(), text.as_str(), images.as_slice())
                })) {
                    out.push(user);
                }
            }
            ModelMessage::User { .. } => {
                let run = rest
                    .iter()
                    .take_while(|message| matches!(message, ModelMessage::User { .. }))
                    .count();
                let (users, tail) = rest.split_at(run);
                rest = tail;
                if let Some(user) = user(users.iter().filter_map(|message| match message {
                    ModelMessage::User { text, images } => {
                        Some((None, text.as_str(), images.as_slice()))
                    }
                    _ => None,
                })) {
                    out.push(user);
                }
            }
        }
    }
    out
}

/// A step's results, each with the picture message the loop placed right after it.
type Results<'a> = Vec<(&'a ToolCallId, &'a str)>;
type Pictures<'a> = Vec<(&'a ToolCallId, &'a String, &'a Vec<Image>)>;

fn tool_step(messages: &[ModelMessage]) -> (Results<'_>, Pictures<'_>, &[ModelMessage]) {
    let mut results = Vec::new();
    let mut pictures = Vec::new();
    let mut rest = messages;
    while let Some((ModelMessage::ToolResult { id, text }, tail)) = rest.split_first() {
        results.push((id, text.as_str()));
        rest = tail;
        if let Some((ModelMessage::User { text, images }, tail)) = rest.split_first() {
            pictures.push((id, text, images));
            rest = tail;
        }
    }
    (results, pictures, rest)
}

/// One user message from consecutive parts: `(call label, text, pictures)`.
fn user<'a>(
    parts: impl IntoIterator<Item = (Option<&'a str>, &'a str, &'a [Image])>,
) -> Option<Message> {
    let mut content = Vec::new();
    let mut text = String::new();
    for (index, (label, part, images)) in parts.into_iter().enumerate() {
        if index > 0 {
            text.push_str("\n\n");
        }
        if let Some(label) = label {
            text.push_str("Tool call ");
            text.push_str(label);
            text.push_str(": ");
        }
        text.push_str(part);
        if !images.is_empty() {
            content.push(UserContent::text(std::mem::take(&mut text)));
            content.extend(images.iter().map(|image| {
                UserContent::image_raw(image.bytes.clone(), media_type(&image.media_type), None)
            }));
        }
    }
    if !text.is_empty() {
        content.push(UserContent::text(text));
    }
    (!content.is_empty()).then_some(Message::User { content })
}

/// Rig sniffs the type from the bytes when the media type is not one it names.
fn media_type(mime: &str) -> Option<ImageMediaType> {
    Some(match mime {
        "image/png" => ImageMediaType::PNG,
        "image/jpeg" | "image/jpg" => ImageMediaType::JPEG,
        "image/gif" => ImageMediaType::GIF,
        "image/webp" => ImageMediaType::WEBP,
        _ => return None,
    })
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

/// The row's custom headers, validated. Values are marked sensitive so error text is scrubbed
/// of them.
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
    /// `None` for earlier turns loaded from SQLite and for the scripted model. Boxed: Rig's
    /// message is large and most turns in a context are text.
    native: Option<Box<Native>>,
}

/// What a provider returned for its turn, kept for the next step of the same turn.
struct Native {
    /// Rig's assistant message: each block holds the provider item it was decoded from, so
    /// signatures, encrypted reasoning and provider extensions replay to the same model.
    message: AssistantMessage,
    /// The server's id of each call → Rig's, for the tool results that answer them.
    calls: Vec<RigCall>,
}

struct RigCall {
    id: ToolCallId,
    call: CallId,
    name: ToolName,
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
            native: None,
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
