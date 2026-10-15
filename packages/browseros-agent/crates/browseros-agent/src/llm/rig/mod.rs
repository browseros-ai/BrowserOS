//! `RigModel`: the production `LanguageModel`, rig-core 0.44 used only to encode provider wires and
//! decode their streams. Everything BrowserOS-specific stays on this side of the seam: the TS
//! request shape (`overlay`, `transport`), the transcript conversion (`convert`), the error
//! mapping (`errors`) and the event vocabulary below. No rig type leaves this module.
//!
//! ```text
//!  ModelRequest ─convert/overlay─► rig CompletionRequest ─DynModel::stream─► PolicyClient ─► HTTP
//!  ModelStream  ◄──Translator◄──── rig Item<StreamEvent> (Start/Text/Reasoning/Arguments/End)
//!                                   + Streamed::finish() → usage, finish reason
//! ```

mod build;
mod convert;
mod copilot;
mod errors;
mod overlay;
mod transport;
mod wire_calls;

pub(crate) use build::build;

use super::{
    LanguageModel, ModelError, ModelEvent, ModelFinishReason, ModelRequest, ModelStream,
    ProviderKind, Usage,
};
use futures_util::StreamExt;
use rig_core::DynModel;
use rig_core::completion::{CompletionRequest, CompletionResponse, FinishReason};
use rig_core::message::AssistantContent;
use rig_core::operation::Completion;
use rig_core::streaming::{CompletionStream, Item, PartKind, StreamEvent};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

pub(crate) struct RigModel {
    provider: ProviderKind,
    model_id: String,
    model: DynModel<Completion>,
}

impl RigModel {
    /// The rig request for one step. Reasoning and output-cap fields go through
    /// `additional_params`, the top layer of rig's body merge (see `overlay`).
    fn encode(&self, request: ModelRequest) -> Result<CompletionRequest, ModelError> {
        let messages = convert::to_rig_messages(&request.messages)?;
        let Some(first) = messages.first().cloned() else {
            return Err(ModelError::InvalidPrompt(
                "request has no messages; providers require at least one".to_owned(),
            ));
        };
        let mut rig = CompletionRequest::new(first);
        rig.chat_history = messages;
        if let Some(system) = request.system.filter(|system| !system.is_empty()) {
            rig = rig.preamble(system);
        }
        rig.tools = convert::tool_definitions(&request.tools)?;
        let overlay = overlay::overlay(self.provider, &self.model_id, &request.options);
        if !overlay.params.is_empty() {
            rig.additional_params = Some(Value::Object(overlay.params));
        }
        rig.max_tokens = overlay.max_tokens;
        Ok(rig)
    }
}

/// `message-normalization.ts` `supportsToolResultMediaTransport`.
pub(crate) fn media_in_tool_results(provider: ProviderKind, model_id: &str) -> bool {
    match provider {
        ProviderKind::Anthropic
        | ProviderKind::OpenAi
        | ProviderKind::Azure
        | ProviderKind::Bedrock => true,
        ProviderKind::Google => {
            let model = model_id.to_lowercase();
            model.contains("gemini-3") && !model.contains("gemini-2")
        }
        _ => false,
    }
}

#[async_trait::async_trait]
impl LanguageModel for RigModel {
    fn provider(&self) -> ProviderKind {
        self.provider
    }

    fn model_id(&self) -> &str {
        &self.model_id
    }

    fn supports_media_in_tool_results(&self) -> bool {
        media_in_tool_results(self.provider, &self.model_id)
    }

    async fn stream(
        &self,
        request: ModelRequest,
        cancel: CancellationToken,
    ) -> Result<ModelStream, ModelError> {
        let request = self.encode(request)?;
        // Nothing is sent until the stream is first polled.
        let stream = self
            .model
            .stream(request)
            .map_err(|error| errors::model_error(error, self.provider))?;
        Ok(Translator::new(stream, cancel, self.provider).into_stream())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Text,
    Reasoning,
    ToolCall,
}

struct OpenPart {
    id: String,
    kind: Kind,
    /// Raw argument JSON as the provider streamed it (tool calls only).
    arguments: String,
    /// The UI part this rig part feeds is not open: it was ended early by the lifecycle, or (for
    /// `CompatChat`) not started yet. rig's `End` for it then only delivers metadata.
    ui_closed: bool,
}

/// Which AI SDK provider's part lifecycle the events reproduce. rig decodes the wire; the TS
/// server's chunks follow the `@ai-sdk/*` provider's own start/end rules, which the app renders
/// (a reasoning part still open while the answer streams shows as "thinking").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Lifecycle {
    /// `@ai-sdk/openai-compatible` (`doStream`): one text (`txt-0`) and one reasoning
    /// (`reasoning-0`) part at a time, each started by its first non-empty delta. Reasoning ends at
    /// the first text or tool-call delta, text ends when reasoning resumes, and whatever is open
    /// ends with the response.
    CompatChat,
    /// Every other provider: rig's parts and ids, except that an open reasoning part ends when the
    /// next part starts (as the Anthropic/OpenAI/Google SDKs emit it; rig holds Responses
    /// reasoning open until the following item completes).
    Native,
}

