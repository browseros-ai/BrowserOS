//! Shared support for the `llm_*` integration tests: a fake provider HTTP server that records
//! requests and replays canned replies, the TS-captured scenarios, and the transcripts both sides
//! send (`fixtures/llm/capture-ts.ts` `FIRST_STEP` / `CONTINUATION`).

#![allow(dead_code)]

use axum::body::Body;
use axum::extract::{Request, State};
use axum::response::Response;
use browseros_agent::llm::{
    HttpStack, LlmConfig, ModelError, ModelEvent, ModelRequest, ModelStream, ProviderKind,
    ReasoningEffort, ReasoningSettings, ReasoningSummary, ToolDefinition, provider_options,
};
use browseros_agent::messages::{
    AssistantPart, ModelMessage, ToolContentItem, ToolMessagePart, ToolResultOutput,
    ToolResultPart, UserPart,
};
use futures_util::StreamExt;
use serde_json::{Value, json};
use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};

pub const SYSTEM: &str = "You are BrowserOS, a browser agent.";
/// A 48x32 crimson PNG (the capture script's screenshot).
pub const RED_PNG_BASE64: &str = "iVBORw0KGgoAAAANSUhEUgAAADAAAAAgCAIAAADbtmxLAAAAOElEQVR4nO3OQQ0AMAgEMEQgB/8qJmYuOB5NKqD1ek6p+EBISEgoPRASEhJKD4SEhITSAyEhoWUfDmAIarwCNQUAAAAASUVORK5CYII=";

const TS_REQUESTS: &str = include_str!("../fixtures/llm/ts-requests.json");

/// One recorded request.
#[derive(Debug, Clone)]
pub struct Captured {
    pub method: String,
    pub path_and_query: String,
    /// Lower-case names; repeated headers joined with `, `.
    pub headers: BTreeMap<String, String>,
    pub body: Value,
}

impl Captured {
    pub fn to_json(&self) -> Value {
        json!({
            "method": self.method,
            "path": self.path_and_query,
            "headers": self.headers,
            "body": self.body,
        })
    }
}

/// What the fake provider answers, in order; the last reply repeats.
#[derive(Debug, Clone)]
pub enum Reply {
    Ok {
        content_type: String,
        body: String,
    },
    Error {
        status: u16,
        headers: Vec<(String, String)>,
        body: String,
    },
    /// A 200 stream that sends `first` and then never ends (a provider stuck mid-reply).
    Stall {
        content_type: String,
        first: String,
    },
}

impl Reply {
    pub fn sse(events: &[Value]) -> Self {
        let mut body = String::new();
        for event in events {
            body.push_str(&format!("data: {event}\n\n"));
        }
        Self::Ok {
            content_type: "text/event-stream".to_owned(),
            body,
        }
    }

    /// SSE with `event:` lines (Anthropic, OpenAI Responses).
    pub fn named_sse(events: &[(&str, Value)]) -> Self {
        let mut body = String::new();
        for (name, data) in events {
            body.push_str(&format!("event: {name}\ndata: {data}\n\n"));
        }
        Self::Ok {
            content_type: "text/event-stream".to_owned(),
            body,
        }
    }

    pub fn raw_sse(body: &str) -> Self {
        Self::Ok {
            content_type: "text/event-stream".to_owned(),
            body: body.to_owned(),
        }
    }

    pub fn error(status: u16, body: &str) -> Self {
        Self::Error {
            status,
            headers: vec![("content-type".to_owned(), "application/json".to_owned())],
            body: body.to_owned(),
        }
    }

    pub fn error_with_headers(status: u16, headers: &[(&str, &str)], body: &str) -> Self {
        Self::Error {
            status,
            headers: headers
                .iter()
                .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
                .collect(),
            body: body.to_owned(),
        }
    }
}

struct FakeState {
    captured: Mutex<Vec<Captured>>,
    replies: Mutex<VecDeque<Reply>>,
}

