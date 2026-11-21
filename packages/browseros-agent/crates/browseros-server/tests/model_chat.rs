//! Hand-authored streams from the OpenAI Chat Completions streaming grammar, with extensions
//! from the Gemini OpenAI compatibility and OpenRouter reasoning-details documentation.

#[path = "support/fake_provider.rs"]
mod fake_provider;

use anyhow::Context;
use browseros_server::model::ModelMessage;
use browseros_server::store::{Credentials, ProviderKind, Secret};
use browseros_server::turn::{FinishReason, TurnEvent};
use fake_provider::{FakeProvider, Reply, models, provider, request};
use serde_json::{Value, json};
use uuid::Uuid;

fn finished() -> Reply {
    Reply::events(&[
        json!({"choices":[{"index":0,"delta":{"content":"ok"},"finish_reason":null}]}),
        json!({"choices":[{"index":0,"delta":{},"finish_reason":"stop"}], "usage":{"prompt_tokens":10,"completion_tokens":2}}),
    ])
}

#[tokio::test]
async fn tool_deltas_and_provider_extensions_survive_follow_up() -> anyhow::Result<()> {
    let signature = json!({"google":{"thought_signature":"opaque-gemini-signature"}});
    let detail = json!({"type":"reasoning.encrypted","format":"anthropic-claude-v1","index":0,"data":"opaque-ciphertext"});
    for kind in [ProviderKind::Google, ProviderKind::Openrouter] {
        let mut delta = json!({"role":"assistant","tool_calls":[{"index":1,"id":"second","type":"function","function":{"name":"run","arguments":"{\"tab\":"}},{"index":0,"id":"first","type":"function","function":{"name":"run","arguments":"{}"}}]});
        if kind == ProviderKind::Google {
            delta["tool_calls"][0]["extra_content"] = signature.clone();
        } else {
            delta["reasoning"] = json!("Thinking");
            delta["reasoning_details"] = json!([detail]);
        }
        let fake = FakeProvider::new(vec![
            Reply::events(&[
                json!({"choices":[{"index":0,"delta":delta}]}),
                json!({"choices":[{"index":0,"delta":{"tool_calls":[{"index":1,"function":{"arguments":"2}"}}]},"finish_reason":"tool_calls"}]}),
                json!({"choices":[],"usage":{"prompt_tokens":12,"completion_tokens":3}}),
            ]),
            finished()
        ]).await?;
        let (_dir, models) = models().await?;
        let mut config = provider(
            kind,
            format!("{}/v1/", fake.base),
            Credentials::ApiKey(Secret::new("fixture-key")),
        );
        config
            .headers
            .push(("x-conversation".into(), "id={{conversationId}}".into()));
        let conversation = Uuid::new_v4();
        let model = models.resolve(config, conversation).await?;
        let mut events = Vec::new();
        let turn = model.step(&request(&[]), &mut events).await?;
        assert_eq!(turn.finish(), FinishReason::ToolCalls);
        assert_eq!(turn.usage().context("usage")?.input_tokens, 12);
        assert_eq!(turn.tool_calls()[0].id.0, "first");
        assert_eq!(turn.tool_calls()[1].input, json!({"tab":2}));
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, TurnEvent::ToolStarted { .. }))
                .count(),
            2
        );
        assert!(
            !events
                .iter()
                .any(|event| matches!(event, TurnEvent::ToolInput { .. }))
        );
        let mut follow_up: Vec<_> = turn
            .tool_calls()
            .iter()
            .map(|call| ModelMessage::ToolResult {
                id: call.id.clone(),
                text: "result".into(),
            })
            .collect();
        follow_up.insert(0, ModelMessage::Assistant(turn));
        model.step(&request(&follow_up), &mut events).await?;
        let calls = fake
            .calls
            .lock()
            .map_err(|_| anyhow::anyhow!("capture lock"))?;
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].uri.path(), "/v1/chat/completions");
        assert_eq!(
            calls[0].headers["x-conversation"],
            format!("id={conversation}")
        );
        let replay = &calls[1].body["messages"][1];
        let mut expected = json!({"id":"second","type":"function","function":{"name":"run","arguments":"{\"tab\":2}"}});
        if kind == ProviderKind::Google {
            expected["extra_content"] = signature.clone();
            assert_eq!(calls[0].body["reasoning_effort"], "medium");
        } else {
            assert_eq!(replay["reasoning_details"], json!([detail]));
            assert_eq!(replay["reasoning"], "Thinking");
            assert_eq!(calls[0].body["reasoning"], json!({}));
        }
        assert_eq!(replay["tool_calls"][1], expected);
        assert_eq!(calls[1].body["messages"][3]["tool_call_id"], "second");
        assert_eq!(calls[1].body["messages"][3]["content"], "result");
    }
    Ok(())
}

