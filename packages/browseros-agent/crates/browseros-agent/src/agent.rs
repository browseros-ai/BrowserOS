//! The agent loop: builds a merged tool set from its sources, renders the system prompt, then runs
//! turns as a step loop that emits `UiChunk`s exactly as `ai@7.0.114`'s `ToolLoopAgent` +
//! `createAgentUIStream` do for the TS server (`stream-text.ts`,
//! `stream-language-model-call.ts`, `execute-tools-from-stream.ts`, `to-ui-message-chunk.ts`).
//!
//! ```text
//!  run_turn ─spawn─► Turn::drive                                   chunks ─► mpsc ─► ChunkStream
//!    start{messageId}
//!    prompt copy: filter_valid → strip_reasoning → sanitize_for_toolset → wrap last user text
//!    ui_to_model ─► working transcript
//!    loop (≤ max_steps):
//!      normalize_for_model → compaction::prepare_step   (result replaces `working`: sticky)
//!      open model stream, retrying only before its first event        (no client output yet)
//!      start-step · text/reasoning/tool-input chunks · tool-input-available|error
//!      on finish(stop|tool-calls): run the step's tools concurrently,
//!        tool-output-* in completion order
//!      finish-step; append the step's assistant + tool messages to `working`
//!      continue iff every tool call got an output and there was at least one
//!    finish{finishReason}
//! ```
//!
//! Semantics worth knowing before changing anything here:
//! - **Tools start when the step's model stream finishes**, not when each call is parsed, and only
//!   for finish reason `stop`/`tool-calls` (`execute-tools-from-stream.ts`). A step cut off by
//!   `length` or an error never runs its browser side effects. This follows the SDK source.
//! - **Retries** (2, `retry-after-ms`/`retry-after` honoured within 0–60 s, else 2 s × 2ⁿ) happen
//!   only before a step's first event, so nothing a client saw is ever repeated. A failure there,
//!   retries exhausted, is the pre-stream shape: `error` and no `finish`.
//! - **Cancellation** is the run's token. Every wait selects on it (biased), the turn then emits
//!   `abort{"reason":"AbortError: Conversation stopped"}` and nothing else. Tools and the model
//!   get a child token; MCP sources forward it as `notifications/cancelled`. A dropped
//!   `ChunkStream` stops the turn the same way, silently.
//! - **Errors during a turn are chunks**, never stream errors; their `errorText` is the
//!   `ChatError` envelope JSON.

use crate::{
    compaction::{self, Budget, CompactionStage, StepUsage},
    llm::{
        self, ChatError, ChatErrorCode, LanguageModel, ModelError, ModelEvent, ModelFinishReason,
        ModelRequest, ModelStream, ProviderOptions, ToolDefinition,
    },
    messages::{
        self, AssistantPart, ModelMessage, ToolMessagePart, ToolResultOutput, ToolResultPart,
    },
    prompt,
    tools::{self, ToolCatalog, ToolKind, ToolOutput, ToolSource, ToolSpec},
};
use futures_util::{StreamExt, stream::FuturesUnordered};
use indexmap::IndexMap;
use serde_json::{Value, json};
use std::{
    collections::{BTreeSet, HashMap, HashSet},
    sync::Arc,
    time::Duration,
};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use ui_message_stream::{ChunkStream, FinishReason, Role, TextPart, UiChunk, UiMessage, UiPart};

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
    /// `start.messageId` of the assistant message (TS's own built-in turn persists `""`).
    pub message_id: String,
}

