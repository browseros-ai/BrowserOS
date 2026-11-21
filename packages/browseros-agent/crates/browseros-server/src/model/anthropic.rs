//! Anthropic Messages. Thinking, redacted thinking and tool use are reconstructed from indexed
//! content blocks and replayed together, including the provider's signatures.

use std::collections::BTreeMap;

use base64::Engine;
use futures_util::StreamExt;
use http::header::{HeaderMap, HeaderName};
use serde_json::{Map, Value, json};
use url::Url;

use super::{AssistantTurn, Effort, ModelMessage, NativeOutput, Reasoning, StepRequest, Usage};
use super::{errors, headers, set_header, sse, tool_call, tool_started};
use crate::store::ProviderConfig;
use crate::turn::{ChatError, EventSink, FinishReason, TurnEvent};

pub(super) struct Endpoint {
    url: Url,
    headers: HeaderMap,
}

pub(super) fn endpoint(provider: &ProviderConfig) -> Result<Endpoint, ChatError> {
    let key = super::api_key(provider)?;
    let base = provider
        .base_url
        .as_deref()
        .filter(|url| !url.trim().is_empty())
        .unwrap_or("https://api.anthropic.com/v1/");
    let url = super::endpoint(base, "messages")?;
    let mut headers = headers(&provider.headers)?;
    let version = HeaderName::from_static("anthropic-version");
    if !headers.contains_key(&version) {
        set_header(&mut headers, version, "2023-06-01", false)?;
    }
    set_header(
        &mut headers,
        HeaderName::from_static("x-api-key"),
        key,
        true,
    )?;
    Ok(Endpoint { url, headers })
}

fn body(model: &str, request: &StepRequest<'_>, reasoning: &Reasoning) -> Value {
    let mut messages = Vec::new();
    let mut user = Vec::new();
    for message in request.messages {
        match message {
            ModelMessage::User { text, images } => {
                if !text.is_empty() {
                    user.push(json!({"type": "text", "text": text}));
                }
                user.extend(images.iter().map(|image| {
                    json!({
                        "type": "image",
                        "source": {
                            "type": "base64", "media_type": image.media_type,
                            "data": base64::engine::general_purpose::STANDARD.encode(&image.bytes),
                        }
                    })
                }));
            }
            ModelMessage::ToolResult { id, text } => {
                user.push(json!({"type": "tool_result", "tool_use_id": id.0, "content": text}));
            }
            ModelMessage::Assistant(turn) => {
                flush_user(&mut messages, &mut user);
                let content = match &turn.native {
                    NativeOutput::Anthropic(blocks) => blocks.clone(),
                    _ => vec![json!({"type": "text", "text": turn.text})],
                };
                messages.push(json!({"role": "assistant", "content": content}));
            }
        }
    }
    flush_user(&mut messages, &mut user);
    let mut body =
        json!({"model": model, "messages": messages, "stream": true, "max_tokens": 4096});
    if !request.system.is_empty() {
        body["system"] = json!(request.system);
    }
    if !request.tools.is_empty() {
        body["tools"] = Value::Array(request.tools.iter().map(|tool| json!({
            "name": tool.name, "description": tool.description, "input_schema": tool.input_schema
        })).collect());
    }
    if let Some((fields, max_tokens)) = thinking_fields(model, request.effort, reasoning)
        && let Some(body) = body.as_object_mut()
    {
        body.extend(fields);
        body.insert("max_tokens".into(), json!(max_tokens));
    }
    body
}

fn flush_user(messages: &mut Vec<Value>, user: &mut Vec<Value>) {
    if !user.is_empty() {
        // Anthropic requires all tool results immediately after the assistant tool calls,
        // before any text or images attached to those results.
        user.sort_by_key(|block| block["type"] != "tool_result");
        messages.push(json!({"role": "user", "content": std::mem::take(user)}));
    }
}

