//! OpenAI Responses and the ChatGPT Codex backend share a request builder and stream reader.
//! The final output items belong to the current turn and include opaque reasoning that must
//! travel back with the matching function results.

use std::collections::{BTreeMap, BTreeSet};

use futures_util::StreamExt;
use serde_json::{Value, json};

use super::{AssistantTurn, ModelMessage, NativeOutput, Reasoning, StepRequest};
use super::{data_url, errors, headers, sse, tool_call, tool_input, tool_started, usage};
use crate::turn::{ChatError, EventSink, FinishReason, TurnEvent};

/// Where and how to speak the OpenAI Responses wire. The wire is written once; the `openai`
/// provider and the ChatGPT sign-in backend differ only in this value.
pub(super) struct ResponsesEndpoint {
    pub url: url::Url,
    pub headers: Vec<(String, String)>,
    pub dialect: ResponsesDialect,
}

#[derive(PartialEq, Eq)]
pub(super) enum ResponsesDialect {
    /// api.openai.com: the wire as documented.
    OpenAi,
    /// The ChatGPT Codex backend: `stream: true`, `store: false`; no `previous_response_id`,
    /// temperature or token limits; `instructions` never empty; no `id` on input items;
    /// encrypted reasoning requested and replayed.
    #[allow(dead_code, reason = "constructed by the sign-in implementation")]
    ChatGptCodex,
}

fn body(
    dialect: &ResponsesDialect,
    model: &str,
    request: &StepRequest<'_>,
    reasoning: &Reasoning,
) -> Value {
    let mut input = Vec::new();
    for message in request.messages {
        match message {
            ModelMessage::User { text, images } => {
                let mut content = Vec::new();
                if !text.is_empty() || images.is_empty() {
                    content.push(json!({"type": "input_text", "text": text}));
                }
                content.extend(images.iter().map(|image|
                    json!({"type": "input_image", "image_url": data_url(image), "detail": "auto"})));
                input.push(json!({"role": "user", "content": content}));
            }
            ModelMessage::Assistant(turn) => match &turn.native {
                NativeOutput::Responses(items) => input.extend(items.iter().cloned()),
                _ => input.push(json!({"role": "assistant", "content": turn.text})),
            },
            ModelMessage::ToolResult { id, text } => input.push(json!({
                "type": "function_call_output", "call_id": id.0, "output": text
            })),
        }
    }
    let mut instructions = request.system;
    if *dialect == ResponsesDialect::ChatGptCodex {
        if instructions.trim().is_empty() {
            instructions = "You are a helpful assistant.";
        }
        for item in &mut input {
            if let Some(item) = item.as_object_mut() {
                // These are response item identities, separate from the required call_id.
                item.remove("id");
            }
        }
    }
    // A continuation is self-contained even if the service defaults to reasoning with no
    // explicit configuration. Project encrypted content without enabling a reasoning level.
    let mut body = json!({
        "model": model,
        "input": input,
        "stream": true,
        "store": false,
        "include": ["reasoning.encrypted_content"]
    });
    if !instructions.is_empty() {
        body["instructions"] = json!(instructions);
    }
    if let Some(effort) = reasoning.effort(
        request.effort,
        &["minimal", "low", "medium", "high", "xhigh", "max"],
    ) {
        let summary = reasoning
            .summary
            .as_deref()
            .filter(|s| !s.is_empty())
            .unwrap_or("auto");
        body["reasoning"] = json!({"effort": effort, "summary": summary});
    }
    if !request.tools.is_empty() {
        body["tools"] = Value::Array(
            request
                .tools
                .iter()
                .map(|tool| {
                    json!({
                        "type": "function", "name": tool.name, "description": tool.description,
                        "parameters": tool.input_schema, "strict": false
                    })
                })
                .collect(),
        );
    }
    body
}

/// One Responses call. Same contract as `Model::step`.
pub(super) async fn step(
    http: &reqwest::Client,
    endpoint: &ResponsesEndpoint,
    model: &str,
    request: &StepRequest<'_>,
    sink: &mut dyn EventSink,
) -> Result<AssistantTurn, ChatError> {
    step_configured(http, endpoint, model, request, &Reasoning::default(), sink).await
}

pub(super) async fn step_configured(
    http: &reqwest::Client,
    endpoint: &ResponsesEndpoint,
    model: &str,
    request: &StepRequest<'_>,
    reasoning: &Reasoning,
    sink: &mut dyn EventSink,
) -> Result<AssistantTurn, ChatError> {
    let headers = headers(&endpoint.headers)?;
    let response = sse::post(
        http,
        &endpoint.url,
        &headers,
        &body(&endpoint.dialect, model, request, reasoning),
    )
    .await?;
    let mut events = std::pin::pin!(sse::events(response));
    let mut state = Output::default();
    while let Some(data) = events.next().await {
        let data = data?;
        if data.trim() == "[DONE]" {
            break;
        }
        let event = sse::json(&data)?;
        match event["type"].as_str() {
            Some("response.output_text.delta" | "response.refusal.delta") => {
                if let Some(text) = event["delta"].as_str().filter(|text| !text.is_empty()) {
                    state.text.push_str(text);
                    sink.emit(TurnEvent::Text(text.into())).await;
                }
            }
            Some("response.reasoning_summary_text.delta") => {
                if let Some(text) = event["delta"].as_str().filter(|text| !text.is_empty()) {
                    sink.emit(TurnEvent::Reasoning(text.into())).await;
                }
            }
            Some("response.output_item.added" | "response.output_item.done") => {
                let item = &event["item"];
                state.announce(item, sink).await;
                if event["type"] == "response.output_item.done" {
                    let index = event["output_index"].as_u64().ok_or_else(|| {
                        errors::protocol("A Responses output item arrived without its index.")
                    })?;
                    state.items.insert(index, item.clone());
                }
            }
            Some("response.completed" | "response.incomplete") => {
                return state.complete(&event, sink).await;
            }
            Some("response.failed") => return Err(errors::stream(&event["response"], &headers)),
            Some("error") => return Err(errors::stream(&event, &headers)),
            _ => {}
        }
    }
    Err(errors::disconnected())
}