#[derive(Debug, thiserror::Error)]
pub enum AgentBuildError {
    #[error("Required MCP server \"{0}\" is unavailable.")]
    RequiredSourceUnavailable(String),
    #[error(transparent)]
    Source(#[from] tools::ToolSourceError),
}

/// `maxRetries` of the AI SDK's model-call retry.
const MAX_RETRIES: u32 = 2;
const INITIAL_RETRY_DELAY_MS: f64 = 2_000.0;
/// `getErrorMessage(DOMException('Conversation stopped', 'AbortError'))`, the reason TS's stop
/// route aborts with.
const ABORT_REASON: &str = "AbortError: Conversation stopped";
const NO_OUTPUT_MESSAGE: &str =
    "No output generated. The model stream ended without a finish chunk.";

/// A prepared agent for one conversation session: model, rendered system prompt, merged tool
/// set. Cheap to share; the engine caches one per conversation until its inputs change.
pub struct Agent {
    inner: Arc<Inner>,
}

/// Everything a turn reads; shared with the turn's task so `run_turn` can return immediately.
struct Inner {
    model: Arc<dyn LanguageModel>,
    info: ModelInfo,
    system: String,
    options: ProviderOptions,
    /// Merged tools in merge order (the order the model sees them in).
    tools: IndexMap<String, RegisteredTool>,
    definitions: Vec<ToolDefinition>,
    catalog: ToolCatalog,
    tool_names: BTreeSet<String>,
    budget: Budget,
    max_steps: u32,
    /// Every source that connected, closed once by `dispose`.
    sources: Vec<Arc<dyn ToolSource>>,
}

struct RegisteredTool {
    spec: ToolSpec,
    source: Arc<dyn ToolSource>,
}

impl Agent {
    /// Connects every source (in parallel; a required failure aborts and closes the rest),
    /// merges tools, renders the prompt.
    pub async fn build(config: AgentConfig) -> Result<Agent, AgentBuildError> {
        let AgentConfig {
            model,
            info,
            reasoning,
            prompt: prompt_inputs,
            sources,
            max_steps,
        } = config;

        let listed =
            futures_util::future::join_all(sources.iter().map(|spec| spec.source.tools())).await;
        let mut connected = Vec::new();
        let mut failed_required = None;
        for (spec, result) in sources.iter().zip(listed) {
            match result {
                Ok(specs) => connected.push((spec, specs)),
                Err(error) if spec.required => {
                    tracing::warn!(source = spec.source.label(), %error, "Failed to connect required tool source");
                    failed_required.get_or_insert_with(|| spec.source.label().to_owned());
                }
                Err(error) => {
                    tracing::warn!(source = spec.source.label(), %error, "Failed to connect tool source, skipping");
                    spec.source.close().await;
                }
            }
        }
        // Until the agent exists this function owns every source; a required failure must not
        // leak the ones that did connect.
        if let Some(label) = failed_required {
            for (spec, _) in &connected {
                spec.source.close().await;
            }
            return Err(AgentBuildError::RequiredSourceUnavailable(label));
        }

        let reserved: HashSet<String> = connected
            .iter()
            .filter(|(spec, _)| spec.required)
            .flat_map(|(_, specs)| specs.iter().map(|tool| tool.name.clone()))
            .collect();
        let mut tools: IndexMap<String, RegisteredTool> = IndexMap::new();
        let mut skipped = Vec::new();
        for (spec, specs) in &connected {
            for tool in specs {
                if !spec.required && reserved.contains(&tool.name) {
                    skipped.push(tool.name.clone());
                    continue;
                }
                // Like a JS object spread, a later source replaces the tool but keeps its slot.
                tools.insert(
                    tool.name.clone(),
                    RegisteredTool {
                        spec: tool.clone(),
                        source: Arc::clone(&spec.source),
                    },
                );
            }
        }
        if !skipped.is_empty() {
            tracing::warn!(tool_names = ?skipped, "External MCP tools skipped due to BrowserOS tool name collision");
        }
        if prompt_inputs.is_scheduled_task || prompt_inputs.chat_mode {
            tools.shift_remove(tools::SUGGEST_SCHEDULE);
            tools.shift_remove(tools::SUGGEST_APP_CONNECTION);
        }

        let tool_names: BTreeSet<String> = tools.keys().cloned().collect();
        let specs: Vec<ToolSpec> = tools.values().map(|tool| tool.spec.clone()).collect();
        let definitions = specs
            .iter()
            .map(|spec| ToolDefinition {
                name: spec.name.clone(),
                description: spec.description.clone(),
                input_schema: spec.input_schema.clone(),
            })
            .collect();
        let system = prompt::system_prompt(&prompt_inputs, &tool_names);
        let options = llm::provider_options(info.provider, &info.model_id, &reasoning);
        tracing::info!(
            provider = info.provider.as_str(),
            model = %info.model_id,
            tool_count = tools.len(),
            "Agent session created"
        );

        Ok(Agent {
            inner: Arc::new(Inner {
                model,
                budget: Budget::for_context_window(info.context_window),
                info,
                system,
                options,
                catalog: ToolCatalog::new(&specs),
                tools,
                definitions,
                tool_names,
                max_steps: max_steps.max(1),
                sources: connected
                    .into_iter()
                    .map(|(spec, _)| Arc::clone(&spec.source))
                    .collect(),
            }),
        })
    }

    pub fn tool_names(&self) -> &BTreeSet<String> {
        &self.inner.tool_names
    }

    /// Runs one turn. Never errors: failures are `error` chunks; cancel → `abort` chunk then end.
    /// The turn runs on its own task and stops if the returned stream is dropped.
    pub fn run_turn(&self, input: TurnInput, cancel: CancellationToken) -> ChunkStream {
        let (tx, rx) = mpsc::channel(64);
        let turn = Turn {
            inner: Arc::clone(&self.inner),
            stop: cancel.child_token(),
            cancel,
            tx,
            text_ids: PartIds::default(),
            reasoning_ids: PartIds::default(),
        };
        tokio::spawn(turn.run(input));
        Box::pin(futures_util::stream::unfold(rx, |mut rx| async move {
            rx.recv().await.map(|chunk| (chunk, rx))
        }))
    }

    /// Closes MCP clients. Must run when the engine evicts a session, or loopback MCP sessions leak.
    pub async fn dispose(self) {
        for source in &self.inner.sources {
            source.close().await;
        }
    }
}

/// Why a turn ended early: the run was stopped (emit `abort`), or nobody reads the stream.
enum Halt {
    Cancelled,
    Disconnected,
}

type Flow<T = ()> = Result<T, Halt>;

/// One running turn. Owns the chunk sender; `stop` is the token handed to the model and tools, a
/// child of the run's `cancel`, also cancelled when the turn ends for any other reason.
struct Turn {
    inner: Arc<Inner>,
    cancel: CancellationToken,
    stop: CancellationToken,
    tx: mpsc::Sender<UiChunk>,
    text_ids: PartIds,
    reasoning_ids: PartIds,
}

/// A parsed, valid tool call waiting for the step's model stream to finish.
struct PendingCall {
    ui_id: String,
    transcript_id: String,
    tool_name: String,
    input: Value,
    provider_metadata: Option<Value>,
}

/// What a step contributes once its stream is done.
enum StepOutcome {
    /// The turn is over and its last chunk was already sent (the no-output error).
    Ended,
    Finished {
        reason: FinishReason,
        usage: StepUsage,
        messages: Vec<ModelMessage>,
        continue_loop: bool,
    },
}

impl Turn {
    async fn run(mut self, input: TurnInput) {
        let outcome = self.drive(input).await;
        // Whatever ended the turn, nothing it started may keep running.
        self.stop.cancel();
        if let Err(Halt::Cancelled) = outcome {
            let _ = self
                .tx
                .send(UiChunk::Abort {
                    reason: Some(ABORT_REASON.to_owned()),
                })
                .await;
        }
    }