impl Lifecycle {
    fn of(provider: ProviderKind) -> Self {
        // TS `provider-factory.ts`: the providers built with `createOpenAICompatible`.
        match provider {
            ProviderKind::OpenAiCompatible
            | ProviderKind::Ollama
            | ProviderKind::LmStudio
            | ProviderKind::Moonshot
            | ProviderKind::QwenCode
            | ProviderKind::GitHubCopilot => Self::CompatChat,
            _ => Self::Native,
        }
    }
}

const COMPAT_TEXT_ID: &str = "txt-0";
const COMPAT_REASONING_ID: &str = "reasoning-0";

/// Turns rig's part events into `ModelEvent`s with the AI SDK provider's [`Lifecycle`]. Native part
/// ids are `<random>_<part index>`, unique per call. A tool call is named, from its `Start`, by
/// the provider's own id as the raw wire reported it ([`wire_calls`]; rig names it only at `End`),
/// falling back to ours on wires without call ids; its empty argument fragments, which rig drops,
/// are restored where the wire had them.
///
/// **Call order = start order.** rig holds a Responses reasoning item open until the item it
/// precedes completes, so its `End` follows that item's. A consumer that appends calls as they end
/// (the AI SDK convention) would then replay a call before its reasoning, which the provider
/// rejects or ignores. `*End` and `ToolCall` events from rig are therefore released in part-start
/// order: one that ends while an earlier rig part is still open waits for it (in practice, the
/// next frame). Lifecycle ends (a reasoning part closed because the next part started) are UI
/// events and go out at once; rig's later `End` for that part becomes `PartMetadata`.
struct Translator {
    /// `None` once the reply ended; dropping it closes the HTTP response.
    stream: Option<CompletionStream>,
    cancel: CancellationToken,
    provider: ProviderKind,
    lifecycle: Lifecycle,
    prefix: String,
    /// rig parts that rig has not ended yet, by part index.
    open: HashMap<usize, OpenPart>,
    /// Ended parts waiting for an earlier part to end, by part index.
    ended: BTreeMap<usize, ModelEvent>,
    /// `CompatChat`: whether the single UI text / reasoning part is open.
    text_open: bool,
    reasoning_open: bool,
    /// What the raw wire says about this reply's tool calls.
    wire: Arc<wire_calls::WireToolCalls>,
    /// rig part index of a tool call → (its position among the reply's calls, non-empty argument
    /// fragments seen so far), to line rig's fragments up with the wire's.
    tools: HashMap<usize, (usize, usize)>,
    /// The reply's serialized `Origin`, read once, the first time a block needs it.
    origin: Option<Option<Value>>,
    queue: VecDeque<Result<ModelEvent, ModelError>>,
}

impl Translator {
    fn new(stream: CompletionStream, cancel: CancellationToken, provider: ProviderKind) -> Self {
        Self {
            stream: Some(stream),
            cancel,
            provider,
            lifecycle: Lifecycle::of(provider),
            prefix: format!("{:016x}", rand::random::<u64>()),
            open: HashMap::new(),
            ended: BTreeMap::new(),
            text_open: false,
            reasoning_open: false,
            wire: Arc::default(),
            tools: HashMap::new(),
            origin: None,
            queue: VecDeque::new(),
        }
    }

    fn into_stream(self) -> ModelStream {
        Box::pin(futures_util::stream::unfold(self, |mut this| async move {
            loop {
                if let Some(item) = this.queue.pop_front() {
                    return Some((item, this));
                }
                let stream = this.stream.as_mut()?;
                // Biased: a cancelled turn never reads one more provider event.
                // Scoped so the request rig opens inside this poll reports its raw tool calls.
                let next = tokio::select! {
                    biased;
                    _ = this.cancel.cancelled() => None,
                    item = wire_calls::WIRE_TOOL_CALLS.scope(this.wire.clone(), stream.next()) => Some(item),
                };
                match next {
                    None => this.fail(ModelError::Cancelled),
                    Some(Some(Ok(Item::Event(event)))) => this.on_event(event),
                    Some(Some(Ok(Item::Unknown(payload)))) => {
                        tracing::trace!(payload = %payload.value(), "unmodeled provider payload");
                    }
                    Some(Some(Err(error))) => this.fail(errors::model_error(error, this.provider)),
                    Some(None) => this.finish().await,
                }
            }
        }))
    }

