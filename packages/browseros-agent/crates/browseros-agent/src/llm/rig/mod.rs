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
use std::collections::{HashMap, VecDeque};
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

struct OpenPart {
    id: String,
    /// Raw argument JSON as the provider streamed it (tool calls only).
    arguments: String,
}

/// Turns rig's part events into `ModelEvent`s. Part ids are `<random>_<part index>`: unique per
/// call, so they stay unique across a turn's steps. Tool calls get our id at `Start` (rig names
/// the provider's call id only at `End`); `ToolCall` pairs it with the provider's.
struct Translator {
    /// `None` once the reply ended; dropping it closes the HTTP response.
    stream: Option<CompletionStream>,
    cancel: CancellationToken,
    provider: ProviderKind,
    prefix: String,
    open: HashMap<usize, OpenPart>,
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
            prefix: format!("{:016x}", rand::random::<u64>()),
            open: HashMap::new(),
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
                let next = tokio::select! {
                    biased;
                    _ = this.cancel.cancelled() => None,
                    item = stream.next() => Some(item),
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

    fn part_id(&mut self, index: usize) -> String {
        let prefix = &self.prefix;
        self.open
            .entry(index)
            .or_insert_with(|| OpenPart {
                id: format!("{prefix}_{index}"),
                arguments: String::new(),
            })
            .id
            .clone()
    }

    fn push(&mut self, event: ModelEvent) {
        self.queue.push_back(Ok(event));
    }

    fn on_event(&mut self, event: StreamEvent) {
        let index = event.part().index();
        match event {
            StreamEvent::Start { kind, name, .. } => {
                let id = self.part_id(index);
                match kind {
                    PartKind::Text => self.push(ModelEvent::TextStart {
                        id,
                        provider_metadata: None,
                    }),
                    PartKind::Reasoning => self.push(ModelEvent::ReasoningStart {
                        id,
                        provider_metadata: None,
                    }),
                    PartKind::ToolCall => self.push(ModelEvent::ToolInputStart {
                        id,
                        tool_name: name
                            .map(|name| name.as_str().to_owned())
                            .unwrap_or_default(),
                    }),
                    PartKind::Image | PartKind::Opaque => {}
                }
            }
            StreamEvent::Text { text, .. } => {
                let id = self.part_id(index);
                self.push(ModelEvent::TextDelta { id, delta: text });
            }
            StreamEvent::Reasoning { text, .. } => {
                let id = self.part_id(index);
                self.push(ModelEvent::ReasoningDelta {
                    id,
                    delta: text,
                    provider_metadata: None,
                });
            }
            StreamEvent::Arguments { json, .. } => {
                let id = self.part_id(index);
                if let Some(part) = self.open.get_mut(&index) {
                    part.arguments.push_str(&json);
                }
                self.push(ModelEvent::ToolInputDelta { id, delta: json });
            }
            StreamEvent::End { content, .. } => self.on_end(index, content),
        }
    }

    fn on_end(&mut self, index: usize, content: AssistantContent) {
        let opened = self.open.remove(&index);
        let provider_metadata = self.metadata(&content);
        let id = match &opened {
            Some(part) => part.id.clone(),
            None => format!("{}_{index}", self.prefix),
        };
        match content {
            AssistantContent::Text(_) => self.push(ModelEvent::TextEnd {
                id,
                provider_metadata,
            }),
            AssistantContent::Reasoning(_) => self.push(ModelEvent::ReasoningEnd {
                id,
                provider_metadata,
            }),
            AssistantContent::ToolCall(call) => {
                let tool_name = call.function.name.as_str().to_owned();
                let streamed = match opened {
                    Some(part) => part.arguments,
                    // A wire that reports a call only at its end still opens it first, so the
                    // loop always sees start → (deltas) → call.
                    None => {
                        self.push(ModelEvent::ToolInputStart {
                            id: id.clone(),
                            tool_name: tool_name.clone(),
                        });
                        String::new()
                    }
                };
                // The provider's own argument text, as AI SDK's `tool-call.input`.
                let input_json = call
                    .function
                    .invalid_arguments
                    .clone()
                    .or_else(|| (!streamed.trim().is_empty()).then_some(streamed))
                    .unwrap_or_else(|| call.function.arguments_value().to_string());
                self.push(ModelEvent::ToolCall {
                    id,
                    provider_call_id: call.id.wire().into_owned(),
                    tool_name,
                    input_json,
                    provider_metadata,
                });
            }
            _ => {}
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