    async fn emit(&self, chunk: UiChunk) -> Flow {
        self.tx.send(chunk).await.map_err(|_| Halt::Disconnected)
    }

    /// Resolves when the turn must stop: the run was stopped or the stream was dropped.
    async fn halted(&self) -> Halt {
        tokio::select! {
            biased;
            () = self.cancel.cancelled() => Halt::Cancelled,
            () = self.tx.closed() => Halt::Disconnected,
        }
    }

    async fn drive(&mut self, input: TurnInput) -> Flow {
        self.emit(UiChunk::Start {
            message_metadata: None,
            message_id: Some(input.message_id.clone()),
        })
        .await?;
        if self.cancel.is_cancelled() {
            return Err(Halt::Cancelled);
        }

        let prompt_messages = self.prompt_copy(input);
        let mut working = match messages::ui_to_model(&prompt_messages, &self.inner.catalog) {
            Ok(converted) => converted,
            Err(error) => {
                tracing::error!(%error, "Failed to convert history for the model");
                return self
                    .emit(UiChunk::Error {
                        error_text: self.message_error_text(&error.0),
                    })
                    .await;
            }
        };

        let mut usages: Vec<StepUsage> = Vec::new();
        let mut step = 0;
        loop {
            step += 1;
            let normalized = messages::normalize_for_model(
                std::mem::take(&mut working),
                self.inner.model.supports_media_in_tool_results(),
                self.inner.info.supports_images,
            );
            let (prepared, stage) =
                compaction::prepare_step(normalized, &usages, &self.inner.budget);
            if stage != CompactionStage::Untouched {
                tracing::info!(step, ?stage, "Compacted the working transcript");
            }
            working = prepared;

            let request = ModelRequest {
                system: Some(self.inner.system.clone()),
                messages: working.clone(),
                tools: self.inner.definitions.clone(),
                options: self.inner.options.clone(),
            };
            let (stream, first) = match self.open_step(request).await? {
                Ok(opened) => opened,
                Err(error) => {
                    if self.cancel.is_cancelled() {
                        return Err(Halt::Cancelled);
                    }
                    return self
                        .emit(UiChunk::Error {
                            error_text: self.model_error_text(&error),
                        })
                        .await;
                }
            };

            match self.stream_step(stream, first).await? {
                StepOutcome::Ended => return Ok(()),
                StepOutcome::Finished {
                    reason,
                    usage,
                    messages,
                    continue_loop,
                } => {
                    usages.push(usage);
                    working.extend(messages);
                    if continue_loop && step < self.inner.max_steps {
                        continue;
                    }
                    return self
                        .emit(UiChunk::Finish {
                            finish_reason: Some(reason),
                            message_metadata: None,
                        })
                        .await;
                }
            }
        }
    }

    /// The model-facing copy of the display history (`chat-service.ts:417-441`): invalid messages
    /// dropped, reasoning stripped, tool parts for tools this agent lacks removed, and this turn's
    /// user message replaced by `prompt_text`. A blank user message is filtered out first and so
    /// is never replaced, as in TS.
    fn prompt_copy(&self, input: TurnInput) -> Vec<UiMessage> {
        let wrapped_id = input
            .history
            .last()
            .filter(|message| message.role == Role::User)
            .map(|message| message.id.clone());
        let copy = messages::sanitize_for_toolset(
            messages::strip_reasoning(messages::filter_valid(input.history)),
            &self.inner.tool_names,
        );
        copy.into_iter()
            .map(|mut message| {
                if message.role == Role::User && wrapped_id.as_deref() == Some(message.id.as_str())
                {
                    message.parts = vec![UiPart::Text(TextPart {
                        text: input.prompt_text.clone(),
                        state: None,
                        provider_metadata: None,
                    })];
                }
                message
            })
            .collect()
    }

    /// Opens the step's model stream and reads its first event, retrying failures that happen
    /// before any event. The outer `Result` is the halt; the inner one the final model error.
    async fn open_step(
        &self,
        request: ModelRequest,
    ) -> Flow<Result<(ModelStream, Option<ModelEvent>), ModelError>> {
        let mut attempt = 0;
        loop {
            let opened = tokio::select! {
                biased;
                halt = self.halted() => return Err(halt),
                opened = self.inner.model.stream(request.clone(), self.stop.clone()) => opened,
            };
            let error = match opened {
                Ok(mut stream) => {
                    let first = tokio::select! {
                        biased;
                        halt = self.halted() => return Err(halt),
                        first = stream.next() => first,
                    };
                    match first {
                        Some(Ok(event)) => return Ok(Ok((stream, Some(event)))),
                        None => return Ok(Ok((stream, None))),
                        Some(Err(error)) => error,
                    }
                }
                Err(error) => error,
            };
            if attempt >= MAX_RETRIES || !is_retryable(&error) {
                return Ok(Err(error));
            }
            let delay = retry_delay(&error, attempt);
            attempt += 1;
            tracing::warn!(attempt, delay_ms = delay.as_millis() as u64, %error, "Retrying model call");
            tokio::select! {
                biased;
                halt = self.halted() => return Err(halt),
                () = tokio::time::sleep(delay) => {}
            }
        }
    }

