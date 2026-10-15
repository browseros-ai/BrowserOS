//! The built-in agent loop against scripted models and in-memory tool sources: chunk order and
//! shapes (recon A11.4), tool scheduling, retries, limits, cancellation and the transcript each
//! step adds. Time is paused, so backoff and tool sleeps cost nothing.

// Scripts hold `Result<ModelEvent, ModelError>` items by value, the shape `LanguageModel` streams;
// `ModelError` belongs to the llm module and its size is not this test's to change.
#![allow(clippy::result_large_err)]

use browseros_agent::{
    Agent, AgentBuildError, AgentConfig, ModelInfo, TurnInput,
    llm::{
        LanguageModel, ModelError, ModelEvent, ModelFinishReason, ModelRequest, ModelStream,
        ProviderKind, ReasoningSettings, ScriptedModel, Usage,
    },
    messages::{AssistantPart, ModelMessage, ToolMessagePart, ToolResultOutput, UserPart},
    prompt::PromptInputs,
    tools::{
        SourceSpec, ToolCallError, ToolKind, ToolOutput, ToolSource, ToolSourceError, ToolSpec,
        nudge_tools,
    },
};
use futures_util::StreamExt;
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;
use ui_message_stream::{FinishReason, UiChunk, UiMessage};

type TestResult = Result<(), Box<dyn std::error::Error>>;

// ───── scripted model with requests, cancellation and gates ─────

enum Step {
    Events(Vec<Result<ModelEvent, ModelError>>),
    /// `stream()` itself fails (an HTTP error before any body).
    Fail(ModelError),
    /// The events, then nothing until the model's token is cancelled.
    Hang(Vec<Result<ModelEvent, ModelError>>),
    /// `before`, then wait for the gate, then `after`.
    Gated(
        Vec<Result<ModelEvent, ModelError>>,
        Arc<Notify>,
        Vec<Result<ModelEvent, ModelError>>,
    ),
}

#[derive(Default)]
struct TestModel {
    steps: Mutex<VecDeque<Step>>,
    requests: Mutex<Vec<ModelRequest>>,
    tokens: Mutex<Vec<CancellationToken>>,
}

impl TestModel {
    fn new(steps: Vec<Step>) -> Arc<Self> {
        Arc::new(Self {
            steps: Mutex::new(steps.into()),
            ..Self::default()
        })
    }

    fn requests(&self) -> Vec<ModelRequest> {
        self.requests.lock().map(|r| r.clone()).unwrap_or_default()
    }

    fn calls(&self) -> usize {
        self.requests().len()
    }

    fn token(&self, index: usize) -> Option<CancellationToken> {
        self.tokens.lock().ok()?.get(index).cloned()
    }
}

#[async_trait::async_trait]
impl LanguageModel for TestModel {
    fn provider(&self) -> ProviderKind {
        ProviderKind::OpenAi
    }

    fn model_id(&self) -> &str {
        "test-model"
    }

    fn supports_media_in_tool_results(&self) -> bool {
        true
    }

    async fn stream(
        &self,
        request: ModelRequest,
        cancel: CancellationToken,
    ) -> Result<ModelStream, ModelError> {
        if let Ok(mut requests) = self.requests.lock() {
            requests.push(request);
        }
        if let Ok(mut tokens) = self.tokens.lock() {
            tokens.push(cancel.clone());
        }
        let step = self
            .steps
            .lock()
            .ok()
            .and_then(|mut steps| steps.pop_front())
            .ok_or_else(|| ModelError::Config("test model has no steps left".to_owned()))?;
        Ok(match step {
            Step::Events(events) => Box::pin(futures_util::stream::iter(events)),
            Step::Fail(error) => return Err(error),
            Step::Hang(events) => Box::pin(futures_util::stream::iter(events).chain(
                futures_util::stream::once(async move {
                    cancel.cancelled().await;
                    Err(ModelError::Cancelled)
                }),
            )),
            Step::Gated(before, gate, after) => Box::pin(
                futures_util::stream::iter(before)
                    .chain(
                        futures_util::stream::once(async move {
                            gate.notified().await;
                            None
                        })
                        .filter_map(|item| async move { item }),
                    )
                    .chain(futures_util::stream::iter(after)),
            ),
        })
    }
}

// ───── in-memory tool source ─────

#[derive(Default)]
struct TestTools {
    label: String,
    specs: Vec<ToolSpec>,
    unavailable: bool,
    calls: Mutex<Vec<(String, Value)>>,
    cancelled: Mutex<Vec<String>>,
    closed: AtomicBool,
}

impl TestTools {
    fn new(label: &str, specs: Vec<ToolSpec>) -> Arc<Self> {
        Arc::new(Self {
            label: label.to_owned(),
            specs,
            ..Self::default()
        })
    }

