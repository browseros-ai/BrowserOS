//! Hand-authored streams from the OpenAI Chat Completions streaming grammar, with extensions
//! from the Gemini OpenAI compatibility, OpenRouter reasoning-details and Kimi chat documentation.

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
    for kind in [
        ProviderKind::Google,
        ProviderKind::Openrouter,
        ProviderKind::Moonshot,
    ] {
        let mut delta = json!({"role":"assistant","tool_calls":[{"index":1,"id":"second","type":"function","function":{"name":"run","arguments":"{\"tab\":"}},{"index":0,"id":"first","type":"function","function":{"name":"run","arguments":"{}"}}]});
        if kind == ProviderKind::Google {
            delta["tool_calls"][0]["extra_content"] = signature.clone();
        } else if kind == ProviderKind::Openrouter {
            delta["reasoning"] = json!("Thinking");
            delta["reasoning_details"] = json!([detail]);
        } else {
            delta["reasoning_content"] = json!("Kimi is thinking");
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
        } else if kind == ProviderKind::Openrouter {
            assert_eq!(replay["reasoning_details"], json!([detail]));
            assert_eq!(replay["reasoning"], "Thinking");
            assert_eq!(calls[0].body["reasoning"], json!({}));
        } else {
            assert_eq!(replay["reasoning_content"], "Kimi is thinking");
            assert_eq!(events[0], TurnEvent::Reasoning("Kimi is thinking".into()));
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

#[tokio::test]
async fn classifies_provider_errors_and_retries_only_before_output() -> anyhow::Result<()> {
    use browseros_server::turn::ChatErrorCode;
    use http::{HeaderMap, HeaderValue, StatusCode};

    let (_dir, models) = models().await?;
    for (status, code, retry, attempts) in [
        (401, ChatErrorCode::AuthFailed, None, 1),
        (403, ChatErrorCode::AuthFailed, None, 1),
        (429, ChatErrorCode::RateLimited, Some("9"), 1),
        (429, ChatErrorCode::RateLimited, Some("0"), 3),
        (413, ChatErrorCode::ContextLength, None, 1),
        (500, ChatErrorCode::ProviderUnavailable, None, 3),
    ] {
        let mut headers = HeaderMap::new();
        if let Some(retry) = retry {
            headers.insert("retry-after", HeaderValue::from_str(retry)?);
        }
        let status = StatusCode::from_u16(status)?;
        let fake = FakeProvider::new(
            (0..attempts)
                .map(|_| {
                    Reply(
                        status,
                        headers.clone(),
                        json!({"error":{"message":"Upstream echoed fixture-secret"}}).to_string(),
                    )
                })
                .collect(),
        )
        .await?;
        let model = models
            .resolve(
                provider(
                    ProviderKind::Openrouter,
                    fake.base.clone(),
                    Credentials::ApiKey(Secret::new("fixture-secret")),
                ),
                Uuid::new_v4(),
            )
            .await?;
        let Err(error) = model.step(&request(&[]), &mut Vec::new()).await else {
            anyhow::bail!("expected provider error for {status}");
        };
        assert_eq!(error.code, code);
        assert_eq!(error.status_code, Some(status.as_u16()));
        assert_eq!(
            error.retry_after_seconds,
            retry.and_then(|value| value.parse().ok())
        );
        assert_eq!(error.message, "Upstream echoed [redacted]");
        assert!(
            !error
                .details
                .context("error details")?
                .contains("fixture-secret")
        );
        assert_eq!(
            fake.calls
                .lock()
                .map_err(|_| anyhow::anyhow!("capture lock"))?
                .len(),
            attempts
        );
    }

    let fake = FakeProvider::new(vec![Reply::events(&[
        json!({"choices":[{"index":0,"delta":{"content":"visible"}}]}),
        json!({"error":{"code":502,"message":"upstream wrapper","metadata":{"raw":"{\"error\":{\"message\":\"actual upstream failure\"}}"}}}),
    ])]).await?;
    let model = models
        .resolve(
            provider(
                ProviderKind::Openrouter,
                fake.base.clone(),
                Credentials::ApiKey(Secret::new("fixture-secret")),
            ),
            Uuid::new_v4(),
        )
        .await?;
    let mut events = Vec::new();
    let Err(error) = model.step(&request(&[]), &mut events).await else {
        anyhow::bail!("expected an embedded stream error");
    };
    assert_eq!(error.code, ChatErrorCode::ProviderUnavailable);
    assert_eq!(error.message, "actual upstream failure");
    assert_eq!(events, [TurnEvent::Text("visible".into())]);
    assert_eq!(
        fake.calls
            .lock()
            .map_err(|_| anyhow::anyhow!("capture lock"))?
            .len(),
        1
    );
    Ok(())
}

// Codex returns a named error. llama.cpp's router returns a numeric code.
const MISSING_MODEL_REPLIES: [&str; 2] = [
    r#"{"error":{"type":"invalid_request_error","code":"model_not_found","message":"The 'fixture-model' model is not supported."}}"#,
    r#"{"error":{"type":"invalid_request_error","code":400,"message":"model 'fixture-model' not found"}}"#,
];

#[tokio::test]
async fn invalid_model_opens_provider_settings() -> anyhow::Result<()> {
    use browseros_server::turn::ChatErrorCode;
    use http::{HeaderMap, StatusCode};

    let replies = MISSING_MODEL_REPLIES
        .map(|body| Reply(StatusCode::BAD_REQUEST, HeaderMap::new(), body.into()));
    let fake = FakeProvider::new(Vec::from(replies)).await?;
    let (_dir, models) = models().await?;
    for _ in MISSING_MODEL_REPLIES {
        let key = Credentials::ApiKey(Secret::new("fixture-key"));
        let config = provider(ProviderKind::Openai, fake.base.clone(), key);
        let Err(error) = models.test(config).await else {
            anyhow::bail!("expected a model configuration error");
        };
        assert_eq!(error.code, ChatErrorCode::ProviderConfig);
        assert!(error.message.contains("fixture-model"));
    }
    Ok(())
}

#[tokio::test]
async fn dropped_connection_is_classified_after_two_safe_retries() -> anyhow::Result<()> {
    use tokio::net::TcpListener;
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let base = format!("http://{}", listener.local_addr()?);
    let escaped = tokio::spawn(async move {
        for _ in 0..3 {
            let (socket, _) = listener.accept().await?;
            drop(socket);
        }
        std::io::Result::Ok(())
    });
    let (_dir, models) = models().await?;
    let model = models
        .resolve(
            provider(ProviderKind::OpenaiCompatible, base, Credentials::None),
            Uuid::new_v4(),
        )
        .await?;
    let Err(error) = model.step(&request(&[]), &mut Vec::new()).await else {
        anyhow::bail!("expected dropped connection");
    };
    assert_eq!(
        error.code,
        browseros_server::turn::ChatErrorCode::ConnectionFailed
    );
    tokio::time::timeout(std::time::Duration::from_secs(1), escaped).await???;
    Ok(())
}

#[tokio::test]
async fn a_content_filter_finishes_without_running_tools() -> anyhow::Result<()> {
    let fake = FakeProvider::new(vec![Reply::events(&[
        json!({"choices":[{"index":0,"delta":{},"finish_reason":"content_filter"}]}),
    ])])
    .await?;
    let (_dir, models) = models().await?;
    let model = models
        .resolve(
            provider(
                ProviderKind::OpenaiCompatible,
                fake.base.clone(),
                Credentials::None,
            ),
            Uuid::new_v4(),
        )
        .await?;
    let turn = model.step(&request(&[]), &mut Vec::new()).await?;
    assert_eq!(turn.finish(), FinishReason::ContentFilter);
    assert!(turn.tool_calls().is_empty());
    Ok(())
}

#[tokio::test]
async fn refusal_is_visible_and_retains_its_native_field() -> anyhow::Result<()> {
    let fake = FakeProvider::new(vec![
        Reply::events(&[
            json!({"choices":[{"index":0,"delta":{"refusal":"I cannot "}}]}),
            json!({"choices":[{"index":0,"delta":{"refusal":"do that."},"finish_reason":"stop"}]}),
        ]),
        finished(),
    ])
    .await?;
    let (_dir, models) = models().await?;
    let model = models
        .resolve(
            provider(
                ProviderKind::OpenaiCompatible,
                fake.base.clone(),
                Credentials::None,
            ),
            Uuid::new_v4(),
        )
        .await?;
    let mut events = Vec::new();
    let turn = model.step(&request(&[]), &mut events).await?;
    assert_eq!(turn.finish(), FinishReason::Stop);
    assert_eq!(
        events,
        [
            TurnEvent::Text("I cannot ".into()),
            TurnEvent::Text("do that.".into())
        ]
    );
    model
        .step(&request(&[ModelMessage::Assistant(turn)]), &mut Vec::new())
        .await?;
    let calls = fake
        .calls
        .lock()
        .map_err(|_| anyhow::anyhow!("capture lock"))?;
    assert_eq!(
        calls[1].body["messages"][1],
        json!({"role":"assistant","content":null,"refusal":"I cannot do that."})
    );
    Ok(())
}