    /// One model step: forwards its events as chunks, validates tool calls, runs them after the
    /// model finishes, and returns the messages the step adds to the transcript.
    async fn stream_step(
        &mut self,
        mut stream: ModelStream,
        first: Option<ModelEvent>,
    ) -> Flow<StepOutcome> {
        let mut pending_first = first.map(Ok);
        let mut started = false;
        let mut has_output = false;
        let mut errored = false;
        let mut finish = None;
        let mut content: Vec<AssistantPart> = Vec::new();
        // Provider part id → (stream-unique id, index into `content`).
        let mut open_text: HashMap<String, (String, usize)> = HashMap::new();
        let mut open_reasoning: HashMap<String, (String, usize)> = HashMap::new();
        // Provider part id → index into `content` of the latest ended part, for late metadata.
        let mut ended_parts: HashMap<String, usize> = HashMap::new();
        let mut pending: Vec<PendingCall> = Vec::new();
        // Every tool call of the step (valid or not), in call order: transcript id → order.
        let mut call_order: Vec<String> = Vec::new();
        let mut results: Vec<ToolResultPart> = Vec::new();

        loop {
            let item = match pending_first.take() {
                Some(item) => Some(item),
                None => tokio::select! {
                    biased;
                    halt = self.halted() => return Err(halt),
                    item = stream.next() => item,
                },
            };
            let Some(item) = item else { break };
            if !started {
                started = true;
                self.emit(UiChunk::StartStep).await?;
            }
            let event = match item {
                Ok(event) => event,
                Err(error) => {
                    if self.cancel.is_cancelled() {
                        return Err(Halt::Cancelled);
                    }
                    self.emit(UiChunk::Error {
                        error_text: self.model_error_text(&error),
                    })
                    .await?;
                    // The SDK provider's `flush` still runs after a stream error: reasoning, then
                    // text, end, so the persisted parts read `done` rather than `streaming`.
                    for open in [&mut open_reasoning, &mut open_text] {
                        let mut ids: Vec<(usize, String)> = open
                            .drain()
                            .map(|(_, (stream_id, index))| (index, stream_id))
                            .collect();
                        ids.sort_unstable();
                        for (index, id) in ids {
                            let chunk = match content.get(index) {
                                Some(AssistantPart::Reasoning { .. }) => UiChunk::ReasoningEnd {
                                    id,
                                    provider_metadata: None,
                                },
                                _ => UiChunk::TextEnd {
                                    id,
                                    provider_metadata: None,
                                },
                            };
                            self.emit(chunk).await?;
                        }
                    }
                    errored = true;
                    break;
                }
            };
            match event {
                ModelEvent::TextStart {
                    id,
                    provider_metadata,
                } => {
                    let stream_id = self.text_ids.reserve(&id);
                    content.push(AssistantPart::Text {
                        text: String::new(),
                        provider_options: provider_metadata.clone(),
                    });
                    open_text.insert(id, (stream_id.clone(), content.len() - 1));
                    self.emit(UiChunk::TextStart {
                        id: stream_id,
                        provider_metadata: ui_metadata(provider_metadata),
                    })
                    .await?;
                }
                ModelEvent::TextDelta { id, delta } => {
                    if delta.is_empty() {
                        continue;
                    }
                    has_output = true;
                    let stream_id = match open_text.get(&id) {
                        Some((stream_id, index)) => {
                            if let Some(AssistantPart::Text { text, .. }) = content.get_mut(*index)
                            {
                                text.push_str(&delta);
                            }
                            stream_id.clone()
                        }
                        None => id,
                    };
                    self.emit(UiChunk::TextDelta {
                        id: stream_id,
                        delta,
                        provider_metadata: None,
                    })
                    .await?;
                }
                ModelEvent::TextEnd {
                    id,
                    provider_metadata,
                } => {
                    let stream_id = match open_text.remove(&id) {
                        Some((stream_id, index)) => {
                            if let (
                                Some(metadata),
                                Some(AssistantPart::Text {
                                    provider_options, ..
                                }),
                            ) = (&provider_metadata, content.get_mut(index))
                            {
                                *provider_options = Some(metadata.clone());
                            }
                            ended_parts.insert(id, index);
                            stream_id
                        }
                        None => id,
                    };
                    self.emit(UiChunk::TextEnd {
                        id: stream_id,
                        provider_metadata: ui_metadata(provider_metadata),
                    })
                    .await?;
                }
                ModelEvent::ReasoningStart {
                    id,
                    provider_metadata,
                } => {
                    let stream_id = self.reasoning_ids.reserve(&id);
                    content.push(AssistantPart::Reasoning {
                        text: String::new(),
                        provider_options: provider_metadata.clone(),
                    });
                    open_reasoning.insert(id, (stream_id.clone(), content.len() - 1));
                    self.emit(UiChunk::ReasoningStart {
                        id: stream_id,
                        provider_metadata: ui_metadata(provider_metadata),
                    })
                    .await?;
                }
                ModelEvent::ReasoningDelta {
                    id,
                    delta,
                    provider_metadata,
                } => {
                    has_output |= !delta.is_empty();
                    let stream_id = match open_reasoning.get(&id) {
                        Some((stream_id, index)) => {
                            if let Some(AssistantPart::Reasoning {
                                text,
                                provider_options,
                            }) = content.get_mut(*index)
                            {
                                text.push_str(&delta);
                                if provider_metadata.is_some() {
                                    provider_options.clone_from(&provider_metadata);
                                }
                            }
                            stream_id.clone()
                        }
                        None => id,
                    };
                    self.emit(UiChunk::ReasoningDelta {
                        id: stream_id,
                        delta,
                        provider_metadata: ui_metadata(provider_metadata),
                    })
                    .await?;
                }
                ModelEvent::ReasoningEnd {
                    id,
                    provider_metadata,
                } => {
                    let stream_id = match open_reasoning.remove(&id) {
                        Some((stream_id, index)) => {
                            if let (
                                Some(metadata),
                                Some(AssistantPart::Reasoning {
                                    provider_options, ..
                                }),
                            ) = (&provider_metadata, content.get_mut(index))
                            {
                                *provider_options = Some(metadata.clone());
                            }
                            ended_parts.insert(id, index);
                            stream_id
                        }
                        None => id,
                    };
                    self.emit(UiChunk::ReasoningEnd {
                        id: stream_id,
                        provider_metadata: ui_metadata(provider_metadata),
                    })
                    .await?;
                }
                ModelEvent::PartMetadata {
                    id,
                    provider_metadata,
                } => {
                    // The adapter ended this part early (AI SDK lifecycle); its native item came
                    // with the provider's own end. Transcript only: the UI part is already done.
                    if let Some(index) = ended_parts.get(&id)
                        && let Some(
                            AssistantPart::Text {
                                provider_options, ..
                            }
                            | AssistantPart::Reasoning {
                                provider_options, ..
                            },
                        ) = content.get_mut(*index)
                    {
                        *provider_options = Some(provider_metadata);
                    }
                }
                ModelEvent::ToolInputStart { id, tool_name } => {
                    let (tool_metadata, dynamic, title) = match self.inner.tools.get(&tool_name) {
                        Some(tool) => wire_tool_fields(&tool.spec.kind),
                        // `tool-input-start` for an unknown tool says `dynamic: false`.
                        None => (None, Some(false), None),
                    };
                    self.emit(UiChunk::ToolInputStart {
                        tool_call_id: id,
                        tool_name,
                        provider_executed: None,
                        provider_metadata: None,
                        tool_metadata,
                        dynamic,
                        title,
                    })
                    .await?;
                }
                ModelEvent::ToolInputDelta { id, delta } => {
                    has_output |= !delta.is_empty();
                    self.emit(UiChunk::ToolInputDelta {
                        tool_call_id: id,
                        input_text_delta: delta,
                    })
                    .await?;
                }
                ModelEvent::ToolCall {
                    id,
                    provider_call_id,
                    tool_name,
                    input_json,
                    provider_metadata,
                } => {
                    has_output = true;
                    let transcript_id = if provider_call_id.is_empty() {
                        id.clone()
                    } else {
                        provider_call_id
                    };
                    call_order.push(transcript_id.clone());
                    match self.parse_tool_call(&tool_name, &input_json) {
                        Ok(input) => {
                            let (tool_metadata, dynamic, title) = self
                                .inner
                                .tools
                                .get(&tool_name)
                                .map(|tool| wire_tool_fields(&tool.spec.kind))
                                .unwrap_or_default();
                            content.push(AssistantPart::ToolCall {
                                tool_call_id: transcript_id.clone(),
                                tool_name: tool_name.clone(),
                                input: Some(input.clone()),
                                provider_executed: None,
                                provider_options: provider_metadata.clone(),
                            });
                            self.emit(UiChunk::ToolInputAvailable {
                                tool_call_id: id.clone(),
                                tool_name: tool_name.clone(),
                                input: input.clone(),
                                provider_executed: None,
                                provider_metadata: ui_metadata(provider_metadata.clone()),
                                tool_metadata,
                                dynamic,
                                title,
                            })
                            .await?;
                            pending.push(PendingCall {
                                ui_id: id,
                                transcript_id,
                                tool_name,
                                input,
                                provider_metadata,
                            });
                        }
                        Err(invalid) => {
                            let known = self.inner.tools.get(&tool_name);
                            let (tool_metadata, dynamic, title) = match known {
                                Some(tool) => wire_tool_fields(&tool.spec.kind),
                                None => (None, Some(true), None),
                            };
                            content.push(AssistantPart::ToolCall {
                                tool_call_id: transcript_id.clone(),
                                tool_name: tool_name.clone(),
                                input: Some(if invalid.input.is_object() {
                                    invalid.input.clone()
                                } else {
                                    json!({})
                                }),
                                provider_executed: None,
                                provider_options: provider_metadata.clone(),
                            });
                            self.emit(UiChunk::ToolInputError {
                                tool_call_id: id.clone(),
                                tool_name: tool_name.clone(),
                                input: invalid.input,
                                provider_executed: None,
                                provider_metadata: ui_metadata(provider_metadata),
                                tool_metadata: tool_metadata.clone(),
                                dynamic,
                                error_text: self.message_error_text(&invalid.message),
                                title,
                            })
                            .await?;
                            // The SDK fails an invalid call at once, with the error's
                            // `toString()` (`AI_NoSuchToolError: …`) as its output.
                            let failure = format!("{}: {}", invalid.error_name, invalid.message);
                            self.emit(UiChunk::ToolOutputError {
                                tool_call_id: id,
                                error_text: self.message_error_text(&failure),
                                provider_executed: None,
                                provider_metadata: None,
                                tool_metadata,
                                dynamic,
                            })
                            .await?;
                            results.push(ToolResultPart {
                                tool_call_id: transcript_id,
                                tool_name,
                                output: ToolResultOutput::ErrorText { value: failure },
                                provider_options: None,
                            });
                        }
                    }
                }
                ModelEvent::Finish { reason, usage } => {
                    finish = Some((reason, usage));
                }
            }
        }

        if !errored && finish.is_none() && !has_output {
            self.emit(UiChunk::Error {
                error_text: self.message_error_text(NO_OUTPUT_MESSAGE),
            })
            .await?;
            return Ok(StepOutcome::Ended);
        }

        let tools_may_run = !errored
            && finish.is_some_and(|(reason, _)| {
                matches!(
                    reason,
                    ModelFinishReason::Stop | ModelFinishReason::ToolCalls
                )
            });
        if tools_may_run {
            results.extend(self.run_tools(pending).await?);
        }
        self.emit(UiChunk::FinishStep).await?;

        let reason = if errored {
            FinishReason::Error
        } else {
            finish.map_or(FinishReason::Other, |(reason, _)| ui_finish_reason(reason))
        };
        let usage = finish.map_or_else(StepUsage::default, |(_, usage)| StepUsage {
            input_tokens: usage.input_tokens,
            output_tokens: usage.output_tokens,
        });
        // Continue only when every call (valid or not) has an output: calls left unexecuted
        // because the model stopped for another reason end the turn.
        let continue_loop = !call_order.is_empty() && call_order.len() == results.len();

        Ok(StepOutcome::Finished {
            reason,
            usage,
            messages: step_messages(content, results, &call_order),
            continue_loop,
        })
    }

