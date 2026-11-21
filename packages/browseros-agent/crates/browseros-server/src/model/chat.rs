//! The Chat Completions wire for eight provider types. Indexed tool deltas are assembled
//! into assistant calls so route-specific extensions survive the next step of the turn.

use std::collections::{BTreeMap, BTreeSet};

use futures_util::StreamExt;
use http::header::{AUTHORIZATION, HeaderMap, HeaderName};
use serde_json::{Value, json};
use url::Url;

use super::{AssistantTurn, Effort, ModelMessage, NativeOutput, Reasoning, StepRequest, Usage};
use super::{
    bearer, data_url, errors, headers, set_header, sse, tool_call, tool_input, tool_started, usage,
};
use crate::store::{Credentials, ProviderConfig, ProviderKind};
use crate::turn::{ChatError, EventSink, FinishReason, TurnEvent};

pub(super) struct Endpoint {
    url: Url,
    headers: HeaderMap,
    kind: ProviderKind,
}

pub(super) fn endpoint(provider: &ProviderConfig) -> Result<Endpoint, ChatError> {
    use ProviderKind::*;
    let mut headers = headers(&provider.headers)?;
    let default = match provider.kind {
        Google => "https://generativelanguage.googleapis.com/v1beta/openai/",
        Openrouter => "https://openrouter.ai/api/v1/",
        Fireworks => "https://api.fireworks.ai/inference/v1/",
        Ollama => "http://localhost:11434/v1/",
        Lmstudio => "http://localhost:1234/v1/",
        Azure | Moonshot | OpenaiCompatible => "",
        _ => {
            return Err(errors::config(
                "This provider does not use Chat Completions.",
            ));
        }
    };
    let base = provider
        .base_url
        .as_deref()
        .filter(|url| !url.trim().is_empty());
    if provider.kind == Azure && super::api_key(provider).is_err() {
        return Err(errors::config(
            "Azure requires an API key and either a resource name or a base URL.",
        ));
    }
    if provider.kind == Moonshot && (base.is_none() || super::api_key(provider).is_err()) {
        return Err(errors::config(
            "Moonshot requires an API key and a base URL.",
        ));
    }
    let url = match (provider.kind, base) {
        (Azure, None) => {
            let resource = provider
                .resource_name
                .as_deref()
                .filter(|name| {
                    !name.is_empty()
                        && name
                            .bytes()
                            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
                })
                .ok_or_else(|| errors::config("Azure requires a resource name or a base URL."))?;
            super::endpoint(
                &format!("https://{resource}.openai.azure.com/openai/v1"),
                "chat/completions",
            )?
        }
        (OpenaiCompatible, None) => {
            return Err(errors::config(
                "An OpenAI-compatible provider requires a base URL.",
            ));
        }
        (_, base) => super::endpoint(base.unwrap_or(default), "chat/completions")?,
    };
    match provider.kind {
        Azure => {
            headers.remove(AUTHORIZATION);
            set_header(
                &mut headers,
                HeaderName::from_static("api-key"),
                super::api_key(provider)?,
                true,
            )?;
        }
        // Local configuration can authenticate to a reverse proxy. With no credential
        // configured, use no placeholder key.
        OpenaiCompatible | Ollama | Lmstudio
            if matches!(provider.credentials, Credentials::None) => {}
        _ => bearer(&mut headers, super::api_key(provider)?)?,
    }
    Ok(Endpoint {
        url,
        headers,
        kind: provider.kind,
    })
}

fn body(
    endpoint: &Endpoint,
    model: &str,
    request: &StepRequest<'_>,
    reasoning: &Reasoning,
) -> Value {
    let mut messages = Vec::new();
    if !request.system.is_empty() {
        messages.push(json!({"role": "system", "content": request.system}));
    }
    let ordered = order_tool_pictures(request.messages);
    for run in ordered.chunk_by(|(left, _), (right, _)| {
        matches!(
            (left, right),
            (ModelMessage::User { .. }, ModelMessage::User { .. })
        )
    }) {
        messages.push(match run {
            [(ModelMessage::Assistant(turn), _)] => match &turn.native {
                NativeOutput::Chat(message) => message.clone(),
                _ => json!({"role": "assistant", "content": turn.text}),
            },
            [(ModelMessage::ToolResult { id, text }, _)] => {
                json!({"role": "tool", "tool_call_id": id.0, "content": text})
            }
            users => user_message(users),
        });
    }
    let mut body = json!({"model": model, "messages": messages, "stream": true});
    if !request.tools.is_empty() {
        body["tools"] = Value::Array(request.tools.iter().map(|tool| json!({
            "type": "function",
            "function": {"name": tool.name, "description": tool.description, "parameters": tool.input_schema}
        })).collect());
    }
    use ProviderKind::*;
    match endpoint.kind {
        Google | Azure => {
            let gemini_xhigh = ["gemini-4-neon", "gemini-4-hydrogen"]
                .iter()
                .any(|family| model.trim_start_matches("models/").starts_with(family));
            let levels: &[_] = if endpoint.kind == Google && !gemini_xhigh {
                &["minimal", "low", "medium", "high"]
            } else {
                &["minimal", "low", "medium", "high", "xhigh"]
            };
            if let Some(effort) = reasoning.effort(request.effort, levels) {
                body["reasoning_effort"] = json!(effort);
            }
        }
        Openrouter => {
            let levels = ["minimal", "low", "medium", "high", "xhigh", "max"];
            if let Some(effort) = reasoning.effort(request.effort, &levels) {
                // Keep the established empty options object for the default medium case.
                body["reasoning"] =
                    if request.effort == Some(Effort::Medium) && reasoning.effort.is_none() {
                        json!({})
                    } else {
                        json!({"effort": effort})
                    };
            }
        }
        _ => {}
    }
    if matches!(endpoint.kind, Azure | Fireworks | Google | Openrouter) {
        body["stream_options"] = json!({"include_usage": true});
    }
    body
}

