//! Wire tests for every provider family: the request `RigModel` sends is compared with the
//! request the TS server sent for the same config and transcript (`fixtures/llm/ts-requests.json`,
//! captured by `fixtures/llm/capture-ts.ts`), and canned provider streams are decoded into
//! `ModelEvent`s, including native-item replay into the next step.
//!
//! Each test starts a local fake provider; `HttpStack::with_test_origin` sends every request
//! there while keeping the provider's own path and query.

mod llm_support;

use browseros_agent::llm::{
    ChatErrorCode, LanguageModel, ModelError, ModelEvent, ModelFinishReason, ModelRequest,
    ProviderKind, ProviderOptions, ScriptedModel, create_language_model, generate_text,
    to_chat_error,
};
use llm_support::{
    Fake, Reply, SYSTEM, assistant_from, collect, config, events, get_time_tool, request_for,
    scenario, scenarios, tool_text_result, user,
};
use serde_json::{Value, json};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

/// The request fields that must be identical to TS for each family. Everything else (field order,
/// rig's own defaults such as tool `strict` flags, `stream_options.include_usage`, an implicit
/// `tool_choice: auto`) is free to differ. Known, accepted differences are listed in
/// `accepted_difference`.
fn compared_pointers(provider: ProviderKind) -> &'static [&'static str] {
    match provider {
        ProviderKind::Anthropic => &[
            "/model",
            "/max_tokens",
            "/thinking",
            "/output_config",
            "/stream",
            "/messages",
        ],
        ProviderKind::OpenAi | ProviderKind::Azure => {
            &["/model", "/reasoning", "/store", "/include", "/stream"]
        }
        ProviderKind::ChatGptPro => &[
            "/model",
            "/reasoning",
            "/store",
            "/include",
            "/stream",
            "/instructions",
        ],
        ProviderKind::Google => &[
            "/generationConfig/thinkingConfig",
            "/contents/0",
            "/contents/1",
        ],
        ProviderKind::Bedrock => &[
            "/additionalModelRequestFields/thinking",
            "/inferenceConfig/maxTokens",
            "/messages",
        ],
        _ => &[
            "/model",
            "/reasoning",
            "/stream",
            "/messages/0",
            "/messages/1",
        ],
    }
}

/// Differences from TS that are deliberate (each recorded as a DECIDED in the commit):
/// - OpenAI/Azure Responses: rig asks for `reasoning.encrypted_content` whenever the body
///   reasons, so in-turn reasoning replays statelessly; TS relies on stored item ids.
/// - Gemini: rig marks user parts `thought: false` and sends no `id` on our call/response parts
///   (they correlate by name and order); both are optional fields of the same API.
fn accepted_difference(
    provider: ProviderKind,
    pointer: &str,
    ours: Option<&Value>,
    theirs: Option<&Value>,
) -> bool {
    match (provider, pointer) {
        (ProviderKind::OpenAi | ProviderKind::Azure, "/include") => {
            theirs.is_none() && ours == Some(&serde_json::json!(["reasoning.encrypted_content"]))
        }
        (ProviderKind::Google, pointer) if pointer.starts_with("/contents") => {
            ours.map(normalize_gemini) == theirs.map(normalize_gemini)
        }
        _ => false,
    }
}

fn normalize_gemini(value: &Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.iter()
                .filter(|(key, value)| {
                    !(key.as_str() == "thought" && **value == Value::Bool(false))
                })
                .filter(|(key, _)| key.as_str() != "id")
                .map(|(key, value)| (key.clone(), normalize_gemini(value)))
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.iter().map(normalize_gemini).collect()),
        other => other.clone(),
    }
}

/// Headers that must match TS by value (when TS sent them) or by presence (secrets).
const HEADERS_BY_VALUE: &[&str] = &[
    "x-trace",
    "anthropic-version",
    "openai-intent",
    "x-initiator",
    "openai-beta",
    "originator",
    "chatgpt-account-id",
];
const HEADERS_BY_PRESENCE: &[&str] = &["authorization", "x-api-key", "api-key", "x-goog-api-key"];