/// Claude on the Messages API and Bedrock uses the same model-specific thinking fields.
pub(super) fn thinking_fields(
    model: &str,
    requested: Option<Effort>,
    reasoning: &Reasoning,
) -> Option<(Map<String, Value>, i32)> {
    let mut parts = model.split('-');
    let family = parts.next();
    let series = parts.next();
    let major = parts.next().and_then(|part| part.parse::<u32>().ok());
    let minor = parts.next().and_then(|part| part.parse::<u32>().ok());
    let adaptive = family == Some("claude")
        && (matches!(series, Some("fable" | "mythos"))
            || major.is_some_and(|major| major >= 5)
            || (major == Some(4)
                && matches!(series, Some("sonnet" | "opus"))
                && minor.is_some_and(|minor| minor >= 6)));
    let mut fields = Map::new();
    let max_tokens = if adaptive {
        // Claude 4.6 accepts max, with xhigh introduced for later models.
        let levels: &[_] = if major == Some(4) && minor == Some(6) {
            &["low", "medium", "high", "max"]
        } else {
            &["low", "medium", "high", "xhigh", "max"]
        };
        let effort = reasoning.effort(requested, levels)?;
        fields.insert(
            "thinking".into(),
            json!({"type": "adaptive", "display": "summarized"}),
        );
        fields.insert("output_config".into(), json!({"effort": effort}));
        32_768
    } else {
        let effort = reasoning.effort(
            requested,
            &["minimal", "low", "medium", "high", "xhigh", "max"],
        )?;
        let budget = super::thinking_budget(effort);
        fields.insert(
            "thinking".into(),
            json!({"type": "enabled", "budget_tokens": budget}),
        );
        budget + 4096
    };
    Some((fields, max_tokens))
}

pub(super) async fn step(
    http: &reqwest::Client,
    endpoint: &Endpoint,
    model: &str,
    request: &StepRequest<'_>,
    reasoning: &Reasoning,
    sink: &mut dyn EventSink,
) -> Result<AssistantTurn, ChatError> {
    let mut events = std::pin::pin!(sse::events(
        sse::post(
            http,
            &endpoint.url,
            &endpoint.headers,
            &body(model, request, reasoning)
        )
        .await?
    ));
    let mut message = Message::default();
    while let Some(data) = events.next().await {
        let event = sse::json(&data?)?;
        match event["type"].as_str() {
            Some("message_start") => message.metadata(&event["message"]),
            Some("content_block_start") => message.start_block(&event, sink).await?,
            Some("content_block_delta") => message.delta(&event, sink).await?,
            Some("content_block_stop") => message.stop_block(&event)?,
            Some("message_delta") => {
                message.metadata(&event);
                message.metadata(&event["delta"]);
            }
            Some("message_stop") => return message.complete(),
            Some("error") => return Err(errors::stream(&event, &endpoint.headers)),
            _ => {}
        }
    }
    Err(errors::disconnected())
}

/// Each block retains its native fields. The one exception during streaming is tool input,
/// which arrives as a partial JSON string and becomes an object on content_block_stop.
struct Block {
    value: Value,
    partial_input: String,
    closed: bool,
}

#[derive(Default)]
struct Message {
    blocks: BTreeMap<u64, Block>,
    text: String,
    input: Option<u64>,
    output: Option<u64>,
    finish: Option<FinishReason>,
}

impl Message {
    fn metadata(&mut self, event: &Value) {
        let usage = &event["usage"];
        if let Some(input) = usage["input_tokens"].as_u64() {
            self.input = Some(
                input
                    .saturating_add(usage["cache_creation_input_tokens"].as_u64().unwrap_or(0))
                    .saturating_add(usage["cache_read_input_tokens"].as_u64().unwrap_or(0)),
            );
        }
        self.output = usage["output_tokens"].as_u64().or(self.output);
        if let Some(reason) = event["stop_reason"].as_str() {
            self.finish = Some(match reason {
                "end_turn" | "stop_sequence" => FinishReason::Stop,
                "tool_use" => FinishReason::ToolCalls,
                "max_tokens" | "model_context_window_exceeded" => FinishReason::Length,
                "refusal" => FinishReason::ContentFilter,
                _ => FinishReason::Other,
            });
        }
    }