#[derive(Default)]
struct Output {
    text: String,
    items: BTreeMap<u64, Value>,
    announced: BTreeSet<String>,
}

impl Output {
    async fn announce(&mut self, item: &Value, sink: &mut dyn EventSink) {
        if item["type"] == "function_call"
            && let (Some(id), Some(name)) = (item["call_id"].as_str(), item["name"].as_str())
            && !id.is_empty()
            && !name.is_empty()
            && self.announced.insert(id.into())
        {
            tool_started(sink, id, name).await;
        }
    }

    async fn complete(
        mut self,
        event: &Value,
        sink: &mut dyn EventSink,
    ) -> Result<AssistantTurn, ChatError> {
        let response = &event["response"];
        // The final response is authoritative when the service includes its output array.
        // Some Responses endpoints send only the terminal usage and status after item.done.
        let items: Vec<_> = match response["output"].as_array() {
            Some(items) if !items.is_empty() => items.clone(),
            _ => std::mem::take(&mut self.items).into_values().collect(),
        };
        let mut finish =
            if event["type"] == "response.incomplete" || response["status"] == "incomplete" {
                match response["incomplete_details"]["reason"].as_str() {
                    Some("max_output_tokens") => FinishReason::Length,
                    Some("content_filter") => FinishReason::ContentFilter,
                    _ => FinishReason::Other,
                }
            } else {
                FinishReason::Stop
            };
        let mut calls = Vec::new();
        if finish == FinishReason::Stop {
            for item in items.iter().filter(|item| item["type"] == "function_call") {
                self.announce(item, sink).await;
                calls.push(tool_call(
                    &item["call_id"],
                    &item["name"],
                    tool_input(&item["arguments"])?,
                )?);
            }
            if !calls.is_empty() {
                finish = FinishReason::ToolCalls;
            }
        }
        Ok(AssistantTurn {
            text: self.text,
            calls,
            finish,
            usage: usage(
                &response["usage"]["input_tokens"],
                &response["usage"]["output_tokens"],
            ),
            native: NativeOutput::Responses(items),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Effort;
    use crate::model::fake_provider::{FakeProvider, Reply};
    use crate::turn::ToolCallId;

    #[tokio::test]
    async fn codex_request_rewrites_input_and_replays_encrypted_reasoning() -> anyhow::Result<()> {
        let fake = FakeProvider::new(vec![Reply::events(&[
            json!({"type":"response.output_text.delta","delta":"ok"}),
            json!({"type":"response.completed","response":{"status":"completed"}}),
        ])])
        .await?;
        let url = url::Url::parse(&format!("{}/backend-api/codex/responses", fake.base))?;
        let assistant = AssistantTurn {
            native: NativeOutput::Responses(vec![
                json!({"type":"reasoning","id":"rs_1","summary":[],"encrypted_content":"opaque"}),
                json!({"type":"function_call","id":"fc_1","call_id":"call_1","name":"run","arguments":"{}"}),
            ]),
            ..AssistantTurn::text(String::new())
        };
        let messages = [
            ModelMessage::Assistant(assistant),
            ModelMessage::ToolResult {
                id: ToolCallId("call_1".into()),
                text: "ok".into(),
            },
        ];
        let endpoint = ResponsesEndpoint {
            url,
            headers: vec![],
            dialect: ResponsesDialect::ChatGptCodex,
        };
        let mut events = Vec::new();
        let result = step(
            &reqwest::Client::new(),
            &endpoint,
            "fixture-codex",
            &StepRequest {
                system: " ",
                messages: &messages,
                tools: &[],
                effort: Some(Effort::Medium),
            },
            &mut events,
        )
        .await;
        assert_eq!(result?.finish(), FinishReason::Stop);
        assert_eq!(events, [TurnEvent::Text("ok".into())]);
        let calls = fake
            .calls
            .lock()
            .map_err(|_| anyhow::anyhow!("capture lock"))?;
        assert_eq!(calls[0].uri.path(), "/backend-api/codex/responses");
        let body = &calls[0].body;
        assert_eq!(body["stream"], true, "streaming required");
        assert_eq!(body["store"], false, "server-side storage disabled");
        assert!(
            body.get("previous_response_id").is_none(),
            "self-contained history"
        );
        assert!(body.get("temperature").is_none(), "no temperature");
        for field in ["max_tokens", "max_output_tokens", "max_completion_tokens"] {
            assert!(body.get(field).is_none(), "no token limit {field}");
        }
        assert!(
            body["instructions"]
                .as_str()
                .is_some_and(|text| !text.trim().is_empty()),
            "nonempty instructions"
        );
        assert_eq!(
            body["include"],
            json!(["reasoning.encrypted_content"]),
            "request encrypted reasoning"
        );
        assert_eq!(
            body["input"],
            json!([
                {"type":"reasoning","summary":[],"encrypted_content":"opaque"},
                {"type":"function_call","call_id":"call_1","name":"run","arguments":"{}"},
                {"type":"function_call_output","call_id":"call_1","output":"ok"}
            ]),
            "remove item ids and preserve encrypted reasoning and call identities"
        );
        Ok(())
    }
}