#[tokio::test]
async fn every_ts_scenario_sends_the_ts_request() -> anyhow::Result<()> {
    let mut mismatches = Vec::new();
    let mut dump = serde_json::Map::new();
    for (name, entry) in scenarios()? {
        let fake = Fake::start(vec![Reply::error(
            400,
            r#"{"error":{"message":"captured"}}"#,
        )])
        .await?;
        let scenario = scenario(&entry)?;
        let model = browseros_agent::llm::create_language_model(&scenario.config, &fake.stack()?)?;
        let request = request_for(&name, &scenario);
        let _ = collect(model.stream(request, Default::default()).await).await;
        let Some(sent) = fake.requests().into_iter().next() else {
            mismatches.push(format!("{name}: no request reached the fake provider"));
            continue;
        };
        let ts = &entry["request"];
        dump.insert(name.clone(), sent.to_json());

        let ts_url = ts["url"].as_str().unwrap_or_default();
        let ts_path = ts_url
            .splitn(4, '/')
            .nth(3)
            .map(|rest| format!("/{rest}"))
            .unwrap_or_default();
        if sent.path_and_query != ts_path {
            mismatches.push(format!(
                "{name}: url {} != TS {ts_path}",
                sent.path_and_query
            ));
        }
        for header in HEADERS_BY_VALUE {
            let ours = sent.headers.get(*header);
            let theirs = ts["headers"].get(*header).and_then(Value::as_str);
            if ours.map(String::as_str) != theirs {
                mismatches.push(format!("{name}: header {header} {ours:?} != TS {theirs:?}"));
            }
        }
        for header in HEADERS_BY_PRESENCE {
            let ours = sent.headers.contains_key(*header);
            let theirs = ts["headers"].get(*header).is_some();
            if ours != theirs {
                mismatches.push(format!(
                    "{name}: header {header} present={ours} != TS {theirs}"
                ));
            }
        }
        let provider = scenario.config.provider;
        if provider == ProviderKind::ChatGptPro {
            // TS: `{role:"developer", content: SYSTEM}` first; rig spells content as parts.
            let first = &sent.body["input"][0];
            let text = first["content"][0]["text"]
                .as_str()
                .or(first["content"].as_str());
            if first["role"] != "developer" || text != Some(llm_support::SYSTEM) {
                mismatches.push(format!(
                    "{name}: /input/0 is not the developer system prompt: {first}"
                ));
            }
        }
        for pointer in compared_pointers(provider) {
            let ours = sent.body.pointer(pointer);
            let theirs = ts["body"].pointer(pointer);
            if ours != theirs && !accepted_difference(provider, pointer, ours, theirs) {
                mismatches.push(format!(
                    "{name}: {pointer}\n    ours: {}\n    TS:   {}",
                    ours.map(Value::to_string).unwrap_or_default(),
                    theirs.map(Value::to_string).unwrap_or_default()
                ));
            }
        }
    }
    if let Ok(path) = std::env::var("BROWSEROS_DUMP_WIRE") {
        std::fs::write(path, serde_json::to_string_pretty(&Value::Object(dump))?)?;
    }
    assert!(
        mismatches.is_empty(),
        "{} mismatches:\n{}",
        mismatches.len(),
        mismatches.join("\n")
    );
    Ok(())
}

fn request(messages: Vec<browseros_agent::messages::ModelMessage>) -> ModelRequest {
    ModelRequest {
        system: Some(SYSTEM.to_owned()),
        messages,
        tools: vec![get_time_tool()],
        options: ProviderOptions::default(),
    }
}

fn model_on(
    fake: &Fake,
    config: &browseros_agent::llm::LlmConfig,
) -> anyhow::Result<Arc<dyn LanguageModel>> {
    Ok(create_language_model(config, &fake.stack()?)?)
}

/// Runs one step and the next one built from its events, as the loop does.
async fn step_and_replay(
    fake: &Fake,
    model: &dyn LanguageModel,
    call_result: &str,
) -> anyhow::Result<(Vec<ModelEvent>, Value)> {
    let first = request(vec![user("What time is it?")]);
    let step = events(collect(model.stream(first, CancellationToken::new()).await).await)?;
    let call_id = step
        .iter()
        .find_map(|event| match event {
            ModelEvent::ToolCall {
                provider_call_id, ..
            } => Some(provider_call_id.clone()),
            _ => None,
        })
        .unwrap_or_default();
    let second = request(vec![
        user("What time is it?"),
        assistant_from(&step),
        tool_text_result(&call_id, call_result),
    ]);
    let _ = collect(model.stream(second, CancellationToken::new()).await).await;
    let replayed = fake
        .requests()
        .get(1)
        .map(|request| request.body.clone())
        .unwrap_or_default();
    Ok((step, replayed))
}

fn kinds(events: &[ModelEvent]) -> Vec<&'static str> {
    events
        .iter()
        .map(|event| match event {
            ModelEvent::TextStart { .. } => "text-start",
            ModelEvent::TextDelta { .. } => "text-delta",
            ModelEvent::TextEnd { .. } => "text-end",
            ModelEvent::ReasoningStart { .. } => "reasoning-start",
            ModelEvent::ReasoningDelta { .. } => "reasoning-delta",
            ModelEvent::ReasoningEnd { .. } => "reasoning-end",
            ModelEvent::ToolInputStart { .. } => "tool-input-start",
            ModelEvent::ToolInputDelta { .. } => "tool-input-delta",
            ModelEvent::ToolCall { .. } => "tool-call",
            ModelEvent::Finish { .. } => "finish",
        })
        .collect()
}