    async fn start_block(
        &mut self,
        event: &Value,
        sink: &mut dyn EventSink,
    ) -> Result<(), ChatError> {
        let index = index(event)?;
        if self.blocks.contains_key(&index) {
            return Err(errors::protocol(
                "The provider started a content block twice.",
            ));
        }
        let block = &event["content_block"];
        if !block.is_object() {
            return Err(errors::protocol(
                "The provider started an invalid content block.",
            ));
        }
        match block["type"].as_str() {
            Some("tool_use") => {
                let tool = tool_call(&block["id"], &block["name"], block["input"].clone())?;
                tool_started(sink, &tool.id.0, &tool.name).await;
            }
            Some("text") => {
                if let Some(text) = block["text"].as_str().filter(|text| !text.is_empty()) {
                    self.text.push_str(text);
                    sink.emit(TurnEvent::Text(text.into())).await;
                }
            }
            Some("thinking") => {
                if let Some(text) = block["thinking"].as_str().filter(|text| !text.is_empty()) {
                    sink.emit(TurnEvent::Reasoning(text.into())).await;
                }
            }
            _ => {}
        }
        self.blocks.insert(
            index,
            Block {
                value: block.clone(),
                partial_input: String::new(),
                closed: false,
            },
        );
        Ok(())
    }

    async fn delta(&mut self, event: &Value, sink: &mut dyn EventSink) -> Result<(), ChatError> {
        let block = self
            .blocks
            .get_mut(&index(event)?)
            .filter(|block| !block.closed)
            .ok_or_else(|| {
                errors::protocol("The provider sent a delta for a closed or missing content block.")
            })?;
        let delta = &event["delta"];
        match delta["type"].as_str() {
            Some("text_delta") => {
                if let Some(text) = append(&mut block.value, delta, "text") {
                    self.text.push_str(text);
                    sink.emit(TurnEvent::Text(text.into())).await;
                }
            }
            Some("thinking_delta") => {
                if let Some(text) = append(&mut block.value, delta, "thinking") {
                    sink.emit(TurnEvent::Reasoning(text.into())).await;
                }
            }
            Some("signature_delta") => {
                append(&mut block.value, delta, "signature");
            }
            Some("input_json_delta") => {
                if let Some(input) = delta["partial_json"].as_str() {
                    block.partial_input.push_str(input);
                }
            }
            Some("citations_delta") => {
                let citations = &mut block.value["citations"];
                if citations.is_null() {
                    *citations = json!([]);
                }
                if let Some(citations) = citations.as_array_mut() {
                    citations.push(delta["citation"].clone());
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn stop_block(&mut self, event: &Value) -> Result<(), ChatError> {
        let block = self
            .blocks
            .get_mut(&index(event)?)
            .filter(|block| !block.closed)
            .ok_or_else(|| {
                errors::protocol("The provider stopped a closed or missing content block.")
            })?;
        if !block.partial_input.is_empty() {
            block.value["input"] =
                super::tool_input(&Value::String(std::mem::take(&mut block.partial_input)))?;
        }
        block.closed = true;
        Ok(())
    }

    fn complete(self) -> Result<AssistantTurn, ChatError> {
        let finish = self.finish.ok_or_else(errors::disconnected)?;
        if self.blocks.values().any(|block| !block.closed) {
            return Err(errors::disconnected());
        }
        let blocks: Vec<_> = self.blocks.into_values().map(|block| block.value).collect();
        let calls = if finish == FinishReason::ToolCalls {
            blocks
                .iter()
                .filter(|block| block["type"] == "tool_use")
                .map(|block| tool_call(&block["id"], &block["name"], block["input"].clone()))
                .collect::<Result<_, _>>()?
        } else {
            Vec::new()
        };
        let usage = self
            .input
            .zip(self.output)
            .map(|(input_tokens, output_tokens)| Usage {
                input_tokens,
                output_tokens,
            });
        Ok(AssistantTurn {
            text: self.text,
            calls,
            finish,
            usage,
            native: NativeOutput::Anthropic(blocks),
        })
    }
}

fn index(event: &Value) -> Result<u64, ChatError> {
    event["index"]
        .as_u64()
        .ok_or_else(|| errors::protocol("An Anthropic content block arrived without its index."))
}

fn append<'a>(block: &mut Value, delta: &'a Value, field: &str) -> Option<&'a str> {
    let fragment = delta[field].as_str().filter(|text| !text.is_empty())?;
    match &mut block[field] {
        Value::String(content) => content.push_str(fragment),
        slot => *slot = Value::String(fragment.into()),
    }
    Some(fragment)
}