    /// Runs the step's tools concurrently and emits each output as it completes. Each call is
    /// polled once in call order first, as JS starts every `execute` before awaiting any: a tool
    /// that answers at once (nudges, local checks) reports in call order, deterministically, like
    /// TS. Only a call still pending moves to its own task with a child of `stop`, so a stopped
    /// turn leaves nothing executing, and an MCP call still gets to send `notifications/cancelled`.
    async fn run_tools(&self, calls: Vec<PendingCall>) -> Flow<Vec<ToolResultPart>> {
        type Called = Result<ToolOutput, tools::ToolCallError>;
        let mut running = FuturesUnordered::new();
        for (index, call) in calls.iter().enumerate() {
            let tool = self
                .inner
                .tools
                .get(&call.tool_name)
                .map(|tool| Arc::clone(&tool.source));
            let name = call.tool_name.clone();
            let input = call.input.clone();
            let token = self.stop.child_token();
            let mut execution: futures_util::future::BoxFuture<'static, Called> =
                Box::pin(async move {
                    match tool {
                        Some(source) => source.call(&name, input, token).await,
                        None => Err(tools::ToolCallError(format!("Unknown tool: {name}"))),
                    }
                });
            match futures_util::FutureExt::now_or_never(&mut execution) {
                Some(done) => running.push(futures_util::future::Either::Left(
                    futures_util::future::ready((
                        index,
                        Ok::<Called, tokio::task::JoinError>(done),
                    )),
                )),
                None => {
                    let task = tokio::spawn(execution);
                    running.push(futures_util::future::Either::Right(async move {
                        (index, task.await)
                    }));
                }
            }
        }

        let mut outputs: Vec<Option<ToolResultPart>> = vec![None; calls.len()];
        while !running.is_empty() {
            let (index, joined) = tokio::select! {
                biased;
                halt = self.halted() => return Err(halt),
                Some(done) = running.next() => done,
            };
            let call = &calls[index];
            let result = joined.unwrap_or_else(|error| {
                Err(tools::ToolCallError(format!(
                    "Tool execution failed: {error}"
                )))
            });
            let (tool_metadata, dynamic, _) = self
                .inner
                .tools
                .get(&call.tool_name)
                .map(|tool| wire_tool_fields(&tool.spec.kind))
                .unwrap_or_default();
            let model_output = match result {
                Ok(ToolOutput { ui, model }) => {
                    self.emit(UiChunk::ToolOutputAvailable {
                        tool_call_id: call.ui_id.clone(),
                        output: ui,
                        provider_executed: None,
                        provider_metadata: ui_metadata(call.provider_metadata.clone()),
                        tool_metadata,
                        preliminary: None,
                        dynamic,
                    })
                    .await?;
                    model
                }
                Err(error) => {
                    tracing::info!(tool = %call.tool_name, %error, "Tool call failed");
                    self.emit(UiChunk::ToolOutputError {
                        tool_call_id: call.ui_id.clone(),
                        error_text: self.message_error_text(&error.0),
                        provider_executed: None,
                        provider_metadata: ui_metadata(call.provider_metadata.clone()),
                        tool_metadata,
                        dynamic,
                    })
                    .await?;
                    ToolResultOutput::ErrorText { value: error.0 }
                }
            };
            outputs[index] = Some(ToolResultPart {
                tool_call_id: call.transcript_id.clone(),
                tool_name: call.tool_name.clone(),
                output: model_output,
                provider_options: call.provider_metadata.clone(),
            });
        }
        Ok(outputs.into_iter().flatten().collect())
    }