fn finish(events: &[ModelEvent]) -> Option<(ModelFinishReason, browseros_agent::llm::Usage)> {
    events.iter().find_map(|event| match event {
        ModelEvent::Finish { reason, usage } => Some((*reason, *usage)),
        _ => None,
    })
}

fn anthropic_block(index: usize, start: Value, deltas: &[Value]) -> Vec<(&'static str, Value)> {
    let mut frames = vec![(
        "content_block_start",
        json!({"type": "content_block_start", "index": index, "content_block": start}),
    )];
    for delta in deltas {
        frames.push((
            "content_block_delta",
            json!({"type": "content_block_delta", "index": index, "delta": delta}),
        ));
    }
    frames.push((
        "content_block_stop",
        json!({"type": "content_block_stop", "index": index}),
    ));
    frames
}

#[tokio::test]
async fn anthropic_stream_decodes_and_replays_signed_thinking() -> anyhow::Result<()> {
    let mut frames = vec![(
        "message_start",
        json!({"type": "message_start", "message": {
            "id": "msg_1", "type": "message", "role": "assistant", "model": "claude-sonnet-4-5",
            "content": [], "stop_reason": null, "stop_sequence": null,
            "usage": {"input_tokens": 12, "output_tokens": 1}}}),
    )];
    frames.extend(anthropic_block(
        0,
        json!({"type": "thinking", "thinking": "", "signature": ""}),
        &[
            json!({"type": "thinking_delta", "thinking": "Let me check."}),
            json!({"type": "signature_delta", "signature": "sig-abc"}),
        ],
    ));
    frames.extend(anthropic_block(
        1,
        json!({"type": "text", "text": ""}),
        &[json!({"type": "text_delta", "text": "Checking."})],
    ));
    frames.extend(anthropic_block(
        2,
        json!({"type": "tool_use", "id": "toolu_1", "name": "get_time", "input": {}}),
        &[json!({"type": "input_json_delta", "partial_json": "{\"tz\":\"UTC\"}"})],
    ));
    frames.push((
        "message_delta",
        json!({"type": "message_delta", "delta": {"stop_reason": "tool_use", "stop_sequence": null},
            "usage": {"output_tokens": 30}}),
    ));
    frames.push(("message_stop", json!({"type": "message_stop"})));
    let fake = Fake::start(vec![Reply::named_sse(&frames), Reply::error(400, "{}")]).await?;
    let model = model_on(&fake, &config(ProviderKind::Anthropic, "claude-sonnet-4-5"))?;
    assert!(model.supports_media_in_tool_results());

    let (step, replayed) = step_and_replay(&fake, model.as_ref(), "12:00").await?;
    assert_eq!(
        kinds(&step),
        [
            "reasoning-start",
            "reasoning-delta",
            "reasoning-end",
            "text-start",
            "text-delta",
            "text-end",
            "tool-input-start",
            "tool-input-delta",
            "tool-call",
            "finish",
        ]
    );
    let call = step
        .iter()
        .find(|event| matches!(event, ModelEvent::ToolCall { .. }));
    let Some(ModelEvent::ToolCall {
        id,
        provider_call_id,
        tool_name,
        input_json,
        ..
    }) = call
    else {
        anyhow::bail!("no tool call");
    };
    assert_eq!(
        (
            provider_call_id.as_str(),
            tool_name.as_str(),
            input_json.as_str()
        ),
        ("toolu_1", "get_time", r#"{"tz":"UTC"}"#)
    );
    let started = step.iter().find_map(|event| match event {
        ModelEvent::ToolInputStart { id, tool_name } => Some((id.clone(), tool_name.clone())),
        _ => None,
    });
    assert_eq!(
        started,
        Some((id.clone(), "get_time".to_owned())),
        "tool-input-start pairs with tool-call by id"
    );
    let Some((reason, usage)) = finish(&step) else {
        anyhow::bail!("no finish");
    };
    assert_eq!(reason, ModelFinishReason::ToolCalls);
    assert_eq!(usage.output_tokens, Some(30));

    // The signed thinking block goes back verbatim ahead of the call it produced.
    assert_eq!(
        replayed["messages"][1]["content"][0],
        json!({"type": "thinking", "thinking": "Let me check.", "signature": "sig-abc"}),
        "{replayed}"
    );
    assert_eq!(replayed["messages"][1]["content"][2]["id"], "toolu_1");
    assert_eq!(
        replayed["messages"][2]["content"][0]["tool_use_id"],
        "toolu_1"
    );
    Ok(())
}

#[tokio::test]
async fn unsigned_reasoning_is_dropped_on_replay() -> anyhow::Result<()> {
    // A reasoning part with no provider item (e.g. from another provider) never reaches the wire.
    let fake = Fake::start(vec![Reply::error(400, "{}")]).await?;
    let model = model_on(&fake, &config(ProviderKind::Anthropic, "claude-sonnet-4-5"))?;
    let messages = vec![
        user("hi"),
        browseros_agent::messages::ModelMessage::Assistant {
            content: vec![
                browseros_agent::messages::AssistantPart::Reasoning {
                    text: "secret thoughts".to_owned(),
                    provider_options: None,
                },
                browseros_agent::messages::AssistantPart::Text {
                    text: "Hello.".to_owned(),
                    provider_options: None,
                },
            ],
        },
        user("again"),
    ];
    let _ = collect(
        model
            .stream(request(messages), CancellationToken::new())
            .await,
    )
    .await;
    let body = fake
        .requests()
        .first()
        .map(|request| request.body.clone())
        .unwrap_or_default();
    assert_eq!(
        body["messages"][1]["content"],
        json!([{"type": "text", "text": "Hello."}]),
        "{body}"
    );
    Ok(())
}

#[tokio::test]
async fn responses_stream_decodes_and_replays_encrypted_reasoning() -> anyhow::Result<()> {
    let reasoning_done = json!({"type": "reasoning", "id": "rs_1",
        "summary": [{"type": "summary_text", "text": "Checking the clock."}],
        "encrypted_content": "ENC", "status": "completed"});
    let call_done = json!({"type": "function_call", "id": "fc_1", "status": "completed",
        "call_id": "call_1", "name": "get_time", "arguments": "{}"});
    let frames = [
        json!({"type": "response.created", "sequence_number": 0, "response": {
            "id": "resp_1", "object": "response", "created_at": 0, "status": "in_progress",
            "model": "gpt-6-astra", "output": [], "tools": []}}),
        json!({"type": "response.output_item.added", "output_index": 0, "sequence_number": 1,
            "item": {"type": "reasoning", "id": "rs_1", "summary": []}}),
        json!({"type": "response.reasoning_summary_part.added", "output_index": 0, "item_id": "rs_1",
            "summary_index": 0, "sequence_number": 2, "part": {"type": "summary_text", "text": ""}}),
        json!({"type": "response.reasoning_summary_text.delta", "output_index": 0, "item_id": "rs_1",
            "summary_index": 0, "sequence_number": 3, "delta": "Checking the clock."}),
        json!({"type": "response.output_item.done", "output_index": 0, "sequence_number": 4,
            "item": reasoning_done.clone()}),
        json!({"type": "response.output_item.added", "output_index": 1, "sequence_number": 5,
            "item": {"type": "function_call", "id": "fc_1", "status": "in_progress",
                "call_id": "call_1", "name": "get_time", "arguments": ""}}),
        json!({"type": "response.function_call_arguments.delta", "output_index": 1, "item_id": "fc_1",
            "sequence_number": 6, "delta": "{}"}),
        json!({"type": "response.output_item.done", "output_index": 1, "sequence_number": 7,
            "item": call_done.clone()}),
        json!({"type": "response.completed", "sequence_number": 8, "response": {
            "id": "resp_1", "object": "response", "created_at": 0, "status": "completed",
            "model": "gpt-6-astra", "output": [reasoning_done, call_done], "tools": [],
            "usage": {"input_tokens": 20, "input_tokens_details": {"cached_tokens": 4},
                "output_tokens": 15, "output_tokens_details": {"reasoning_tokens": 10}, "total_tokens": 35}}}),
    ];
    let fake = Fake::start(vec![Reply::sse(&frames), Reply::error(400, "{}")]).await?;
    let model = model_on(&fake, &config(ProviderKind::ChatGptPro, "gpt-6-astra"))?;
    let (step, replayed) = step_and_replay(&fake, model.as_ref(), "12:00").await?;
    // rig closes the reasoning only once the call it led to completes; the adapter still releases
    // `reasoning-end` first, so parts appended as they end keep the provider's order.
    assert_eq!(
        kinds(&step),
        [
            "reasoning-start",
            "reasoning-delta",
            "tool-input-start",
            "tool-input-delta",
            "reasoning-end",
            "tool-call",
            "finish",
        ]
    );
    let Some((reason, usage)) = finish(&step) else {
        anyhow::bail!("no finish");
    };
    assert_eq!(reason, ModelFinishReason::ToolCalls);
    assert_eq!(
        (
            usage.input_tokens,
            usage.output_tokens,
            usage.cached_input_tokens,
            usage.reasoning_tokens
        ),
        (Some(20), Some(15), Some(4), Some(10))
    );

    let reasoning_meta = step.iter().find_map(|event| match event {
        ModelEvent::ReasoningEnd {
            provider_metadata, ..
        } => Some(provider_metadata.clone()),
        _ => None,
    });
    assert!(
        reasoning_meta.flatten().is_some(),
        "reasoning carries its native item"
    );
    let input = replayed["input"].as_array().cloned().unwrap_or_default();
    let reasoning = input.iter().find(|item| item["type"] == "reasoning");
    assert_eq!(
        reasoning.map(|item| item["encrypted_content"].clone()),
        Some(json!("ENC")),
        "{replayed}"
    );
    // Codex keeps nothing server-side: every item id is stripped.
    assert!(
        input.iter().all(|item| item.get("id").is_none()),
        "{replayed}"
    );
    let call = input
        .iter()
        .position(|item| item["type"] == "function_call");
    let output = input
        .iter()
        .position(|item| item["type"] == "function_call_output");
    assert!(matches!((call, output), (Some(call), Some(output)) if call < output));
    assert_eq!(input[output.unwrap_or_default()]["call_id"], "call_1");
    assert_eq!(replayed["store"], false);
    assert_eq!(replayed["stream"], true);
    Ok(())
}

#[tokio::test]
async fn chat_stream_decodes_text_and_tool_calls() -> anyhow::Result<()> {
    let chunk = |delta: Value, finish: Value| {
        json!({"id": "c1", "object": "chat.completion.chunk", "created": 0, "model": "gemma4",
            "choices": [{"index": 0, "delta": delta, "finish_reason": finish}]})
    };
    let frames = [
        chunk(
            json!({"role": "assistant", "content": "Let me "}),
            Value::Null,
        ),
        chunk(json!({"content": "check."}), Value::Null),
        chunk(
            json!({"tool_calls": [{"index": 0, "id": "call_9", "type": "function",
                "function": {"name": "get_time", "arguments": ""}}]}),
            Value::Null,
        ),
        chunk(
            json!({"tool_calls": [{"index": 0, "function": {"arguments": "{}"}}]}),
            Value::Null,
        ),
        chunk(json!({}), json!("tool_calls")),
        json!({"id": "c1", "object": "chat.completion.chunk", "created": 0, "model": "gemma4",
            "choices": [], "usage": {"prompt_tokens": 9, "completion_tokens": 7, "total_tokens": 16}}),
    ];
    let mut body = String::new();
    for frame in &frames {
        body.push_str(&format!("data: {frame}\n\n"));
    }
    body.push_str("data: [DONE]\n\n");
    let fake = Fake::start(vec![Reply::raw_sse(&body)]).await?;
    let keyless = browseros_agent::llm::LlmConfig {
        api_key: None,
        ..config(ProviderKind::Ollama, "gemma4")
    };
    let model = model_on(&fake, &keyless)?;
    assert!(!model.supports_media_in_tool_results());
    let step = events(
        collect(
            model
                .stream(request(vec![user("hi")]), CancellationToken::new())
                .await,
        )
        .await,
    )?;
    assert_eq!(
        kinds(&step),
        [
            "text-start",
            "text-delta",
            "text-delta",
            "text-end",
            "tool-input-start",
            "tool-input-delta",
            "tool-call",
            "finish"
        ]
    );
    let text: String = step
        .iter()
        .filter_map(|event| match event {
            ModelEvent::TextDelta { delta, .. } => Some(delta.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(text, "Let me check.");
    assert!(step.iter().any(|event| matches!(event,
        ModelEvent::ToolCall { provider_call_id, input_json, .. } if provider_call_id == "call_9" && input_json == "{}")));
    assert_eq!(
        finish(&step).map(|(reason, usage)| (reason, usage.input_tokens)),
        Some((ModelFinishReason::ToolCalls, Some(9)))
    );
    // No key configured: no Authorization header at all (createOpenAICompatible).
    let sent = fake.requests();
    assert!(
        sent.first()
            .is_some_and(|request| !request.headers.contains_key("authorization"))
    );
    Ok(())
}

#[tokio::test]
async fn gemini_stream_replays_thought_signatures() -> anyhow::Result<()> {
    let chunk = |parts: Value, finish: Option<&str>| {
        let mut candidate = json!({"content": {"role": "model", "parts": parts}, "index": 0});
        if let Some(finish) = finish {
            candidate["finishReason"] = json!(finish);
        }
        json!({"candidates": [candidate], "modelVersion": "gemini-3-pro",
            "usageMetadata": {"promptTokenCount": 11, "candidatesTokenCount": 5, "thoughtsTokenCount": 3, "totalTokenCount": 19}})
    };
    let frames = [
        chunk(json!([{"text": "Considering.", "thought": true}]), None),
        chunk(
            json!([{"functionCall": {"name": "get_time", "args": {}}, "thoughtSignature": "c2lnLWNhbGw="}]),
            Some("STOP"),
        ),
    ];
    let fake = Fake::start(vec![Reply::sse(&frames), Reply::error(400, "{}")]).await?;
    let model = model_on(&fake, &config(ProviderKind::Google, "gemini-3-pro"))?;
    assert!(
        model.supports_media_in_tool_results(),
        "gemini-3 takes media in tool results"
    );
    let (step, replayed) = step_and_replay(&fake, model.as_ref(), "12:00").await?;
    assert!(
        kinds(&step).contains(&"reasoning-delta"),
        "{:?}",
        kinds(&step)
    );
    assert!(kinds(&step).contains(&"tool-call"));
    let parts = replayed["contents"][1]["parts"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let call = parts.iter().find(|part| part.get("functionCall").is_some());
    assert_eq!(
        call.map(|part| part["thoughtSignature"].clone()),
        Some(json!("c2lnLWNhbGw=")),
        "{replayed}"
    );
    // The key travels in the header, never in the URL.
    let sent = fake.requests();
    assert!(
        sent.iter()
            .all(|request| !request.path_and_query.contains("key="))
    );
    assert!(sent.iter().all(
        |request| request.headers.get("x-goog-api-key").map(String::as_str) == Some("test-key")
    ));
    Ok(())
}

#[tokio::test]
async fn http_failures_reach_the_classifier_as_the_ai_sdk_shapes_them() -> anyhow::Result<()> {
    let fake = Fake::start(vec![Reply::error_with_headers(
        429,
        &[("content-type", "application/json"), ("retry-after", "3")],
        r#"{"error":{"message":"Rate limit reached for gpt-5","type":"rate_limit"}}"#,
    )])
    .await?;
    let model = model_on(&fake, &config(ProviderKind::OpenAi, "gpt-5"))?;
    let items = collect(
        model
            .stream(request(vec![user("hi")]), CancellationToken::new())
            .await,
    )
    .await;
    let Some(Err(error)) = items.into_iter().next() else {
        anyhow::bail!("expected an error");
    };
    assert!(error.is_retryable());
    let ModelError::Api {
        status, message, ..
    } = &error
    else {
        anyhow::bail!("expected an API error, got {error:?}");
    };
    assert_eq!(
        (*status, message.as_str()),
        (429, "Rate limit reached for gpt-5")
    );
    let chat = to_chat_error(&error, Some(ProviderKind::OpenAi));
    assert_eq!(chat.code, ChatErrorCode::RateLimited);
    assert_eq!(chat.retry_after_seconds, Some(3));
    assert!(chat.details.unwrap_or_default().contains("rate_limit"));

    let fake = Fake::start(vec![Reply::error(
        402,
        r#"{"error":{"code":402,"message":"Insufficient credits","metadata":{"raw":{"balance":0}}}}"#,
    )])
    .await?;
    let model = model_on(
        &fake,
        &config(ProviderKind::OpenRouter, "anthropic/claude-sonnet-4.5"),
    )?;
    let items = collect(
        model
            .stream(request(vec![user("hi")]), CancellationToken::new())
            .await,
    )
    .await;
    let Some(Err(error)) = items.into_iter().next() else {
        anyhow::bail!("expected an error");
    };
    assert_eq!(
        error.to_string(),
        r#"[402] Insufficient credits ({"balance":0})"#
    );
    assert!(!error.is_retryable());
    assert_eq!(to_chat_error(&error, None).code, ChatErrorCode::Unknown);
    Ok(())
}

#[tokio::test]
async fn an_unreachable_provider_is_a_connection_failure() -> anyhow::Result<()> {
    // Port 9 (discard) on loopback: nothing listens.
    let model = create_language_model(
        &config(ProviderKind::Ollama, "gemma4"),
        &browseros_agent::llm::HttpStack::new(),
    )?;
    let items = collect(
        model
            .stream(request(vec![user("hi")]), CancellationToken::new())
            .await,
    )
    .await;
    let Some(Err(error)) = items.into_iter().next() else {
        anyhow::bail!("expected an error");
    };
    assert!(matches!(error, ModelError::Connection(_)), "{error:?}");
    assert!(error.is_retryable());
    assert_eq!(
        to_chat_error(&error, Some(ProviderKind::Ollama)).code,
        ChatErrorCode::ConnectionFailed
    );
    Ok(())
}

#[tokio::test]
async fn cancelling_ends_a_stalled_stream_promptly() -> anyhow::Result<()> {
    let first = json!({"id": "c1", "object": "chat.completion.chunk", "created": 0, "model": "m",
        "choices": [{"index": 0, "delta": {"content": "Hel"}, "finish_reason": null}]});
    let fake = Fake::start(vec![Reply::Stall {
        content_type: "text/event-stream".to_owned(),
        first: format!("data: {first}\n\n"),
    }])
    .await?;
    let model = model_on(&fake, &config(ProviderKind::OpenAiCompatible, "m"))?;
    let cancel = CancellationToken::new();
    let mut stream = model
        .stream(request(vec![user("hi")]), cancel.clone())
        .await?;
    use futures_util::StreamExt;
    let mut seen = Vec::new();
    while let Some(item) =
        tokio::time::timeout(std::time::Duration::from_secs(5), stream.next()).await?
    {
        let event = item?;
        let delta = matches!(event, ModelEvent::TextDelta { .. });
        seen.push(event);
        if delta {
            break;
        }
    }
    cancel.cancel();
    let rest: Vec<_> = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        stream.collect::<Vec<_>>(),
    )
    .await?;
    assert!(
        matches!(rest.as_slice(), [Err(ModelError::Cancelled)]),
        "{rest:?}"
    );
    Ok(())
}

#[tokio::test]
async fn copilot_flags_vision_and_shrinks_images() -> anyhow::Result<()> {
    use base64::Engine;
    let image = image::RgbImage::from_pixel(3000, 1000, image::Rgb([200, 20, 20]));
    let mut png = Vec::new();
    image::DynamicImage::ImageRgb8(image)
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)?;
    let encoded = base64::engine::general_purpose::STANDARD.encode(&png);
    let fake = Fake::start(vec![Reply::error(400, "{}")]).await?;
    let mut copilot = config(ProviderKind::GitHubCopilot, "gpt-5-mini");
    copilot.headers = Some([("X-Trace".to_owned(), "never".to_owned())].into());
    let model = model_on(&fake, &copilot)?;
    let messages = vec![browseros_agent::messages::ModelMessage::User {
        content: vec![
            browseros_agent::messages::UserPart::Text {
                text: "look".to_owned(),
                provider_options: None,
            },
            browseros_agent::messages::UserPart::Image {
                image: encoded,
                media_type: Some("image/png".to_owned()),
                provider_options: None,
            },
        ],
    }];
    let _ = collect(
        model
            .stream(request(messages), CancellationToken::new())
            .await,
    )
    .await;
    let sent = fake
        .requests()
        .into_iter()
        .next()
        .ok_or_else(|| anyhow::anyhow!("no request"))?;
    assert_eq!(
        sent.headers
            .get("copilot-vision-request")
            .map(String::as_str),
        Some("true")
    );
    assert_eq!(
        sent.headers.get("x-initiator").map(String::as_str),
        Some("user")
    );
    assert!(
        !sent.headers.contains_key("x-trace"),
        "OAuth providers never forward user headers"
    );
    assert_eq!(
        sent.body["messages"][0]["content"], SYSTEM,
        "system prompt is a plain string"
    );
    let url = sent.body["messages"][1]["content"][1]["image_url"]["url"]
        .as_str()
        .unwrap_or_default();
    let (prefix, data) = url.split_once(',').unwrap_or_default();
    assert_eq!(
        prefix, "data:image/jpeg;base64",
        "opaque images re-encode as JPEG"
    );
    let bytes = base64::engine::general_purpose::STANDARD.decode(data)?;
    let resized = image::load_from_memory(&bytes)?;
    assert_eq!((resized.width(), resized.height()), (2048, 683));
    Ok(())
}

#[tokio::test]
async fn construction_errors_carry_the_ts_texts() -> anyhow::Result<()> {
    let http = browseros_agent::llm::HttpStack::new();
    let error =
        |config: browseros_agent::llm::LlmConfig| match create_language_model(&config, &http) {
            Ok(_) => String::new(),
            Err(error) => error.to_string(),
        };
    let without_key = |provider| browseros_agent::llm::LlmConfig {
        api_key: None,
        ..config(provider, "m")
    };
    let without_base = |provider| browseros_agent::llm::LlmConfig {
        base_url: Some(String::new()),
        ..config(provider, "m")
    };
    assert_eq!(
        error(without_key(ProviderKind::Anthropic)),
        "Anthropic provider requires apiKey"
    );
    assert_eq!(
        error(without_key(ProviderKind::OpenAi)),
        "OpenAI provider requires apiKey"
    );
    assert_eq!(
        error(without_key(ProviderKind::Google)),
        "Google provider requires apiKey"
    );
    assert_eq!(
        error(without_key(ProviderKind::OpenRouter)),
        "OpenRouter provider requires apiKey"
    );
    assert_eq!(
        error(without_key(ProviderKind::Azure)),
        "Azure provider requires apiKey and either resourceName or baseUrl"
    );
    assert_eq!(
        error(browseros_agent::llm::LlmConfig {
            resource_name: None,
            ..config(ProviderKind::Azure, "m")
        }),
        "Azure provider requires apiKey and either resourceName or baseUrl"
    );
    assert_eq!(
        error(without_base(ProviderKind::LmStudio)),
        "LMStudio provider requires baseUrl"
    );
    assert_eq!(
        error(without_base(ProviderKind::Ollama)),
        "Ollama provider requires baseUrl"
    );
    assert_eq!(
        error(without_base(ProviderKind::OpenAiCompatible)),
        "OpenAI-compatible provider requires baseUrl"
    );
    assert_eq!(
        error(without_base(ProviderKind::Moonshot)),
        "Moonshot provider requires baseUrl"
    );
    assert_eq!(
        error(without_key(ProviderKind::Moonshot)),
        "Moonshot provider requires apiKey"
    );
    assert_eq!(
        error(without_key(ProviderKind::ChatGptPro)),
        "ChatGPT requires OAuth authentication"
    );
    assert_eq!(
        error(without_key(ProviderKind::GitHubCopilot)),
        "GitHub Copilot requires OAuth authentication"
    );
    assert_eq!(
        error(without_key(ProviderKind::QwenCode)),
        "Qwen Code requires OAuth authentication"
    );
    assert_eq!(
        error(browseros_agent::llm::LlmConfig {
            region: None,
            ..config(ProviderKind::Bedrock, "m")
        }),
        "Bedrock provider requires accessKeyId, secretAccessKey, and region"
    );
    // Keyless local servers are fine.
    assert_eq!(error(without_key(ProviderKind::Ollama)), "");
    assert_eq!(
        "nope"
            .parse::<ProviderKind>()
            .err()
            .map(|error| error.to_string()),
        Some("Unknown provider: nope".to_owned())
    );
    let mut bad_header = config(ProviderKind::OpenAi, "m");
    bad_header.headers = Some([("Bad Header".to_owned(), "v".to_owned())].into());
    assert_eq!(error(bad_header), "Invalid header name: Bad Header");
    Ok(())
}

#[tokio::test]
async fn generate_text_collects_text_and_retries_before_the_first_event() -> anyhow::Result<()> {
    let chunk = |delta: Value, finish: Value| {
        json!({"id": "c1", "object": "chat.completion.chunk", "created": 0, "model": "m",
            "choices": [{"index": 0, "delta": delta, "finish_reason": finish}]})
    };
    let body = format!(
        "data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
        chunk(json!({"content": "ok"}), Value::Null),
        chunk(json!({}), json!("stop"))
    );
    let fake = Fake::start(vec![
        Reply::error_with_headers(
            503,
            &[("retry-after-ms", "10")],
            r#"{"error":{"message":"overloaded"}}"#,
        ),
        Reply::raw_sse(&body),
    ])
    .await?;
    let model = model_on(&fake, &config(ProviderKind::OpenAiCompatible, "m"))?;
    let text = generate_text(
        model.as_ref(),
        Some("Be brief."),
        "Respond with exactly: 'ok'",
        CancellationToken::new(),
    )
    .await?;
    assert_eq!(text, "ok");
    let sent = fake.requests();
    assert_eq!(sent.len(), 2, "one retry after the 503");
    assert_eq!(
        sent[1].body["messages"][0],
        json!({"role": "system", "content": "Be brief."})
    );
    assert!(
        sent[1]
            .body
            .get("tools")
            .is_none_or(|tools| tools.as_array().is_some_and(Vec::is_empty))
    );

    let fake = Fake::start(vec![Reply::error(
        401,
        r#"{"error":{"message":"bad key"}}"#,
    )])
    .await?;
    let model = model_on(&fake, &config(ProviderKind::OpenAiCompatible, "m"))?;
    let error = generate_text(model.as_ref(), None, "hi", CancellationToken::new())
        .await
        .err();
    assert_eq!(
        error.map(|error| error.to_string()),
        Some("bad key".to_owned())
    );
    assert_eq!(fake.requests().len(), 1, "a 401 is not retried");
    Ok(())
}

#[tokio::test]
async fn the_mock_model_answers_every_turn() -> anyhow::Result<()> {
    let mock = ScriptedModel::mock();
    for _ in 0..3 {
        let text = generate_text(&mock, None, "hi", CancellationToken::new()).await?;
        assert_eq!(text, browseros_agent::llm::MOCK_LLM_RESPONSE_TEXT);
    }
    assert_eq!(mock.requests().len(), 3);

    // A scripted step without a terminal item stalls until cancelled.
    let scripted = ScriptedModel::new(vec![vec![Ok(ModelEvent::TextStart {
        id: "t".to_owned(),
        provider_metadata: None,
    })]]);
    let cancel = CancellationToken::new();
    let stream = scripted
        .stream(request(vec![user("hi")]), cancel.clone())
        .await?;
    cancel.cancel();
    let items = collect(Ok(stream)).await;
    assert!(matches!(
        items.as_slice(),
        [Ok(ModelEvent::TextStart { .. }), Err(ModelError::Cancelled)]
    ));
    Ok(())
}
