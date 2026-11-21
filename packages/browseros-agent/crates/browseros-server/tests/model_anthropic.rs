//! Hand-authored from the Claude Messages streaming guide's text, tool and thinking events.

#[path = "support/fake_provider.rs"]
mod fake_provider;

use anyhow::Context;
use browseros_server::model::{Image, ModelMessage};
use browseros_server::store::{Credentials, ProviderKind, Secret};
use browseros_server::turn::{FinishReason, TurnEvent};
use fake_provider::{FakeProvider, Reply, models, provider, request};
use serde_json::json;
use uuid::Uuid;

#[tokio::test]
async fn signed_and_redacted_thinking_replay_with_the_tool_result() -> anyhow::Result<()> {
    let thinking = json!({"type":"thinking","thinking":"Checking.","signature":"opaque-signature"});
    let redacted = json!({"type":"redacted_thinking","data":"opaque-redacted-data"});
    let tool = json!({"type":"tool_use","id":"toolu_1","name":"run","input":{"code":"1+1"}});
    let fake = FakeProvider::new(vec![
        Reply::events(&[
            json!({"type":"message_start","message":{"id":"msg_1","role":"assistant","content":[],"usage":{"input_tokens":2,"cache_read_input_tokens":13,"output_tokens":1}}}),
            json!({"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":"","signature":""}}),
            json!({"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"Checking."}}),
            json!({"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"opaque-signature"}}),
            json!({"type":"content_block_stop","index":0}),
            json!({"type":"content_block_start","index":1,"content_block":redacted}),
            json!({"type":"content_block_stop","index":1}),
            json!({"type":"content_block_start","index":2,"content_block":{"type":"tool_use","id":"toolu_1","name":"run"}}),
            json!({"type":"content_block_delta","index":2,"delta":{"type":"input_json_delta","partial_json":"{\"code\":"}}),
            json!({"type":"content_block_delta","index":2,"delta":{"type":"input_json_delta","partial_json":"\"1+1\"}"}}),
            json!({"type":"content_block_stop","index":2}),
            json!({"type":"message_delta","delta":{"stop_reason":"tool_use"},"usage":{"output_tokens":17}}),
            json!({"type":"message_stop"}),
        ]),
        Reply::events(&[
            json!({"type":"message_start","message":{"role":"assistant","content":[],"usage":{"input_tokens":19,"output_tokens":1}}}),
            json!({"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}),
            json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Two"}}),
            json!({"type":"content_block_stop","index":0}),
            json!({"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":2}}),
            json!({"type":"message_stop"}),
        ]),
    ]).await?;
    let (_dir, models) = models().await?;
    let mut config = provider(
        ProviderKind::Anthropic,
        format!("{}/v1", fake.base),
        Credentials::ApiKey(Secret::new("anthropic-fixture-key")),
    );
    config.model = "claude-sonnet-4-5".into();
    config.reasoning_effort = Some("minimal".into());
    let model = models.resolve(config, Uuid::new_v4()).await?;
    let mut events = Vec::new();
    let turn = model.step(&request(&[]), &mut events).await?;
    assert_eq!(turn.finish(), FinishReason::ToolCalls);
    assert_eq!(turn.usage().context("usage")?.input_tokens, 15);
    assert_eq!(turn.usage().context("usage")?.output_tokens, 17);
    assert_eq!(turn.tool_calls()[0].input, json!({"code":"1+1"}));
    assert!(
        matches!(&events[..], [TurnEvent::Reasoning(text), TurnEvent::ToolStarted { id, .. }] if text == "Checking." && id.0 == "toolu_1")
    );
    let result = ModelMessage::ToolResult {
        id: turn.tool_calls()[0].id.clone(),
        text: "2".into(),
    };
    let messages = [
        ModelMessage::Assistant(turn),
        result,
        ModelMessage::User {
            text: "Attached image from the tool result:".into(),
            images: vec![Image {
                media_type: "image/png".into(),
                bytes: vec![1, 2, 3],
            }],
        },
    ];
    let turn = model.step(&request(&messages), &mut events).await?;
    assert_eq!(turn.finish(), FinishReason::Stop);
    assert_eq!(events.last(), Some(&TurnEvent::Text("Two".into())));
    let calls = fake
        .calls
        .lock()
        .map_err(|_| anyhow::anyhow!("capture lock"))?;
    assert_eq!(calls[0].uri.path(), "/v1/messages");
    assert_eq!(calls[0].headers["x-api-key"], "anthropic-fixture-key");
    assert_eq!(calls[0].headers["anthropic-version"], "2023-06-01");
    assert_eq!(
        calls[0].body["thinking"],
        json!({"type":"enabled","budget_tokens":1024})
    );
    assert_eq!(
        calls[1].body["messages"],
        json!([
            {"role":"assistant","content":[thinking, redacted, tool]},
            {"role":"user","content":[
                {"type":"tool_result","tool_use_id":"toolu_1","content":"2"},
                {"type":"text","text":"Attached image from the tool result:"},
                {"type":"image","source":{"type":"base64","media_type":"image/png","data":"AQID"}}
            ]}
        ])
    );
    Ok(())
}