    fn fail(&mut self, error: ModelError) {
        self.stream = None;
        self.queue.push_back(Err(error));
    }

    async fn finish(&mut self) {
        let Some(stream) = self.stream.take() else {
            return;
        };
        // The reply ended: nothing is still open, whatever rig left unclosed. The SDK's `flush`
        // closes reasoning, then text.
        self.open.clear();
        if self.reasoning_open {
            self.end_compat(Kind::Reasoning, None);
        }
        if self.text_open {
            self.end_compat(Kind::Text, None);
        }
        self.release_ended();
        let finished = tokio::select! {
            biased;
            _ = self.cancel.cancelled() => Err(ModelError::Cancelled),
            response = stream.finish() => response.map_err(|error| errors::model_error(error, self.provider)),
        };
        self.queue
            .push_back(finished.map(|response| finish_event(&response)));
    }

    fn origin(&mut self) -> Option<&Value> {
        if self.origin.is_none() {
            let origin = self
                .stream
                .as_ref()
                .and_then(|stream| serde_json::to_value(&stream.partial().origin).ok());
            self.origin = Some(origin);
        }
        self.origin.as_ref().and_then(Option::as_ref)
    }

    fn metadata(&mut self, block: &AssistantContent) -> Option<Value> {
        block.native_item()?;
        let origin = self.origin().cloned();
        convert::block_metadata(origin.as_ref(), block)
    }

    /// The open rig part at `index`, registered on first sight (rig always starts a part first;
    /// a wire that reports a part only at its end is handled in `on_end`).
    fn part(&mut self, index: usize, kind: Kind) -> &mut OpenPart {
        let prefix = &self.prefix;
        let lifecycle = self.lifecycle;
        self.open.entry(index).or_insert_with(|| OpenPart {
            id: match (lifecycle, kind) {
                (Lifecycle::CompatChat, Kind::Text) => COMPAT_TEXT_ID.to_owned(),
                (Lifecycle::CompatChat, Kind::Reasoning) => COMPAT_REASONING_ID.to_owned(),
                _ => format!("{prefix}_{index}"),
            },
            kind,
            arguments: String::new(),
            ui_closed: lifecycle == Lifecycle::CompatChat && kind != Kind::ToolCall,
        })
    }

    fn push(&mut self, event: ModelEvent) {
        self.queue.push_back(Ok(event));
    }

    fn on_event(&mut self, event: StreamEvent) {
        let index = event.part().index();
        match event {
            StreamEvent::Start { kind, name, .. } => {
                let kind = match kind {
                    PartKind::Text => Kind::Text,
                    PartKind::Reasoning => Kind::Reasoning,
                    PartKind::ToolCall => Kind::ToolCall,
                    PartKind::Image | PartKind::Opaque => return,
                };
                self.on_start(index, kind, name.map(|name| name.as_str().to_owned()));
            }
            StreamEvent::Text { text, .. } => self.on_delta(index, Kind::Text, text),
            StreamEvent::Reasoning { text, .. } => self.on_delta(index, Kind::Reasoning, text),
            StreamEvent::Arguments { json, .. } => {
                let part = self.part(index, Kind::ToolCall);
                part.arguments.push_str(&json);
                let id = part.id.clone();
                if !json.is_empty() {
                    self.restore_empty_fragments(index, &id);
                    if let Some((_, nonempty)) = self.tools.get_mut(&index) {
                        *nonempty += 1;
                    }
                }
                self.push(ModelEvent::ToolInputDelta { id, delta: json });
            }
            StreamEvent::End { content, .. } => self.on_end(index, content),
        }
    }

