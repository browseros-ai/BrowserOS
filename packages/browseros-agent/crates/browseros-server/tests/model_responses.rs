//! What the server guarantees on the OpenAI Responses wire, through Rig. Streams are
//! hand-authored from OpenAI's Responses streaming events and the reasoning continuation docs.

#[path = "support/fake_provider.rs"]
mod fake_provider;

use anyhow::Context;
use browseros_server::model::{AssistantTurn, Image, ModelMessage};
use browseros_server::store::{Credentials, ProviderKind, Secret};
use browseros_server::turn::{FinishReason, TurnEvent};
use fake_provider::{FakeProvider, Reply, models, parts, provider, request};
use serde_json::{Value, json};
use uuid::Uuid;

#[tokio::test]
async fn adjacent_users_send_one_input_with_text_and_pictures_in_order() -> anyhow::Result<()> {
    let fake = FakeProvider::new(vec![Reply::events(&[
        json!({"type":"response.output_text.delta","delta":"ok"}),
        json!({"type":"response.completed","response":{"status":"completed"}}),
    ])])
    .await?;
    let (_dir, models) = models().await?;
    let config = provider(
        ProviderKind::Openai,
        format!("{}/v1", fake.base),
        Credentials::ApiKey(Secret::new("fixture-key")),
    );
    let model = models.resolve(config, Uuid::new_v4()).await?;
    let user = |text: &str, images: Vec<Image>| ModelMessage::User {
        text: text.into(),
        images,
    };
    let picture = |byte| Image {
        media_type: "image/png".into(),
        bytes: vec![byte],
    };
    let messages = [
        user("earlier", vec![]),
        user("now", vec![]),
        ModelMessage::Assistant(AssistantTurn::text("an answer".into())),
        user("first photo", vec![picture(1)]),
        user("second photo", vec![picture(2)]),
        ModelMessage::Assistant(AssistantTurn::text("another answer".into())),
    ];
    model.step(&request(&messages), &mut Vec::new()).await?;

    let calls = fake
        .calls
        .lock()
        .map_err(|_| anyhow::anyhow!("capture lock"))?;
    assert_eq!(
        turns(&calls[0].body["input"]),
        [
            json!({"role":"user","content":[{"text":"earlier\n\nnow"}]}),
            json!({"role":"assistant","content":[{"text":"an answer"}]}),
            json!({"role":"user","content":[
                {"text":"first photo"},
                {"image":"data:image/png;base64,AQ=="},
                {"text":"\n\nsecond photo"},
                {"image":"data:image/png;base64,Ag=="}
            ]}),
            json!({"role":"assistant","content":[{"text":"another answer"}]}),
        ]
    );
    Ok(())
}

#[tokio::test]
async fn responses_replays_the_completed_encrypted_item_and_tool_call() -> anyhow::Result<()> {
    let reasoning = json!({"type":"reasoning","id":"rs_1","summary":[{"type":"summary_text","text":"Checking state"}],"encrypted_content":"opaque-encrypted-reasoning"});
    let call = json!({"type":"function_call","id":"fc_1","call_id":"call_1","name":"run","arguments":"{\"code\":\"1+1\"}","status":"completed"});
    let fake = FakeProvider::new(vec![
        Reply::events(&[
            json!({"type":"response.output_item.added","output_index":0,"item":{"type":"reasoning","id":"rs_1","summary":[]}}),
            json!({"type":"response.reasoning_summary_text.delta","output_index":0,"item_id":"rs_1","summary_index":0,"delta":"Checking state"}),
            json!({"type":"response.output_item.done","output_index":0,"item":reasoning}),
            json!({"type":"response.output_item.added","output_index":1,"item":{"type":"function_call","id":"fc_1","call_id":"call_1","name":"run","arguments":""}}),
            json!({"type":"response.function_call_arguments.delta","output_index":1,"item_id":"fc_1","delta":"{\"code\":\"1+"}),
            json!({"type":"response.function_call_arguments.delta","output_index":1,"item_id":"fc_1","delta":"1\"}"}),
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
    let call_id = turn.tool_calls()[0].id.clone();
    assert!(
        matches!(&events[..], [TurnEvent::Reasoning(text), TurnEvent::ToolStarted { id, .. }]
        if text == "Checking state" && *id == call_id)
    );
    let result = ModelMessage::ToolResult {
        id: turn.tool_calls()[0].id.clone(),
        text: "2".into(),
    };
    let pictures = ModelMessage::User {
        text: "Attached image from the tool result:".into(),
        images: vec![Image {
            media_type: "image/png".into(),
            bytes: vec![1],
        }],
    };
    let messages = [ModelMessage::Assistant(turn), result, pictures];
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
    let input = calls[1].body["input"].as_array().context("input")?;
    assert_eq!(input[0], reasoning, "encrypted reasoning replays verbatim");
    assert_eq!(input[1], call, "the function call replays verbatim");
    assert_eq!(input[2]["type"], "function_call_output");
    assert_eq!(input[2]["call_id"], "call_1", "the provider's call id");
    assert_eq!(input[2]["output"], "2");
    assert_eq!(
        turns(&Value::Array(input[3..].to_vec())),
        [json!({"role":"user","content":[
            {"text":"Attached image from the tool result:"},
            {"image":"data:image/png;base64,AQ=="}
        ]})]
    );
    Ok(())
}

/// Responses input as turns: Rig sends each user part as its own message item, so consecutive
/// user items are read back as one user turn.
fn turns(input: &Value) -> Vec<Value> {
    let mut turns: Vec<Value> = Vec::new();
    for item in input.as_array().into_iter().flatten() {
        let item = parts(item);
        match turns.last_mut() {
            Some(last) if last["role"] == "user" && item["role"] == "user" => {
                if let (Some(into), Some(from)) =
                    (last["content"].as_array_mut(), item["content"].as_array())
                {
                    into.extend(from.iter().cloned());
                }
            }
            _ => turns.push(item),
        }
    }
    turns
}
