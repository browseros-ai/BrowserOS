//! Hand-authored from OpenAI's Responses streaming events and the reasoning continuation docs.

#[path = "support/fake_provider.rs"]
mod fake_provider;

use anyhow::Context;
use browseros_server::model::ModelMessage;
use browseros_server::store::{Credentials, ProviderKind, Secret};
use browseros_server::turn::{FinishReason, TurnEvent};
use fake_provider::{FakeProvider, Reply, models, provider, request};
use serde_json::json;
use uuid::Uuid;

#[tokio::test]
async fn responses_replays_the_completed_encrypted_item_and_tool_call() -> anyhow::Result<()> {
    let reasoning = json!({"type":"reasoning","id":"rs_1","summary":[{"type":"summary_text","text":"Checking state"}],"encrypted_content":"opaque-encrypted-reasoning"});
    let call = json!({"type":"function_call","id":"fc_1","call_id":"call_1","name":"run","arguments":"{\"code\":\"1+1\"}","status":"completed"});
    let fake = FakeProvider::new(vec![
        Reply::events(&[
            json!({"type":"response.reasoning_summary_text.delta","delta":"Checking state"}),
            json!({"type":"response.output_item.done","output_index":0,"item":reasoning}),
            json!({"type":"response.output_item.added","output_index":1,"item":{"type":"function_call","id":"fc_1","call_id":"call_1","name":"run","arguments":""}}),
            json!({"type":"response.function_call_arguments.delta","output_index":1,"delta":"{\"code\":\"1+"}),
            json!({"type":"response.function_call_arguments.delta","output_index":1,"delta":"1\"}"}),
            json!({"type":"response.output_item.done","output_index":1,"item":call}),
            json!({"type":"response.completed","response":{"status":"completed","output":[reasoning, call],"usage":{"input_tokens":16,"output_tokens":7}}}),
        ]),
        Reply::events(&[
            json!({"type":"response.output_text.delta","delta":"Two"}),
            json!({"type":"response.completed","response":{"status":"completed","output":[]}}),
        ]),
    ]).await?;
    let (_dir, models) = models().await?;
    let mut config = provider(
        ProviderKind::Openai,
        format!("{}/v1", fake.base),
        Credentials::ApiKey(Secret::new("openai-fixture-key")),
    );
    config.reasoning_effort = Some("xhigh".into());
    config.reasoning_summary = Some("detailed".into());
    let model = models.resolve(config, Uuid::new_v4()).await?;
    let mut events = Vec::new();
    let turn = model.step(&request(&[]), &mut events).await?;
    assert_eq!(turn.finish(), FinishReason::ToolCalls);
    assert_eq!(turn.usage().context("usage")?.output_tokens, 7);
    assert_eq!(turn.tool_calls()[0].input, json!({"code":"1+1"}));
    assert!(
        matches!(&events[..], [TurnEvent::Reasoning(text), TurnEvent::ToolStarted { id, .. }]
        if text == "Checking state" && id.0 == "call_1")
    );
    let result = ModelMessage::ToolResult {
        id: turn.tool_calls()[0].id.clone(),
        text: "2".into(),
    };
    let messages = [ModelMessage::Assistant(turn), result];
    let mut next = request(&messages);
    next.effort = None;
    let turn = model.step(&next, &mut events).await?;
    assert_eq!(turn.finish(), FinishReason::Stop);
    assert_eq!(events.last(), Some(&TurnEvent::Text("Two".into())));
    let calls = fake
        .calls
        .lock()
        .map_err(|_| anyhow::anyhow!("capture lock"))?;
    assert_eq!(calls[0].uri.path(), "/v1/responses");
    assert_eq!(
        calls[0].headers["authorization"],
        "Bearer openai-fixture-key"
    );
    assert_eq!(
        calls[0].body["reasoning"],
        json!({"effort":"xhigh","summary":"detailed"})
    );
    assert_eq!(
        calls[0].body["include"],
        json!(["reasoning.encrypted_content"])
    );
    assert!(calls[1].body.get("reasoning").is_none());
    assert_eq!(
        calls[1].body["input"],
        json!([reasoning, call, {"type":"function_call_output","call_id":"call_1","output":"2"}])
    );
    Ok(())
}