    fn calls(&self) -> Vec<String> {
        self.calls
            .lock()
            .map(|calls| calls.iter().map(|(name, _)| name.clone()).collect())
            .unwrap_or_default()
    }

    fn cancelled(&self) -> Vec<String> {
        self.cancelled.lock().map(|c| c.clone()).unwrap_or_default()
    }
}

#[async_trait::async_trait]
impl ToolSource for TestTools {
    fn label(&self) -> &str {
        &self.label
    }

    async fn tools(&self) -> Result<Vec<ToolSpec>, ToolSourceError> {
        if self.unavailable {
            return Err(ToolSourceError::Unavailable {
                label: self.label.clone(),
                message: "down".to_owned(),
            });
        }
        Ok(self.specs.clone())
    }

    /// `{"ms": n}` sleeps first; `{"wait": true}` waits for cancellation; `{"fail": true}` errors.
    async fn call(
        &self,
        name: &str,
        input: Value,
        cancel: CancellationToken,
    ) -> Result<ToolOutput, ToolCallError> {
        if let Ok(mut calls) = self.calls.lock() {
            calls.push((name.to_owned(), input.clone()));
        }
        let record_cancel = || {
            if let Ok(mut cancelled) = self.cancelled.lock() {
                cancelled.push(name.to_owned());
            }
            Err(ToolCallError("cancelled".to_owned()))
        };
        if input.get("wait").is_some() {
            cancel.cancelled().await;
            return record_cancel();
        }
        if let Some(ms) = input.get("ms").and_then(Value::as_u64) {
            tokio::select! {
                () = cancel.cancelled() => return record_cancel(),
                () = tokio::time::sleep(Duration::from_millis(ms)) => {}
            }
        }
        if input.get("fail").is_some() {
            return Err(ToolCallError("tool exploded".to_owned()));
        }
        Ok(ToolOutput {
            ui: json!({ "content": [{ "type": "text", "text": format!("{name} done") }], "isError": false }),
            model: ToolResultOutput::Text {
                value: format!("{name} done"),
            },
        })
    }

    async fn close(&self) {
        self.closed.store(true, Ordering::SeqCst);
    }
}

fn static_tool(name: &str) -> ToolSpec {
    ToolSpec {
        name: name.to_owned(),
        description: format!("{name} tool"),
        input_schema: json!({
            "type": "object",
            "properties": {
                "ms": { "type": "number" },
                "wait": { "type": "boolean" },
                "fail": { "type": "boolean" },
                "query": { "type": "string" },
            },
            "required": ["query"],
            "additionalProperties": false,
        }),
        kind: ToolKind::Static,
    }
}

fn dynamic_tool(name: &str) -> ToolSpec {
    ToolSpec {
        name: name.to_owned(),
        description: format!("{name} tool"),
        input_schema: json!({ "type": "object", "properties": {}, "additionalProperties": false }),
        kind: ToolKind::Dynamic {
            title: Some(format!("{name} title")),
            metadata: json!({ "clientName": "ai-sdk-mcp-client", "toolName": name }),
        },
    }
}

// ───── helpers ─────

fn info() -> ModelInfo {
    ModelInfo {
        provider: ProviderKind::OpenAi,
        model_id: "test-model".to_owned(),
        context_window: None,
        supports_images: true,
    }
}

async fn build(
    model: Arc<dyn LanguageModel>,
    sources: Vec<SourceSpec>,
    max_steps: u32,
    prompt: PromptInputs,
) -> Result<Agent, AgentBuildError> {
    Agent::build(AgentConfig {
        model,
        info: info(),
        reasoning: ReasoningSettings::default(),
        prompt,
        sources,
        max_steps,
    })
    .await
}

fn source(tools: &Arc<TestTools>, required: bool) -> SourceSpec {
    SourceSpec {
        source: Arc::clone(tools) as Arc<dyn ToolSource>,
        required,
    }
}

fn input(text: &str) -> TurnInput {
    TurnInput {
        history: vec![UiMessage::user(
            "user-1".to_owned(),
            Some(text.to_owned()),
            Vec::new(),
        )],
        prompt_text: format!("<USER_QUERY>\n{text}\n</USER_QUERY>"),
        message_id: "assistant-1".to_owned(),
    }
}

async fn run(agent: &Agent, cancel: CancellationToken) -> Vec<UiChunk> {
    agent.run_turn(input("hello"), cancel).collect().await
}

fn kinds(chunks: &[UiChunk]) -> Vec<String> {
    chunks
        .iter()
        .map(|chunk| {
            serde_json::to_value(chunk)
                .ok()
                .and_then(|value| value.get("type").and_then(Value::as_str).map(str::to_owned))
                .unwrap_or_default()
        })
        .collect()
}