#[tokio::test]
async fn azure_v1_and_local_keyless_routes() -> anyhow::Result<()> {
    let fake = FakeProvider::new(vec![finished(), finished(), finished()]).await?;
    let (_dir, models) = models().await?;
    for (kind, path, credentials) in [
        (
            ProviderKind::Azure,
            "/openai/v1",
            Credentials::ApiKey(Secret::new("azure-key")),
        ),
        (ProviderKind::Ollama, "/v1", Credentials::None),
        (ProviderKind::Lmstudio, "/v1/", Credentials::None),
    ] {
        let mut config = provider(kind, format!("{}{path}", fake.base), credentials);
        config.reasoning_effort = Some("xhigh".into());
        let model = models.resolve(config, Uuid::new_v4()).await?;
        model.step(&request(&[]), &mut Vec::new()).await?;
    }
    let calls = fake
        .calls
        .lock()
        .map_err(|_| anyhow::anyhow!("capture lock"))?;
    assert_eq!(calls[0].uri.path(), "/openai/v1/chat/completions");
    assert!(calls[0].uri.query().is_none());
    assert_eq!(calls[0].headers["api-key"], "azure-key");
    assert_eq!(calls[0].body["reasoning_effort"], "xhigh");
    assert!(
        calls
            .iter()
            .all(|call| !call.headers.contains_key("authorization"))
    );
    assert!(
        calls[1..]
            .iter()
            .all(|call| call.uri.path() == "/v1/chat/completions")
    );
    Ok(())
}

#[tokio::test]
async fn concurrent_models_keep_their_own_keys() -> anyhow::Result<()> {
    let fake = FakeProvider::new(vec![finished(), finished()]).await?;
    let (_dir, models) = models().await?;
    let mut one = provider(
        ProviderKind::Openrouter,
        fake.base.clone(),
        Credentials::ApiKey(Secret::new("key-one")),
    );
    let mut two = provider(
        ProviderKind::Openrouter,
        fake.base.clone(),
        Credentials::ApiKey(Secret::new("key-two")),
    );
    one.model = "fixture-one".into();
    two.model = "fixture-two".into();
    let one = models.resolve(one, Uuid::new_v4()).await?;
    let two = models.resolve(two, Uuid::new_v4()).await?;
    let request = request(&[]);
    let mut events_one = Vec::new();
    let mut events_two = Vec::new();
    let (a, b) = tokio::join!(
        one.step(&request, &mut events_one),
        two.step(&request, &mut events_two)
    );
    a?;
    b?;
    let calls = fake
        .calls
        .lock()
        .map_err(|_| anyhow::anyhow!("capture lock"))?;
    for (model, key) in [
        ("fixture-one", "Bearer key-one"),
        ("fixture-two", "Bearer key-two"),
    ] {
        let call = calls
            .iter()
            .find(|call| call.body["model"] == Value::String(model.into()))
            .context("call")?;
        assert_eq!(call.headers["authorization"], key);
    }
    Ok(())
}