/// The loop places each picture message immediately after its tool result. The wire sends
/// all results from the assistant first, then labels the pictures when several calls shared
/// that step. Compaction can replace a picture message with a text-only placeholder.
fn order_tool_pictures(messages: &[ModelMessage]) -> Vec<(&ModelMessage, Option<&str>)> {
    let mut ordered = Vec::with_capacity(messages.len());
    let mut pictures = Vec::new();
    let mut remaining = messages;
    let mut results = 0;
    while let Some((message, tail)) = remaining.split_first() {
        remaining = tail;
        ordered.push((message, None));
        let ModelMessage::ToolResult { id, .. } = message else {
            continue;
        };
        results += 1;
        if let Some((user @ ModelMessage::User { .. }, tail)) = remaining.split_first() {
            pictures.push((user, id.0.as_str()));
            remaining = tail;
        }
        if !matches!(remaining.first(), Some(ModelMessage::ToolResult { .. })) {
            ordered.extend(
                pictures
                    .drain(..)
                    .map(|(user, id)| (user, (results > 1).then_some(id))),
            );
            results = 0;
        }
    }
    ordered
}

/// Serialize after compaction so earlier questions can still be dropped independently.
fn user_message(users: &[(&ModelMessage, Option<&str>)]) -> Value {
    let mut content = Vec::new();
    let mut text = String::new();
    for (index, (message, call)) in users.iter().enumerate() {
        let ModelMessage::User { text: user, images } = message else {
            continue;
        };
        if index > 0 {
            text.push_str("\n\n");
        }
        if let Some(call) = call {
            text.push_str("Tool call ");
            text.push_str(call);
            text.push_str(": ");
        }
        text.push_str(user);
        if !images.is_empty() {
            content.push(json!({"type": "text", "text": std::mem::take(&mut text)}));
            content.extend(
                images.iter().map(
                    |image| json!({"type": "image_url", "image_url": {"url": data_url(image)}}),
                ),
            );
        }
    }
    if content.is_empty() {
        return json!({"role": "user", "content": text});
    }
    if !text.is_empty() {
        content.push(json!({"type": "text", "text": text}));
    }
    json!({"role": "user", "content": content})
}

/// A single streamed choice. A tool's transport index orders deltas but is not part of
/// the assistant message sent back to the provider. Reasoning detail indexes are native.
#[derive(Default)]
struct Choice {
    text: String,
    refusal: String,
    reasoning: String,
    details: BTreeMap<(u64, String), Value>,
    calls: BTreeMap<u64, Value>,
    announced: BTreeSet<u64>,
    usage: Option<Usage>,
    finish: Option<FinishReason>,
}

impl Choice {
    async fn delta(
        &mut self,
        delta: &Value,
        kind: ProviderKind,
        sink: &mut dyn EventSink,
    ) -> Result<(), ChatError> {
        if let Some(text) = nonempty(&delta["content"]) {
            self.text.push_str(text);
            sink.emit(TurnEvent::Text(text.into())).await;
        }
        if let Some(refusal) = nonempty(&delta["refusal"]) {
            self.refusal.push_str(refusal);
            sink.emit(TurnEvent::Text(refusal.into())).await;
        }
        if kind == ProviderKind::Openrouter {
            if let Some(text) = nonempty(&delta["reasoning"]) {
                self.reasoning.push_str(text);
                sink.emit(TurnEvent::Reasoning(text.into())).await;
            }
            if let Some(details) = delta["reasoning_details"].as_array() {
                for (position, delta) in details.iter().enumerate() {
                    let index = delta["index"].as_u64().unwrap_or(position as u64);
                    let kind = delta["type"].as_str().unwrap_or_default();
                    let detail = self.details.entry((index, kind.into())).or_default();
                    merge(detail, delta, &["text", "summary", "data", "signature"]);
                }
            }
        } else if kind != ProviderKind::Google
            && let Some(text) =
                nonempty(&delta["reasoning_content"]).or_else(|| nonempty(&delta["reasoning"]))
        {
            // Kimi validates reasoning_content in the assistant message preceding tool results.
            // Generic compatible gateways may reject this extension on replay.
            if kind == ProviderKind::Moonshot {
                self.reasoning.push_str(text);
            }
            sink.emit(TurnEvent::Reasoning(text.into())).await;
        }
        if let Some(calls) = delta["tool_calls"].as_array() {
            for delta in calls {
                let index = delta["index"].as_u64().ok_or_else(|| {
                    errors::protocol("A tool call delta arrived without its index.")
                })?;
                let call = self.calls.entry(index).or_default();
                merge(call, delta, &["arguments"]);
                if let Some(object) = call.as_object_mut() {
                    object.remove("index");
                    object.entry("type").or_insert(json!("function"));
                }
                if let (Some(id), Some(name)) =
                    (nonempty(&call["id"]), nonempty(&call["function"]["name"]))
                    && self.announced.insert(index)
                {
                    tool_started(sink, id, name).await;
                }
            }
        }
        Ok(())
    }