    /// `parseToolCall`: an empty input is `{}`; otherwise JSON. Static tools are then validated
    /// against their schema; MCP tools take any JSON (`jsonSchema()` has no validator).
    fn parse_tool_call(&self, tool_name: &str, input_json: &str) -> Result<Value, InvalidCall> {
        let parsed = if input_json.trim().is_empty() {
            Ok(json!({}))
        } else {
            serde_json::from_str::<Value>(input_json).map_err(|error| {
                format!(
                    "AI_JSONParseError: JSON parsing failed: Text: {input_json}.\nError message: SyntaxError: {error}"
                )
            })
        };
        let Some(tool) = self.inner.tools.get(tool_name) else {
            let available = self.inner.tools.keys().cloned().collect::<Vec<_>>();
            let message = if available.is_empty() {
                format!(
                    "Model tried to call unavailable tool '{tool_name}'. No tools are available."
                )
            } else {
                format!(
                    "Model tried to call unavailable tool '{tool_name}'. Available tools: {}.",
                    available.join(", ")
                )
            };
            return Err(InvalidCall {
                input: parsed.unwrap_or_else(|_| Value::from(input_json)),
                error_name: "AI_NoSuchToolError",
                message,
            });
        };
        let invalid_input = |cause: String, input: Value| InvalidCall {
            input,
            error_name: "AI_InvalidToolInputError",
            message: format!("Invalid input for tool {tool_name}: {cause}"),
        };
        let parsed = parsed.map_err(|cause| invalid_input(cause, Value::from(input_json)))?;
        match tool.spec.kind {
            ToolKind::Dynamic { .. } => Ok(parsed),
            ToolKind::Static => {
                tools::validate_static_input(&tool.spec.input_schema, parsed.clone())
                    .map_err(|cause| invalid_input(cause, parsed))
            }
        }
    }