    fn on_start(&mut self, index: usize, kind: Kind, tool_name: Option<String>) {
        match self.lifecycle {
            Lifecycle::Native => self.close_native_reasoning(index),
            // The SDK ends reasoning at the first tool-call delta; text and reasoning start
            // lazily, at their first delta.
            Lifecycle::CompatChat if kind == Kind::ToolCall && self.reasoning_open => {
                self.end_compat(Kind::Reasoning, None);
            }
            Lifecycle::CompatChat => {}
        }
        let wire_id = (kind == Kind::ToolCall).then(|| {
            let position = self.tools.len();
            self.tools.insert(index, (position, 0));
            self.wire.id(position)
        });
        let part = self.part(index, kind);
        if let Some(Some(wire_id)) = wire_id {
            part.id = wire_id;
        }
        if part.ui_closed {
            return;
        }
        let id = part.id.clone();
        self.push(match kind {
            Kind::Text => ModelEvent::TextStart {
                id,
                provider_metadata: None,
            },
            Kind::Reasoning => ModelEvent::ReasoningStart {
                id,
                provider_metadata: None,
            },
            Kind::ToolCall => ModelEvent::ToolInputStart {
                id,
                tool_name: tool_name.unwrap_or_default(),
            },
        });
    }

    fn on_delta(&mut self, index: usize, kind: Kind, delta: String) {
        if self.lifecycle == Lifecycle::CompatChat {
            if delta.is_empty() {
                return;
            }
            // Text and reasoning take turns; the other one ends before this one (re)starts.
            let (other_open, this_open) = match kind {
                Kind::Text => (self.reasoning_open, self.text_open),
                _ => (self.text_open, self.reasoning_open),
            };
            if other_open {
                let other = if kind == Kind::Text {
                    Kind::Reasoning
                } else {
                    Kind::Text
                };
                self.end_compat(other, None);
            }
            if !this_open {
                let id = self.part(index, kind).id.clone();
                self.push(match kind {
                    Kind::Text => ModelEvent::TextStart {
                        id,
                        provider_metadata: None,
                    },
                    _ => ModelEvent::ReasoningStart {
                        id,
                        provider_metadata: None,
                    },
                });
                match kind {
                    Kind::Text => self.text_open = true,
                    _ => self.reasoning_open = true,
                }
            }
            self.part(index, kind).ui_closed = false;
        } else if self.part(index, kind).ui_closed {
            // rig kept streaming a part the lifecycle already ended: open a new UI part for it.
            let part = self.part(index, kind);
            part.ui_closed = false;
            let id = part.id.clone();
            self.push(match kind {
                Kind::Text => ModelEvent::TextStart {
                    id,
                    provider_metadata: None,
                },
                _ => ModelEvent::ReasoningStart {
                    id,
                    provider_metadata: None,
                },
            });
        }
        let id = self.part(index, kind).id.clone();
        self.push(match kind {
            Kind::Text => ModelEvent::TextDelta { id, delta },
            _ => ModelEvent::ReasoningDelta {
                id,
                delta,
                provider_metadata: None,
            },
        });
    }

    /// Emits the empty fragments the wire streamed for the call at `index` since its last
    /// non-empty one (rig drops them; the SDK forwards each as a `tool-input-delta`).
    fn restore_empty_fragments(&mut self, index: usize, id: &str) {
        let Some(&(position, nonempty)) = self.tools.get(&index) else {
            return;
        };
        for _ in 0..self.wire.empties_after(position, nonempty) {
            self.push(ModelEvent::ToolInputDelta {
                id: id.to_owned(),
                delta: String::new(),
            });
        }
    }

    /// `Native`: a part starting at `index` ends every other open reasoning part's UI part.
    fn close_native_reasoning(&mut self, index: usize) {
        let mut closing: Vec<(usize, String)> = self
            .open
            .iter_mut()
            .filter(|(at, part)| **at != index && part.kind == Kind::Reasoning && !part.ui_closed)
            .map(|(at, part)| {
                part.ui_closed = true;
                (*at, part.id.clone())
            })
            .collect();
        closing.sort_unstable();
        for (_, id) in closing {
            self.push(ModelEvent::ReasoningEnd {
                id,
                provider_metadata: None,
            });
        }
    }

    /// `CompatChat`: ends the single UI text or reasoning part; every rig part feeding it is then
    /// UI-closed, so their `End`s only carry metadata.
    fn end_compat(&mut self, kind: Kind, provider_metadata: Option<Value>) {
        for part in self.open.values_mut().filter(|part| part.kind == kind) {
            part.ui_closed = true;
        }
        match kind {
            Kind::Text => {
                self.text_open = false;
                self.push(ModelEvent::TextEnd {
                    id: COMPAT_TEXT_ID.to_owned(),
                    provider_metadata,
                });
            }
            _ => {
                self.reasoning_open = false;
                self.push(ModelEvent::ReasoningEnd {
                    id: COMPAT_REASONING_ID.to_owned(),
                    provider_metadata,
                });
            }
        }
    }