fn text(id: &str, delta: &str) -> Vec<Result<ModelEvent, ModelError>> {
    vec![
        Ok(ModelEvent::TextStart {
            id: id.to_owned(),
            provider_metadata: None,
        }),
        Ok(ModelEvent::TextDelta {
            id: id.to_owned(),
            delta: delta.to_owned(),
        }),
        Ok(ModelEvent::TextEnd {
            id: id.to_owned(),
            provider_metadata: None,
        }),
    ]
}

fn call(id: &str, tool: &str, input: Value) -> Result<ModelEvent, ModelError> {
    Ok(ModelEvent::ToolCall {
        id: id.to_owned(),
        provider_call_id: id.to_owned(),
        tool_name: tool.to_owned(),
        input_json: input.to_string(),
        provider_metadata: None,
    })
}

fn finish(reason: ModelFinishReason) -> Result<ModelEvent, ModelError> {
    Ok(ModelEvent::Finish {
        reason,
        usage: Usage::default(),
    })
}

fn rate_limited() -> ModelError {
    ModelError::Api {
        status: 429,
        message: "Too Many Requests".to_owned(),
        response_body: None,
        response_headers: vec![("retry-after-ms".to_owned(), "10".to_owned())],
        data: None,
        retryable: true,
    }
}

fn envelope_code(error_text: &str) -> Option<String> {
    let value: Value = serde_json::from_str(error_text).ok()?;
    value.get("error")?.get("code")?.as_str().map(str::to_owned)
}

// ───── tests ─────

#[tokio::test]
async fn scripted_mock_streams_one_text_step() -> TestResult {
    let agent = build(
        Arc::new(ScriptedModel::mock()),
        Vec::new(),
        100,
        PromptInputs::default(),
    )
    .await?;
    let chunks = run(&agent, CancellationToken::new()).await;
    assert_eq!(
        chunks,
        vec![
            UiChunk::Start {
                message_metadata: None,
                message_id: Some("assistant-1".to_owned()),
            },
            UiChunk::StartStep,
            UiChunk::TextStart {
                id: "mock-text".to_owned(),
                provider_metadata: None,
            },
            UiChunk::TextDelta {
                id: "mock-text".to_owned(),
                delta: "Mock BrowserOS test response.".to_owned(),
                provider_metadata: None,
            },
            UiChunk::TextEnd {
                id: "mock-text".to_owned(),
                provider_metadata: None,
            },
            UiChunk::FinishStep,
            UiChunk::Finish {
                finish_reason: Some(FinishReason::Stop),
                message_metadata: None,
            },
        ]
    );
    Ok(())
}