    fn complete(self, kind: ProviderKind) -> Result<AssistantTurn, ChatError> {
        let finish = self.finish.ok_or_else(errors::disconnected)?;
        let calls = if finish == FinishReason::ToolCalls {
            self.calls
                .values()
                .map(|call| {
                    tool_call(
                        &call["id"],
                        &call["function"]["name"],
                        tool_input(&call["function"]["arguments"])?,
                    )
                })
                .collect::<Result<Vec<_>, _>>()?
        } else {
            Vec::new()
        };
        let mut message = json!({"role": "assistant", "content": self.text});
        if self.text.is_empty() && (!self.calls.is_empty() || !self.refusal.is_empty()) {
            message["content"] = Value::Null;
        }
        if !self.calls.is_empty() {
            message["tool_calls"] = Value::Array(self.calls.into_values().collect());
        }
        if !self.refusal.is_empty() {
            message["refusal"] = json!(self.refusal);
        }
        if !self.reasoning.is_empty() {
            let field = if kind == ProviderKind::Moonshot {
                "reasoning_content"
            } else {
                "reasoning"
            };
            message[field] = json!(self.reasoning);
        }
        if !self.details.is_empty() {
            message["reasoning_details"] = Value::Array(self.details.into_values().collect());
        }
        Ok(AssistantTurn {
            text: self.text + &self.refusal,
            calls,
            finish,
            usage: self.usage,
            native: NativeOutput::Chat(message),
        })
    }
}

/// Most fields are complete values (including opaque nested signatures). Only documented
/// incremental strings are appended. Null placeholders on later deltas leave earlier data intact.
fn merge(target: &mut Value, delta: &Value, fragments: &[&str]) {
    if let Some(object) = delta.as_object() {
        if !target.is_object() {
            *target = json!({});
        }
        for (key, value) in object {
            let slot = &mut target[key];
            if fragments.contains(&key.as_str())
                && let (Value::String(first), Value::String(next)) = (&mut *slot, value)
            {
                first.push_str(next);
            } else {
                merge(slot, value, fragments);
            }
        }
    } else if !delta.is_null() {
        *target = delta.clone();
    }
}

fn nonempty(value: &Value) -> Option<&str> {
    value.as_str().filter(|text| !text.is_empty())
}

pub(super) async fn step(
    http: &reqwest::Client,
    endpoint: &Endpoint,
    model: &str,
    request: &StepRequest<'_>,
    reasoning: &Reasoning,
    sink: &mut dyn EventSink,
) -> Result<AssistantTurn, ChatError> {
    let response = sse::post(
        http,
        &endpoint.url,
        &endpoint.headers,
        &body(endpoint, model, request, reasoning),
    )
    .await?;
    let mut events = std::pin::pin!(sse::events(response));
    let mut choice = Choice::default();
    while let Some(data) = events.next().await {
        let data = data?;
        if data.trim() == "[DONE]" {
            break;
        }
        let event = sse::json(&data)?;
        if !event["error"].is_null() {
            return Err(errors::stream(&event, &endpoint.headers));
        }
        choice.usage = usage(
            &event["usage"]["prompt_tokens"],
            &event["usage"]["completion_tokens"],
        )
        .or(choice.usage);
        if let Some(chunks) = event["choices"].as_array() {
            for chunk in chunks
                .iter()
                .filter(|chunk| chunk["index"].as_u64().unwrap_or(0) == 0)
            {
                choice.delta(&chunk["delta"], endpoint.kind, sink).await?;
                if let Some(reason) = chunk["finish_reason"].as_str() {
                    choice.finish = Some(match reason {
                        "stop" => FinishReason::Stop,
                        "tool_calls" | "function_call" => FinishReason::ToolCalls,
                        "length" => FinishReason::Length,
                        "content_filter" => FinishReason::ContentFilter,
                        "error" => FinishReason::Error,
                        _ => FinishReason::Other,
                    });
                }
            }
        }
    }
    choice.complete(endpoint.kind)
}