    fn on_end(&mut self, index: usize, content: AssistantContent) {
        let opened = self.open.remove(&index);
        let provider_metadata = self.metadata(&content);
        let kind = match &content {
            AssistantContent::Text(_) => Kind::Text,
            AssistantContent::Reasoning(_) => Kind::Reasoning,
            AssistantContent::ToolCall(_) => Kind::ToolCall,
            _ => {
                self.release_ended();
                return;
            }
        };
        if kind != Kind::ToolCall {
            let opened = match opened {
                // A wire that reports a part only at its end (a Responses reasoning item without a
                // summary carries just its encrypted content) still opens it first, so the
                // transcript keeps the native item. The compat wire always streams deltas first.
                None if self.lifecycle == Lifecycle::Native => {
                    self.on_start(index, kind, None);
                    self.open.remove(&index)
                }
                opened => opened,
            };
            let ui_open = opened.as_ref().is_some_and(|part| !part.ui_closed);
            let compat_open = match kind {
                Kind::Text => self.text_open,
                _ => self.reasoning_open,
            };
            if self.lifecycle == Lifecycle::CompatChat && ui_open && compat_open {
                // The SDK ends its one part when the response does; rig ends each part then.
                self.end_compat(kind, provider_metadata);
            } else if !ui_open {
                if let (Some(part), Some(provider_metadata)) = (&opened, provider_metadata) {
                    self.push(ModelEvent::PartMetadata {
                        id: part.id.clone(),
                        provider_metadata,
                    });
                }
            } else if let Some(part) = opened {
                let id = part.id;
                self.ended.insert(
                    index,
                    match kind {
                        Kind::Text => ModelEvent::TextEnd {
                            id,
                            provider_metadata,
                        },
                        _ => ModelEvent::ReasoningEnd {
                            id,
                            provider_metadata,
                        },
                    },
                );
            }
            self.release_ended();
            return;
        }
        let AssistantContent::ToolCall(call) = content else {
            return;
        };
        let tool_name = call.function.name.as_str().to_owned();
        let (id, streamed) = match opened {
            Some(part) => {
                self.restore_empty_fragments(index, &part.id);
                (part.id, part.arguments)
            }
            // A wire that reports a call only at its end still opens it first, so the loop
            // always sees start → (deltas) → call.
            None => {
                if self.lifecycle == Lifecycle::CompatChat && self.reasoning_open {
                    self.end_compat(Kind::Reasoning, None);
                } else if self.lifecycle == Lifecycle::Native {
                    self.close_native_reasoning(index);
                }
                let provider_id = call.id.wire();
                let id = if provider_id.is_empty() {
                    format!("{}_{index}", self.prefix)
                } else {
                    provider_id.into_owned()
                };
                self.push(ModelEvent::ToolInputStart {
                    id: id.clone(),
                    tool_name: tool_name.clone(),
                });
                (id, String::new())
            }
        };
        // The provider's own argument text, as AI SDK's `tool-call.input`.
        let input_json = call
            .function
            .invalid_arguments
            .clone()
            .or_else(|| (!streamed.trim().is_empty()).then_some(streamed))
            .unwrap_or_else(|| call.function.arguments_value().to_string());
        self.ended.insert(
            index,
            ModelEvent::ToolCall {
                id,
                provider_call_id: call.id.wire().into_owned(),
                tool_name,
                input_json,
                provider_metadata,
            },
        );
        self.release_ended();
    }

    /// Emits the ended parts that no earlier, still-open part precedes, in part order.
    fn release_ended(&mut self) {
        let first_open = self.open.keys().min().copied();
        while let Some(entry) = self.ended.first_entry() {
            if first_open.is_some_and(|open| *entry.key() > open) {
                break;
            }
            let event = entry.remove();
            self.push(event);
        }
    }
}

fn finish_event(response: &CompletionResponse) -> ModelEvent {
    let has_calls = response.tool_calls().next().is_some();
    let reason = match response.finish_reason() {
        Some(FinishReason::ContentFilter) => ModelFinishReason::ContentFilter,
        _ if response.error.is_some() => ModelFinishReason::Error,
        _ if has_calls => ModelFinishReason::ToolCalls,
        Some(FinishReason::Length) => ModelFinishReason::Length,
        Some(FinishReason::Stop) | None => ModelFinishReason::Stop,
        Some(_) => ModelFinishReason::Other,
    };
    let usage = &response.usage;
    ModelEvent::Finish {
        reason,
        usage: Usage {
            input_tokens: usage.input_tokens,
            output_tokens: usage.output_tokens,
            cached_input_tokens: usage.cached_input_tokens,
            reasoning_tokens: usage.reasoning_tokens,
        },
    }
}