/// Two calls in one step: inputs first, outputs in completion order (the fast one first), then a
/// second step that sees both results in call order.
#[tokio::test(start_paused = true)]
async fn multi_step_runs_tools_concurrently_in_completion_order() -> TestResult {
    let tools = TestTools::new("internal", vec![dynamic_tool("slow"), static_tool("fast")]);
    let mut step_one = vec![
        Ok(ModelEvent::ToolInputStart {
            id: "call-a".to_owned(),
            tool_name: "slow".to_owned(),
        }),
        Ok(ModelEvent::ToolInputDelta {
            id: "call-a".to_owned(),
            delta: "{\"ms\":50}".to_owned(),
        }),
        call("call-a", "slow", json!({ "ms": 50 })),
        call(
            "call-b",
            "fast",
            json!({ "ms": 5, "query": "q", "dropped": 1 }),
        ),
    ];
    step_one.push(finish(ModelFinishReason::ToolCalls));
    let mut step_two = text("t2", "Done");
    step_two.push(finish(ModelFinishReason::Stop));
    let model = TestModel::new(vec![Step::Events(step_one), Step::Events(step_two)]);
    let agent = build(
        model.clone(),
        vec![source(&tools, true)],
        100,
        PromptInputs::default(),
    )
    .await?;

    let chunks = run(&agent, CancellationToken::new()).await;
    assert_eq!(
        kinds(&chunks),
        [
            "start",
            "start-step",
            "tool-input-start",
            "tool-input-delta",
            "tool-input-available",
            "tool-input-available",
            "tool-output-available",
            "tool-output-available",
            "finish-step",
            "start-step",
            "text-start",
            "text-delta",
            "text-end",
            "finish-step",
            "finish",
        ]
    );
    // Dynamic tools carry metadata, `dynamic` and `title`; static ones none of them.
    assert_eq!(
        serde_json::to_value(&chunks[2])?,
        json!({
            "type": "tool-input-start",
            "toolCallId": "call-a",
            "toolName": "slow",
            "toolMetadata": { "clientName": "ai-sdk-mcp-client", "toolName": "slow" },
            "dynamic": true,
            "title": "slow title",
        })
    );
    assert_eq!(
        serde_json::to_value(&chunks[5])?,
        json!({
            "type": "tool-input-available",
            "toolCallId": "call-b",
            "toolName": "fast",
            "input": { "ms": 5, "query": "q" },
        })
    );
    let output_ids: Vec<_> = chunks
        .iter()
        .filter_map(|chunk| match chunk {
            UiChunk::ToolOutputAvailable { tool_call_id, .. } => Some(tool_call_id.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(output_ids, ["call-b", "call-a"]);
    assert_eq!(
        chunks.last(),
        Some(&UiChunk::Finish {
            finish_reason: Some(FinishReason::Stop),
            message_metadata: None,
        })
    );

    let requests = model.requests();
    assert_eq!(requests.len(), 2);
    let second = &requests[1].messages;
    let Some(ModelMessage::Assistant { content }) = second.get(second.len() - 2) else {
        return Err(format!("expected the step's assistant message: {second:?}").into());
    };
    assert!(matches!(&content[..], [
        AssistantPart::ToolCall { tool_call_id: a, .. },
        AssistantPart::ToolCall { tool_call_id: b, .. },
    ] if a == "call-a" && b == "call-b"));
    let Some(ModelMessage::Tool { content }) = second.last() else {
        return Err("expected the step's tool message".into());
    };
    let ids: Vec<_> = content
        .iter()
        .map(|part| match part {
            ToolMessagePart::ToolResult(result) => result.tool_call_id.as_str(),
            ToolMessagePart::ToolApprovalResponse { .. } => "",
        })
        .collect();
    assert_eq!(
        ids,
        ["call-a", "call-b"],
        "results are sorted by call order"
    );
    Ok(())
}

/// Tools start only once the model's step finishes, never while it is still streaming.
#[tokio::test]
async fn tools_start_after_the_model_finishes_the_step() -> TestResult {
    let tools = TestTools::new("internal", vec![dynamic_tool("act")]);
    let gate = Arc::new(Notify::new());
    let model = TestModel::new(vec![
        Step::Gated(
            vec![call("c1", "act", json!({}))],
            Arc::clone(&gate),
            vec![finish(ModelFinishReason::ToolCalls)],
        ),
        Step::Events(vec![finish(ModelFinishReason::Stop)]),
    ]);
    let agent = build(
        model,
        vec![source(&tools, true)],
        100,
        PromptInputs::default(),
    )
    .await?;
    let mut stream = agent.run_turn(input("go"), CancellationToken::new());
    let mut seen = Vec::new();
    while let Some(chunk) = stream.next().await {
        let available = matches!(chunk, UiChunk::ToolInputAvailable { .. });
        seen.push(chunk);
        if available {
            break;
        }
    }
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(
        tools.calls().is_empty(),
        "the tool ran before the step finished"
    );
    gate.notify_one();
    let rest: Vec<UiChunk> = stream.collect().await;
    assert_eq!(tools.calls(), ["act"]);
    assert!(
        rest.iter()
            .any(|chunk| matches!(chunk, UiChunk::ToolOutputAvailable { .. }))
    );
    Ok(())
}

/// A step that stops for any reason other than stop/tool-calls leaves its calls unexecuted and
/// ends the turn with that reason.
#[tokio::test]
async fn length_finish_skips_tool_execution() -> TestResult {
    let tools = TestTools::new("internal", vec![dynamic_tool("act")]);
    let model = TestModel::new(vec![Step::Events(vec![
        call("c1", "act", json!({})),
        finish(ModelFinishReason::Length),
    ])]);
    let agent = build(
        model.clone(),
        vec![source(&tools, true)],
        100,
        PromptInputs::default(),
    )
    .await?;
    let chunks = run(&agent, CancellationToken::new()).await;
    assert_eq!(
        kinds(&chunks),
        [
            "start",
            "start-step",
            "tool-input-available",
            "finish-step",
            "finish"
        ]
    );
    assert_eq!(
        chunks.last(),
        Some(&UiChunk::Finish {
            finish_reason: Some(FinishReason::Length),
            message_metadata: None,
        })
    );
    assert!(tools.calls().is_empty());
    assert_eq!(model.calls(), 1);
    Ok(())
}

/// Unknown tools and invalid static input fail at once (input error + output error) without
/// executing; the turn continues because every call has an output.
#[tokio::test]
async fn invalid_tool_calls_fail_without_executing() -> TestResult {
    let tools = TestTools::new("internal", vec![static_tool("lookup")]);
    let mut done = text("t", "ok");
    done.push(finish(ModelFinishReason::Stop));
    let model = TestModel::new(vec![
        Step::Events(vec![
            call("c1", "nope", json!({ "x": 1 })),
            call("c2", "lookup", json!({ "query": 5 })),
            Ok(ModelEvent::ToolCall {
                id: "c3".to_owned(),
                provider_call_id: "c3".to_owned(),
                tool_name: "lookup".to_owned(),
                input_json: "{not json".to_owned(),
                provider_metadata: None,
            }),
            finish(ModelFinishReason::ToolCalls),
        ]),
        Step::Events(done),
    ]);
    let agent = build(
        model.clone(),
        vec![source(&tools, true)],
        100,
        PromptInputs::default(),
    )
    .await?;
    let chunks = run(&agent, CancellationToken::new()).await;
    assert_eq!(
        kinds(&chunks),
        [
            "start",
            "start-step",
            "tool-input-error",
            "tool-output-error",
            "tool-input-error",
            "tool-output-error",
            "tool-input-error",
            "tool-output-error",
            "finish-step",
            "start-step",
            "text-start",
            "text-delta",
            "text-end",
            "finish-step",
            "finish",
        ]
    );
    assert!(tools.calls().is_empty());
    match (&chunks[2], &chunks[3], &chunks[4], &chunks[6]) {
        (
            UiChunk::ToolInputError {
                dynamic: Some(true),
                input,
                error_text,
                ..
            },
            UiChunk::ToolOutputError {
                dynamic: Some(true),
                ..
            },
            UiChunk::ToolInputError { dynamic: None, .. },
            UiChunk::ToolInputError {
                input: Value::String(raw),
                ..
            },
        ) => {
            assert_eq!(input, &json!({ "x": 1 }));
            assert_eq!(envelope_code(error_text).as_deref(), Some("unknown"));
            assert_eq!(raw, "{not json");
        }
        other => return Err(format!("unexpected invalid-call chunks: {other:?}").into()),
    }
    // The failures go back to the model as error results, in call order.
    let second = model
        .requests()
        .get(1)
        .map(|r| r.messages.clone())
        .unwrap_or_default();
    let Some(ModelMessage::Tool { content }) = second.last() else {
        return Err("expected error results".into());
    };
    assert_eq!(content.len(), 3);
    assert!(content.iter().all(|part| matches!(
        part,
        ToolMessagePart::ToolResult(result) if matches!(result.output, ToolResultOutput::ErrorText { .. })
    )));
    Ok(())
}

#[tokio::test]
async fn thrown_tool_errors_become_output_errors() -> TestResult {
    let tools = TestTools::new("internal", vec![dynamic_tool("act")]);
    let model = TestModel::new(vec![
        Step::Events(vec![
            call("c1", "act", json!({ "fail": true })),
            finish(ModelFinishReason::ToolCalls),
        ]),
        Step::Events(vec![finish(ModelFinishReason::Stop)]),
    ]);
    let agent = build(
        model,
        vec![source(&tools, true)],
        100,
        PromptInputs::default(),
    )
    .await?;
    let chunks = run(&agent, CancellationToken::new()).await;
    let error = chunks.iter().find_map(|chunk| match chunk {
        UiChunk::ToolOutputError {
            error_text,
            dynamic,
            tool_metadata,
            ..
        } => Some((error_text.clone(), *dynamic, tool_metadata.clone())),
        _ => None,
    });
    let Some((error_text, dynamic, metadata)) = error else {
        return Err("expected a tool-output-error".into());
    };
    assert_eq!(envelope_code(&error_text).as_deref(), Some("unknown"));
    assert_eq!(dynamic, Some(true));
    assert!(metadata.is_some());
    Ok(())
}

#[tokio::test]
async fn max_steps_ends_with_the_last_steps_reason() -> TestResult {
    let tools = TestTools::new("internal", vec![dynamic_tool("act")]);
    let step = || {
        Step::Events(vec![
            call("c", "act", json!({})),
            finish(ModelFinishReason::ToolCalls),
        ])
    };
    let model = TestModel::new(vec![step(), step(), step()]);
    let agent = build(
        model.clone(),
        vec![source(&tools, true)],
        2,
        PromptInputs::default(),
    )
    .await?;
    let chunks = run(&agent, CancellationToken::new()).await;
    assert_eq!(
        kinds(&chunks).iter().filter(|k| *k == "start-step").count(),
        2
    );
    assert_eq!(model.calls(), 2);
    assert_eq!(
        chunks.last(),
        Some(&UiChunk::Finish {
            finish_reason: Some(FinishReason::ToolCalls),
            message_metadata: None,
        })
    );
    Ok(())
}

/// Failures before a step's first event are retried (twice) and leave no trace on the wire.
#[tokio::test(start_paused = true)]
async fn retries_happen_before_the_first_event() -> TestResult {
    let mut ok = text("t", "hi");
    ok.push(finish(ModelFinishReason::Stop));
    let model = TestModel::new(vec![
        Step::Events(vec![Err(rate_limited())]),
        Step::Fail(ModelError::Connection("fetch failed".to_owned())),
        Step::Events(ok),
    ]);
    let agent = build(model.clone(), Vec::new(), 100, PromptInputs::default()).await?;
    let chunks = run(&agent, CancellationToken::new()).await;
    assert_eq!(model.calls(), 3);
    assert_eq!(
        kinds(&chunks),
        [
            "start",
            "start-step",
            "text-start",
            "text-delta",
            "text-end",
            "finish-step",
            "finish"
        ]
    );
    Ok(())
}

/// Exhausted retries are the pre-stream failure shape: `start`, `error`, and nothing else.
#[tokio::test(start_paused = true)]
async fn exhausted_retries_end_with_a_pre_stream_error() -> TestResult {
    let model = TestModel::new(vec![
        Step::Fail(rate_limited()),
        Step::Fail(rate_limited()),
        Step::Fail(rate_limited()),
        Step::Events(vec![finish(ModelFinishReason::Stop)]),
    ]);
    let agent = build(model.clone(), Vec::new(), 100, PromptInputs::default()).await?;
    let chunks = run(&agent, CancellationToken::new()).await;
    assert_eq!(model.calls(), 3);
    assert_eq!(kinds(&chunks), ["start", "error"]);
    let UiChunk::Error { error_text } = &chunks[1] else {
        return Err("expected an error chunk".into());
    };
    assert!(
        serde_json::from_str::<Value>(error_text)?
            .get("error")
            .is_some()
    );
    Ok(())
}

#[tokio::test]
async fn non_retryable_failures_are_not_retried() -> TestResult {
    let model = TestModel::new(vec![
        Step::Fail(ModelError::InvalidPrompt("bad prompt".to_owned())),
        Step::Events(vec![finish(ModelFinishReason::Stop)]),
    ]);
    let agent = build(model.clone(), Vec::new(), 100, PromptInputs::default()).await?;
    assert_eq!(
        kinds(&run(&agent, CancellationToken::new()).await),
        ["start", "error"]
    );
    assert_eq!(model.calls(), 1);
    Ok(())
}

/// Once a step has produced an event, a failure is an in-stream error: never retried, and the
/// step and turn still close (`finish-step`, `finish{error}`).
#[tokio::test(start_paused = true)]
async fn errors_after_the_first_event_are_not_retried() -> TestResult {
    let mut events = text("t", "partial");
    events.truncate(2);
    events.push(Err(ModelError::Api {
        status: 500,
        message: "upstream".to_owned(),
        response_body: None,
        response_headers: Vec::new(),
        data: None,
        retryable: true,
    }));
    let model = TestModel::new(vec![
        Step::Events(events),
        Step::Events(vec![finish(ModelFinishReason::Stop)]),
    ]);
    let agent = build(model.clone(), Vec::new(), 100, PromptInputs::default()).await?;
    let chunks = run(&agent, CancellationToken::new()).await;
    assert_eq!(model.calls(), 1);
    assert_eq!(
        kinds(&chunks),
        [
            "start",
            "start-step",
            "text-start",
            "text-delta",
            "error",
            "finish-step",
            "finish"
        ]
    );
    assert_eq!(
        chunks.last(),
        Some(&UiChunk::Finish {
            finish_reason: Some(FinishReason::Error),
            message_metadata: None,
        })
    );
    Ok(())
}

#[tokio::test]
async fn an_empty_model_stream_is_a_no_output_error() -> TestResult {
    let model = TestModel::new(vec![Step::Events(Vec::new())]);
    let agent = build(model, Vec::new(), 100, PromptInputs::default()).await?;
    assert_eq!(
        kinds(&run(&agent, CancellationToken::new()).await),
        ["start", "error"]
    );
    Ok(())
}

/// Stop mid-stream: `abort` with the TS reason is the last chunk, and the model's token fires.
#[tokio::test]
async fn cancel_mid_stream_emits_abort_and_stops_the_model() -> TestResult {
    let mut events = text("t", "thinking");
    events.truncate(2);
    let model = TestModel::new(vec![Step::Hang(events)]);
    let agent = build(model.clone(), Vec::new(), 100, PromptInputs::default()).await?;
    let cancel = CancellationToken::new();
    let mut stream = agent.run_turn(input("go"), cancel.clone());
    let mut chunks = Vec::new();
    while let Some(chunk) = stream.next().await {
        let delta = matches!(chunk, UiChunk::TextDelta { .. });
        chunks.push(chunk);
        if delta {
            cancel.cancel();
        }
    }
    assert_eq!(
        kinds(&chunks),
        ["start", "start-step", "text-start", "text-delta", "abort"]
    );
    assert_eq!(
        chunks.last(),
        Some(&UiChunk::Abort {
            reason: Some("AbortError: Conversation stopped".to_owned()),
        })
    );
    assert!(model.token(0).is_some_and(|token| token.is_cancelled()));
    Ok(())
}

#[tokio::test]
async fn cancel_during_tool_execution_aborts_and_cancels_the_tool() -> TestResult {
    let tools = TestTools::new("internal", vec![dynamic_tool("act")]);
    let model = TestModel::new(vec![Step::Events(vec![
        call("c1", "act", json!({ "wait": true })),
        finish(ModelFinishReason::ToolCalls),
    ])]);
    let agent = build(
        model,
        vec![source(&tools, true)],
        100,
        PromptInputs::default(),
    )
    .await?;
    let cancel = CancellationToken::new();
    let mut stream = agent.run_turn(input("go"), cancel.clone());
    let mut chunks = Vec::new();
    while let Some(chunk) = stream.next().await {
        let available = matches!(chunk, UiChunk::ToolInputAvailable { .. });
        chunks.push(chunk);
        if available {
            // Give the tool task time to start before stopping the run.
            tokio::time::sleep(Duration::from_millis(20)).await;
            cancel.cancel();
        }
    }
    assert_eq!(kinds(&chunks).last().map(String::as_str), Some("abort"));
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert_eq!(tools.cancelled(), ["act"]);
    Ok(())
}

#[tokio::test]
async fn an_already_cancelled_run_aborts_after_start() -> TestResult {
    let agent = build(
        Arc::new(ScriptedModel::mock()),
        Vec::new(),
        100,
        PromptInputs::default(),
    )
    .await?;
    let cancel = CancellationToken::new();
    cancel.cancel();
    assert_eq!(kinds(&run(&agent, cancel).await), ["start", "abort"]);
    Ok(())
}

/// Dropping the chunk stream stops the turn and cancels running tools, silently.
#[tokio::test]
async fn dropping_the_stream_cancels_running_tools() -> TestResult {
    let tools = TestTools::new("internal", vec![dynamic_tool("act")]);
    let model = TestModel::new(vec![Step::Events(vec![
        call("c1", "act", json!({ "wait": true })),
        finish(ModelFinishReason::ToolCalls),
    ])]);
    let agent = build(
        model,
        vec![source(&tools, true)],
        100,
        PromptInputs::default(),
    )
    .await?;
    let mut stream = agent.run_turn(input("go"), CancellationToken::new());
    while let Some(chunk) = stream.next().await {
        if matches!(chunk, UiChunk::ToolInputAvailable { .. }) {
            break;
        }
    }
    tokio::time::sleep(Duration::from_millis(20)).await;
    drop(stream);
    for _ in 0..50 {
        if !tools.cancelled().is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(tools.cancelled(), ["act"]);
    Ok(())
}

/// Providers number text parts per response; the stream keeps ids unique across the turn.
#[tokio::test]
async fn text_part_ids_stay_unique_across_steps() -> TestResult {
    let tools = TestTools::new("internal", vec![dynamic_tool("act")]);
    let mut first = text("0", "a");
    first.push(call("c1", "act", json!({})));
    first.push(finish(ModelFinishReason::ToolCalls));
    let mut second = text("0", "b");
    second.push(finish(ModelFinishReason::Stop));
    let model = TestModel::new(vec![Step::Events(first), Step::Events(second)]);
    let agent = build(
        model,
        vec![source(&tools, true)],
        100,
        PromptInputs::default(),
    )
    .await?;
    let chunks = run(&agent, CancellationToken::new()).await;
    let ids: Vec<&str> = chunks
        .iter()
        .filter_map(|chunk| match chunk {
            UiChunk::TextStart { id, .. } => Some(id.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(ids.len(), 2);
    assert_eq!(ids[0], "0");
    assert_ne!(ids[1], "0");
    let delta_ids: Vec<&str> = chunks
        .iter()
        .filter_map(|chunk| match chunk {
            UiChunk::TextDelta { id, .. } | UiChunk::TextEnd { id, .. } => Some(id.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(delta_ids, [ids[0], ids[0], ids[1], ids[1]]);
    Ok(())
}

/// The model sees this turn's user message as `prompt_text`, the tools in merge order (later
/// sources replace a tool in place; optional sources cannot shadow a required source's tools),
/// and no nudges in chat mode.
#[tokio::test]
async fn build_merges_sources_and_wraps_the_user_message() -> TestResult {
    let internal = TestTools::new(
        "browseros",
        vec![dynamic_tool("tabs"), dynamic_tool("navigate")],
    );
    let custom = TestTools::new(
        "custom-x",
        vec![dynamic_tool("navigate"), dynamic_tool("search")],
    );
    let local = TestTools::new(
        "filesystem",
        vec![static_tool("filesystem_read"), static_tool("search")],
    );
    let model = TestModel::new(vec![Step::Events(vec![finish(ModelFinishReason::Stop)])]);
    let agent = build(
        model.clone(),
        vec![
            source(&internal, true),
            source(&custom, false),
            source(&local, false),
            SourceSpec {
                source: nudge_tools(vec!["Gmail".to_owned()]),
                required: false,
            },
        ],
        100,
        PromptInputs {
            chat_mode: true,
            ..PromptInputs::default()
        },
    )
    .await?;
    let names: Vec<&str> = agent.tool_names().iter().map(String::as_str).collect();
    assert_eq!(names, ["filesystem_read", "navigate", "search", "tabs"]);

    let chunks = run(&agent, CancellationToken::new()).await;
    assert_eq!(
        kinds(&chunks),
        ["start", "start-step", "finish-step", "finish"]
    );
    let request = model.requests().into_iter().next().ok_or("no request")?;
    let order: Vec<&str> = request
        .tools
        .iter()
        .map(|tool| tool.name.as_str())
        .collect();
    assert_eq!(order, ["tabs", "navigate", "search", "filesystem_read"]);
    // The local `search` (the static spec, with `required`) replaced the custom one in place.
    let search = request
        .tools
        .iter()
        .find(|tool| tool.name == "search")
        .ok_or("search")?;
    assert!(search.input_schema.get("required").is_some());
    assert!(request.system.is_some());
    let Some(ModelMessage::User { content }) = request.messages.last() else {
        return Err(format!("expected the user message last: {:?}", request.messages).into());
    };
    assert!(
        matches!(&content[..], [UserPart::Text { text, .. }] if text == "<USER_QUERY>\nhello\n</USER_QUERY>")
    );
    Ok(())
}

#[tokio::test]
async fn nudges_are_kept_outside_chat_and_scheduled_runs() -> TestResult {
    let model = TestModel::new(Vec::new());
    let nudges = SourceSpec {
        source: nudge_tools(vec!["Gmail".to_owned()]),
        required: false,
    };
    let agent = build(model, vec![nudges], 100, PromptInputs::default()).await?;
    let names: Vec<&str> = agent.tool_names().iter().map(String::as_str).collect();
    assert_eq!(names, ["suggest_app_connection", "suggest_schedule"]);

    let scheduled = build(
        TestModel::new(Vec::new()),
        vec![SourceSpec {
            source: nudge_tools(Vec::new()),
            required: false,
        }],
        100,
        PromptInputs {
            is_scheduled_task: true,
            ..PromptInputs::default()
        },
    )
    .await?;
    assert!(scheduled.tool_names().is_empty());
    Ok(())
}

/// A required source that fails aborts the build and closes the sources that did connect; an
/// optional one is skipped and closed.
#[tokio::test]
async fn required_source_failure_fails_the_build_and_closes_the_rest() -> TestResult {
    let internal = Arc::new(TestTools {
        label: "browseros".to_owned(),
        unavailable: true,
        ..TestTools::default()
    });
    let custom = TestTools::new("custom-x", vec![dynamic_tool("search")]);
    let result = build(
        TestModel::new(Vec::new()),
        vec![source(&internal, true), source(&custom, false)],
        100,
        PromptInputs::default(),
    )
    .await;
    match result {
        Err(AgentBuildError::RequiredSourceUnavailable(label)) => {
            assert_eq!(label, "browseros");
            assert_eq!(
                AgentBuildError::RequiredSourceUnavailable(label).to_string(),
                "Required MCP server \"browseros\" is unavailable."
            );
        }
        Err(other) => return Err(other.into()),
        Ok(_) => return Err("the build should have failed".into()),
    }
    assert!(custom.closed.load(Ordering::SeqCst));

    let optional = Arc::new(TestTools {
        label: "custom-down".to_owned(),
        unavailable: true,
        ..TestTools::default()
    });
    let agent = build(
        TestModel::new(Vec::new()),
        vec![source(&optional, false)],
        100,
        PromptInputs::default(),
    )
    .await?;
    assert!(agent.tool_names().is_empty());
    assert!(optional.closed.load(Ordering::SeqCst));

    let live = TestTools::new("browseros", vec![dynamic_tool("tabs")]);
    let agent = build(
        TestModel::new(Vec::new()),
        vec![source(&live, true)],
        100,
        PromptInputs::default(),
    )
    .await?;
    agent.dispose().await;
    assert!(live.closed.load(Ordering::SeqCst));
    Ok(())
}