/// A local provider endpoint. Dropping it stops the server.
pub struct Fake {
    origin: String,
    state: Arc<FakeState>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Fake {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Fake {
    pub async fn start(replies: Vec<Reply>) -> anyhow::Result<Self> {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let origin = format!("http://{}", listener.local_addr()?);
        let state = Arc::new(FakeState {
            captured: Mutex::new(Vec::new()),
            replies: Mutex::new(replies.into()),
        });
        let app = axum::Router::new()
            .fallback(handle)
            .with_state(Arc::clone(&state));
        let task = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        Ok(Self {
            origin,
            state,
            task,
        })
    }

    pub fn origin(&self) -> &str {
        &self.origin
    }

    pub fn stack(&self) -> anyhow::Result<HttpStack> {
        Ok(HttpStack::new().with_test_origin(&self.origin)?)
    }

    pub fn requests(&self) -> Vec<Captured> {
        self.state
            .captured
            .lock()
            .map(|captured| captured.clone())
            .unwrap_or_default()
    }
}

async fn handle(State(state): State<Arc<FakeState>>, request: Request) -> Response {
    let (parts, body) = request.into_parts();
    let bytes = axum::body::to_bytes(body, usize::MAX)
        .await
        .unwrap_or_default();
    let mut headers = BTreeMap::<String, String>::new();
    for (name, value) in &parts.headers {
        let value = String::from_utf8_lossy(value.as_bytes()).into_owned();
        headers
            .entry(name.as_str().to_owned())
            .and_modify(|existing| {
                existing.push_str(", ");
                existing.push_str(&value);
            })
            .or_insert(value);
    }
    let body = serde_json::from_slice(&bytes)
        .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into_owned()));
    if let Ok(mut captured) = state.captured.lock() {
        captured.push(Captured {
            method: parts.method.to_string(),
            path_and_query: parts
                .uri
                .path_and_query()
                .map(|path| path.as_str().to_owned())
                .unwrap_or_default(),
            headers,
            body,
        });
    }
    let reply = state
        .replies
        .lock()
        .ok()
        .and_then(|mut replies| {
            if replies.len() > 1 {
                replies.pop_front()
            } else {
                replies.front().cloned()
            }
        })
        .unwrap_or_else(|| Reply::error(500, r#"{"error":{"message":"no reply scripted"}}"#));
    let mut builder = Response::builder();
    let body = match reply {
        Reply::Ok { content_type, body } => {
            builder = builder.status(200).header("content-type", content_type);
            body
        }
        Reply::Error {
            status,
            headers,
            body,
        } => {
            builder = builder.status(status);
            for (name, value) in headers {
                builder = builder.header(name, value);
            }
            body
        }
        Reply::Stall {
            content_type,
            first,
        } => {
            let chunks = futures_util::stream::once(async move {
                Ok::<_, std::convert::Infallible>(bytes::Bytes::from(first))
            })
            .chain(futures_util::stream::pending());
            return builder
                .status(200)
                .header("content-type", content_type)
                .body(Body::from_stream(chunks))
                .unwrap_or_else(|_| Response::new(Body::empty()));
        }
    };
    builder
        .body(Body::from(body))
        .unwrap_or_else(|_| Response::new(Body::empty()))
}

/// Every `<scenario>/<step>` entry the TS capture recorded, in file order.
pub fn scenarios() -> anyhow::Result<Vec<(String, Value)>> {
    let Value::Object(entries) = serde_json::from_str::<Value>(TS_REQUESTS)? else {
        anyhow::bail!("ts-requests.json is not an object");
    };
    Ok(entries.into_iter().collect())
}

pub fn ts_entry(name: &str) -> anyhow::Result<Value> {
    scenarios()?
        .into_iter()
        .find(|(key, _)| key == name)
        .map(|(_, entry)| entry)
        .ok_or_else(|| anyhow::anyhow!("no TS scenario {name}"))
}

pub struct Scenario {
    pub config: LlmConfig,
    pub settings: ReasoningSettings,
}

fn string(config: &Value, key: &str) -> Option<String> {
    config.get(key).and_then(Value::as_str).map(str::to_owned)
}

/// The Rust config for a TS `ResolvedAgentConfig` (`supportsReasoning` defaults to true, as the
/// chat request schema does).
pub fn scenario(entry: &Value) -> anyhow::Result<Scenario> {
    let config = &entry["config"];
    let provider: ProviderKind = string(config, "provider").unwrap_or_default().parse()?;
    let headers = config
        .get("headers")
        .and_then(Value::as_object)
        .map(|headers| {
            headers
                .iter()
                .map(|(name, value)| (name.clone(), value.as_str().unwrap_or_default().to_owned()))
                .collect::<BTreeMap<_, _>>()
        });
    Ok(Scenario {
        config: LlmConfig {
            provider,
            model: string(config, "model").unwrap_or_default(),
            api_key: string(config, "apiKey"),
            base_url: string(config, "baseUrl"),
            headers,
            resource_name: string(config, "resourceName"),
            region: string(config, "region"),
            access_key_id: string(config, "accessKeyId"),
            secret_access_key: string(config, "secretAccessKey"),
            session_token: string(config, "sessionToken"),
            account_id: string(config, "accountId"),
            upstream_provider: None,
            conversation_id: string(config, "conversationId").unwrap_or_default(),
        },
        settings: ReasoningSettings {
            supports_reasoning: config.get("supportsReasoning").and_then(Value::as_bool)
                != Some(false),
            effort: string(config, "reasoningEffort")
                .and_then(|effort| ReasoningEffort::parse(&effort)),
            summary: string(config, "reasoningSummary")
                .and_then(|summary| ReasoningSummary::parse(&summary))
                .unwrap_or_default(),
        },
    })
}

pub fn user(text: &str) -> ModelMessage {
    ModelMessage::User {
        content: vec![UserPart::Text {
            text: text.to_owned(),
            provider_options: None,
        }],
    }
}

pub fn first_step() -> Vec<ModelMessage> {
    vec![user("What time is it?")]
}

/// `CONTINUATION`: the user turn, a text + tool-call step, and its text + image result.
pub fn continuation() -> Vec<ModelMessage> {
    vec![
        user("What time is it?"),
        ModelMessage::Assistant {
            content: vec![
                AssistantPart::Text {
                    text: "Checking.".to_owned(),
                    provider_options: None,
                },
                AssistantPart::ToolCall {
                    tool_call_id: "call_1".to_owned(),
                    tool_name: "get_time".to_owned(),
                    input: Some(json!({})),
                    provider_executed: None,
                    provider_options: None,
                },
            ],
        },
        ModelMessage::Tool {
            content: vec![ToolMessagePart::ToolResult(ToolResultPart {
                tool_call_id: "call_1".to_owned(),
                tool_name: "get_time".to_owned(),
                output: ToolResultOutput::Content {
                    value: vec![
                        ToolContentItem::Text {
                            text: r#"{"utc":"2026-10-15T04:47:54Z"}"#.to_owned(),
                            extra: Default::default(),
                        },
                        // `{type:'file', mediaType, data:{type:'data', data}}`, as in the capture.
                        ToolContentItem::File {
                            data: json!({ "type": "data", "data": RED_PNG_BASE64 }),
                            media_type: "image/png".to_owned(),
                            filename: None,
                            extra: Default::default(),
                        },
                    ],
                },
                provider_options: None,
            })],
        },
    ]
}

/// The `ModelEvent`s of a drained stream, failing on the first error.
pub fn events(items: Vec<Result<ModelEvent, ModelError>>) -> anyhow::Result<Vec<ModelEvent>> {
    items
        .into_iter()
        .map(|item| item.map_err(|error| anyhow::anyhow!("stream error: {error:?}")))
        .collect()
}

/// The transcript step the loop would build from a step's events, AI SDK style: each part's
/// `provider_metadata` becomes its `provider_options`.
pub fn assistant_from(events: &[ModelEvent]) -> ModelMessage {
    let mut text: BTreeMap<String, String> = BTreeMap::new();
    let mut content = Vec::new();
    // Part id → index of its latest ended part, for late `PartMetadata` (as the loop keeps it).
    let mut ended: BTreeMap<String, usize> = BTreeMap::new();
    for event in events {
        match event {
            ModelEvent::PartMetadata {
                id,
                provider_metadata,
            } => {
                if let Some(
                    AssistantPart::Text {
                        provider_options, ..
                    }
                    | AssistantPart::Reasoning {
                        provider_options, ..
                    },
                ) = ended.get(id).and_then(|index| content.get_mut(*index))
                {
                    *provider_options = Some(provider_metadata.clone());
                }
            }
            ModelEvent::TextDelta { id, delta } | ModelEvent::ReasoningDelta { id, delta, .. } => {
                text.entry(id.clone()).or_default().push_str(delta);
            }
            ModelEvent::TextEnd {
                id,
                provider_metadata,
            } => {
                ended.insert(id.clone(), content.len());
                content.push(AssistantPart::Text {
                    text: text.get(id).cloned().unwrap_or_default(),
                    provider_options: provider_metadata.clone(),
                });
            }
            ModelEvent::ReasoningEnd {
                id,
                provider_metadata,
            } => {
                ended.insert(id.clone(), content.len());
                content.push(AssistantPart::Reasoning {
                    text: text.get(id).cloned().unwrap_or_default(),
                    provider_options: provider_metadata.clone(),
                });
            }
            ModelEvent::ToolCall {
                provider_call_id,
                tool_name,
                input_json,
                provider_metadata,
                ..
            } => content.push(AssistantPart::ToolCall {
                tool_call_id: provider_call_id.clone(),
                tool_name: tool_name.clone(),
                input: serde_json::from_str(input_json).ok(),
                provider_executed: None,
                provider_options: provider_metadata.clone(),
            }),
            _ => {}
        }
    }
    ModelMessage::Assistant { content }
}

pub fn tool_text_result(call_id: &str, text: &str) -> ModelMessage {
    ModelMessage::Tool {
        content: vec![ToolMessagePart::ToolResult(ToolResultPart {
            tool_call_id: call_id.to_owned(),
            tool_name: "get_time".to_owned(),
            output: ToolResultOutput::Text {
                value: text.to_owned(),
            },
            provider_options: None,
        })],
    }
}

pub fn get_time_tool() -> ToolDefinition {
    ToolDefinition {
        name: "get_time".to_owned(),
        description: "Returns the current UTC time.".to_owned(),
        input_schema: json!({ "type": "object", "properties": {}, "additionalProperties": false }),
    }
}

pub fn request_for(name: &str, scenario: &Scenario) -> ModelRequest {
    let messages = if name.ends_with("/continuation") {
        continuation()
    } else {
        first_step()
    };
    ModelRequest {
        system: Some(SYSTEM.to_owned()),
        messages,
        tools: vec![get_time_tool()],
        options: provider_options(
            scenario.config.provider,
            &scenario.config.model,
            &scenario.settings,
        ),
    }
}

/// A config for `provider` with fake credentials and, where the provider needs one, a base URL.
pub fn config(provider: ProviderKind, model: &str) -> LlmConfig {
    let needs_base = matches!(
        provider,
        ProviderKind::Ollama
            | ProviderKind::LmStudio
            | ProviderKind::OpenAiCompatible
            | ProviderKind::Moonshot
    );
    LlmConfig {
        provider,
        model: model.to_owned(),
        api_key: Some("test-key".to_owned()),
        base_url: needs_base.then(|| "http://127.0.0.1:9/v1".to_owned()),
        headers: None,
        resource_name: (provider == ProviderKind::Azure).then(|| "myres".to_owned()),
        region: Some("us-east-1".to_owned()),
        access_key_id: Some("AKIDEXAMPLE".to_owned()),
        secret_access_key: Some("secret".to_owned()),
        session_token: None,
        account_id: Some("acct_123".to_owned()),
        upstream_provider: None,
        conversation_id: "conv-1".to_owned(),
    }
}

/// Drains a `stream()` result: the events in order, or the error that refused the call.
pub async fn collect(
    result: Result<ModelStream, ModelError>,
) -> Vec<Result<ModelEvent, ModelError>> {
    match result {
        Ok(stream) => stream.collect().await,
        Err(error) => vec![Err(error)],
    }
}