    fn model_error_text(&self, error: &ModelError) -> String {
        llm::chat_error_text(&llm::to_chat_error(error, Some(self.inner.info.provider)))
    }

    fn message_error_text(&self, message: &str) -> String {
        llm::chat_error_text(&message_chat_error(
            message,
            self.inner.info.provider.as_str(),
        ))
    }
}

/// A tool call the SDK rejects before execution (`invalid: true`).
struct InvalidCall {
    /// The parsed input, or the raw text when it is not JSON.
    input: Value,
    error_name: &'static str,
    message: String,
}

/// `toolMetadata`, `dynamic` and `title` of a known tool's chunks: MCP tools carry all three,
/// static tools none (`isDynamic` yields `undefined` for them).
fn wire_tool_fields(kind: &ToolKind) -> (Option<Value>, Option<bool>, Option<String>) {
    match kind {
        ToolKind::Static => (None, None, None),
        ToolKind::Dynamic { title, metadata } => {
            (Some(metadata.clone()), Some(true), title.clone())
        }
    }
}

/// `toResponseMessages`: the assistant message (empty text parts skipped) and a tool message with
/// one result per call, sorted by call order.
fn step_messages(
    content: Vec<AssistantPart>,
    mut results: Vec<ToolResultPart>,
    call_order: &[String],
) -> Vec<ModelMessage> {
    let content: Vec<AssistantPart> = content
        .into_iter()
        .filter(|part| !matches!(part, AssistantPart::Text { text, .. } if text.is_empty()))
        .collect();
    let mut messages = Vec::new();
    if !content.is_empty() {
        messages.push(ModelMessage::Assistant { content });
    }
    let order = |id: &str| {
        call_order
            .iter()
            .position(|call| call == id)
            .unwrap_or(usize::MAX)
    };
    results.sort_by_key(|part| order(&part.tool_call_id));
    if !results.is_empty() {
        messages.push(ModelMessage::Tool {
            content: results
                .into_iter()
                .map(ToolMessagePart::ToolResult)
                .collect(),
        });
    }
    messages
}

fn ui_finish_reason(reason: ModelFinishReason) -> FinishReason {
    match reason {
        ModelFinishReason::Stop => FinishReason::Stop,
        ModelFinishReason::Length => FinishReason::Length,
        ModelFinishReason::ContentFilter => FinishReason::ContentFilter,
        ModelFinishReason::ToolCalls => FinishReason::ToolCalls,
        ModelFinishReason::Error => FinishReason::Error,
        ModelFinishReason::Other => FinishReason::Other,
    }
}

/// The SDK retries `APICallError`s marked retryable; network failures are wrapped as retryable
/// API call errors by its fetch layer.
fn is_retryable(error: &ModelError) -> bool {
    match error {
        ModelError::Api { retryable, .. } => *retryable,
        ModelError::Connection(_) => true,
        ModelError::InvalidPrompt(_)
        | ModelError::Config(_)
        | ModelError::Other(_)
        | ModelError::Cancelled => false,
    }
}

/// `getRetryDelayInMs`: `retry-after-ms`, else numeric `retry-after` seconds, used when within
/// 0–60 s (or shorter than the backoff); otherwise 2 s × 2^attempt. An HTTP-date `retry-after`
/// falls back to the backoff.
fn retry_delay(error: &ModelError, attempt: u32) -> Duration {
    let backoff = INITIAL_RETRY_DELAY_MS * 2f64.powi(attempt as i32);
    let header = |name: &str| match error {
        ModelError::Api {
            response_headers, ..
        } => response_headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .and_then(|(_, value)| value.trim().parse::<f64>().ok())
            .filter(|value| !value.is_nan()),
        _ => None,
    };
    let hinted = header("retry-after-ms").or_else(|| header("retry-after").map(|s| s * 1000.0));
    let ms = match hinted {
        Some(ms) if ms >= 0.0 && (ms < 60_000.0 || ms < backoff) => ms,
        _ => backoff,
    };
    Duration::from_secs_f64(ms / 1000.0)
}

/// `toChatError` for an error that is only a message (tool failures, invalid tool calls, the
/// no-output error): the message rungs of `chat-error.ts` `fromMessage`, else `unknown`.
fn message_chat_error(message: &str, provider: &str) -> ChatError {
    const CONNECTION_DOCS_URL: &str =
        "https://docs.browseros.com/troubleshooting/connection-issues";
    let lower = message.to_lowercase();
    let has = |needles: &[&str]| needles.iter().any(|needle| lower.contains(needle));
    let base = |code, title: &str, text: String, retryable| ChatError {
        code,
        title: title.to_owned(),
        message: text,
        retryable,
        provider: Some(provider.to_owned()),
        status_code: None,
        docs_url: None,
        retry_after_seconds: None,
        details: to_details(message),
    };
    if has(&["failed to fetch", "fetch failed"]) {
        return ChatError {
            docs_url: Some(CONNECTION_DOCS_URL.to_owned()),
            ..base(
                ChatErrorCode::ConnectionFailed,
                "Connection failed",
                "Could not reach the model provider. Check your network connection and any local provider URL.".to_owned(),
                true,
            )
        };
    }
    if has(&[
        "context length",
        "context_length_exceeded",
        "too many tokens",
        "maximum context",
    ]) {
        return base(
            ChatErrorCode::ContextLength,
            "Conversation too long",
            "This conversation exceeded the model context window. Start a new chat or pick a model with a larger window.".to_owned(),
            false,
        );
    }
    if has(&["content filter", "content_filter", "content policy"]) {
        return base(
            ChatErrorCode::ContentFilter,
            "Content was rejected",
            "The provider rejected the content being sent or generated. Try rephrasing and send it again.".to_owned(),
            false,
        );
    }
    if has(&[
        "requires apikey",
        "requires baseurl",
        "requires oauth",
        "not authenticated with",
        "is required for",
    ]) {
        return base(
            ChatErrorCode::ProviderConfig,
            "Provider not configured",
            safe_message(message, "This provider is not fully configured."),
            false,
        );
    }
    base(
        ChatErrorCode::Unknown,
        "Something went wrong",
        safe_message(message, "An unexpected error occurred."),
        true,
    )
}

fn safe_message(text: &str, fallback: &str) -> String {
    let redacted = llm::redact(text);
    let trimmed = redacted.trim();
    if trimmed.is_empty() {
        fallback.to_owned()
    } else {
        trimmed.to_owned()
    }
}

/// `toDetails`: redacted, trimmed, capped at 20 000 UTF-16 units with `…`.
fn to_details(text: &str) -> Option<String> {
    const DETAILS_MAX_LENGTH: usize = 20_000;
    let redacted = llm::redact(text);
    let trimmed = redacted.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.encode_utf16().count() <= DETAILS_MAX_LENGTH {
        return Some(trimmed.to_owned());
    }
    let mut units = 0;
    let cut: String = trimmed
        .chars()
        .take_while(|ch| {
            units += ch.len_utf16();
            units <= DETAILS_MAX_LENGTH
        })
        .collect();
    Some(format!("{cut}…"))
}

/// The SDK's stream-unique part ids: a provider id is used as is the first time it appears in a
/// turn, and replaced by a fresh random id when a later step reuses it (providers often number
/// parts per response).
#[derive(Default)]
struct PartIds {
    used: HashSet<String>,
}

impl PartIds {
    fn reserve(&mut self, id: &str) -> String {
        if self.used.insert(id.to_owned()) {
            return id.to_owned();
        }
        let generated = generate_id();
        let mut unique = generated.clone();
        let mut suffix = 0;
        while self.used.contains(&unique) {
            suffix += 1;
            unique = format!("{generated}-{suffix}");
        }
        self.used.insert(unique.clone());
        unique
    }
}

/// The provider metadata a UI chunk may carry: everything but the adapter's `rig` envelope (the
/// native item it replays on the next step lives in the transcript's `provider_options` only).
/// TS chunks carry only the `@ai-sdk/*` provider's own namespaces, and a persisted UI message must
/// not hold an adapter-internal block.
fn ui_metadata(metadata: Option<Value>) -> Option<Value> {
    let Some(Value::Object(mut map)) = metadata else {
        return metadata;
    };
    map.remove(llm::NATIVE_METADATA_KEY);
    (!map.is_empty()).then_some(Value::Object(map))
}

/// `generateId()`: 16 alphanumeric characters.
fn generate_id() -> String {
    const ALPHABET: &[u8] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
    (0..16)
        .map(|_| char::from(ALPHABET[rand::random_range(0..ALPHABET.len())]))
        .collect()
}
