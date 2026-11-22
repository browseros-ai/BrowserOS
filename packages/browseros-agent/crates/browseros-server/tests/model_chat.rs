//! What the server guarantees on the Chat Completions wire (eight row types), through Rig.
//! Streams are hand-authored from the OpenAI Chat Completions streaming grammar, with extensions
//! from the Gemini OpenAI compatibility, OpenRouter reasoning-details and Kimi chat documentation.

#[path = "support/fake_provider.rs"]
mod fake_provider;

use anyhow::Context;
use browseros_server::model::{AssistantTurn, Image, ModelMessage};
use browseros_server::store::{Credentials, ProviderKind, Secret};
use browseros_server::turn::{FinishReason, TurnEvent};
use fake_provider::{FakeProvider, Reply, models, parts, provider, request};
use serde_json::{Value, json};
use uuid::Uuid;

fn finished() -> Reply {
    Reply::events(&[
        json!({"choices":[{"index":0,"delta":{"content":"ok"},"finish_reason":null}]}),
        json!({"choices":[{"index":0,"delta":{},"finish_reason":"stop"}], "usage":{"prompt_tokens":10,"completion_tokens":2}}),
    ])
}

#[tokio::test]
async fn adjacent_users_send_one_message_with_text_and_pictures_in_order() -> anyhow::Result<()> {
    let fake = FakeProvider::new(vec![finished()]).await?;
    let (_dir, models) = models().await?;
    let config = provider(
        ProviderKind::OpenaiCompatible,
        format!("{}/v1", fake.base),
        Credentials::None,
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
    let messages = calls[0].body["messages"]
        .as_array()
        .context("wire messages")?;
    assert_eq!(
        messages[1]["content"], "earlier\n\nnow",
        "two questions are one plain-text message"
    );
    assert_eq!(
        messages[1..].iter().map(parts).collect::<Vec<_>>(),
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
async fn all_results_follow_the_assistant_before_pictures_labelled_by_call() -> anyhow::Result<()> {
    let fake = FakeProvider::new(vec![
        Reply::events(&[
            json!({"choices":[{"index":0,"delta":{"role":"assistant","tool_calls":[
            {"index":0,"id":"first","type":"function","function":{"name":"run","arguments":"{}"}},
            {"index":1,"id":"second","type":"function","function":{"name":"run","arguments":"{}"}}
        ]},"finish_reason":"tool_calls"}]}),
        ]),
        finished(),
        finished(),
    ])
    .await?;
    let (_dir, models) = models().await?;
    let config = provider(
        ProviderKind::OpenaiCompatible,
        format!("{}/v1", fake.base),
        Credentials::None,
    );
    let model = models.resolve(config, Uuid::new_v4()).await?;
    let turn = model.step(&request(&[]), &mut Vec::new()).await?;
    let capture = "Attached image from the tool result:";
    let picture_message = |byte| ModelMessage::User {
        text: capture.into(),
        images: vec![Image {
            media_type: "image/png".into(),
            bytes: vec![byte],
        }],
    };
    let mut follow_up = Vec::new();
    for (index, call) in turn.tool_calls().iter().enumerate() {
        follow_up.push(ModelMessage::ToolResult {
            id: call.id.clone(),
            text: "done".into(),
        });
        follow_up.push(picture_message(u8::try_from(index)? + 1));
    }
    follow_up.insert(0, ModelMessage::Assistant(turn));
    model
        .step(&request(&follow_up[..4]), &mut Vec::new())
        .await?;
    model.step(&request(&follow_up), &mut Vec::new()).await?;

    let calls = fake
        .calls
        .lock()
        .map_err(|_| anyhow::anyhow!("capture lock"))?;
    for (received, both_picture_sets) in [(&calls[1], false), (&calls[2], true)] {
        let messages = received.body["messages"]
            .as_array()
            .context("wire messages")?;
        let roles: Vec<_> = messages
            .iter()
            .filter_map(|message| message["role"].as_str())
            .collect();
        assert_eq!(roles, ["system", "assistant", "tool", "tool", "user"]);
        assert_eq!(messages[1]["tool_calls"][0]["id"], "first");
        assert_eq!(messages[1]["tool_calls"][1]["id"], "second");
        assert_eq!(messages[2]["tool_call_id"], "first");
        assert_eq!(messages[3]["tool_call_id"], "second");
        let mut expected = vec![
            json!({"text":format!("Tool call first: {capture}")}),
            json!({"image":"data:image/png;base64,AQ=="}),
        ];
        if both_picture_sets {
            expected.extend([
                json!({"text":format!("\n\nTool call second: {capture}")}),
                json!({"image":"data:image/png;base64,Ag=="}),
            ]);
        }
        assert_eq!(parts(&messages[4])["content"], Value::Array(expected));
    }
    Ok(())
}

#[tokio::test]
async fn pictures_after_one_tool_result_keep_their_message() -> anyhow::Result<()> {
    let fake = FakeProvider::new(vec![
        Reply::events(&[
            json!({"choices":[{"index":0,"delta":{"role":"assistant","tool_calls":[
            {"index":0,"id":"one","type":"function","function":{"name":"run","arguments":"{}"}}
        ]},"finish_reason":"tool_calls"}]}),
        ]),
        finished(),
    ])
    .await?;
    let (_dir, models) = models().await?;
    let config = provider(
        ProviderKind::OpenaiCompatible,
        format!("{}/v1", fake.base),
        Credentials::None,
    );
    let model = models.resolve(config, Uuid::new_v4()).await?;
    let turn = model.step(&request(&[]), &mut Vec::new()).await?;
    let id = turn.tool_calls()[0].id.clone();
    let messages = [
        ModelMessage::Assistant(turn),
        ModelMessage::ToolResult {
            id,
            text: "done".into(),
        },
        ModelMessage::User {
            text: "Attached image from the tool result:".into(),
            images: vec![Image {
                media_type: "image/png".into(),
                bytes: vec![1],
            }],
        },
    ];
    model.step(&request(&messages), &mut Vec::new()).await?;
    let calls = fake
        .calls
        .lock()
        .map_err(|_| anyhow::anyhow!("capture lock"))?;
    let messages = &calls[1].body["messages"];
    assert_eq!(messages[2]["tool_call_id"], "one");
    assert_eq!(
        parts(&messages[3]),
        json!({"role":"user","content":[
            {"text":"Attached image from the tool result:"},
            {"image":"data:image/png;base64,AQ=="}
        ]}),
        "a single call's picture is not labelled"
    );
    Ok(())
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
        let started: Vec<_> = events
            .iter()
            .filter_map(|event| match event {
                TurnEvent::ToolStarted { id, .. } => Some(id.clone()),
                _ => None,
            })
            .collect();
        let finished: Vec<_> = turn
            .tool_calls()
            .iter()
            .map(|call| call.id.clone())
            .collect();
        assert_eq!(started, finished, "each card's id is its finished call's");
        let inputs: Vec<_> = turn.tool_calls().iter().map(|call| &call.input).collect();
        assert!(inputs.contains(&&json!({"tab":2})) && inputs.contains(&&json!({})));
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
        let messages = calls[1].body["messages"].as_array().context("messages")?;
        let replay = &messages[1];
        let second = replay["tool_calls"]
            .as_array()
            .and_then(|calls| calls.iter().find(|call| call["id"] == "second"))
            .context("the provider's own call id replays")?;
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
        assert_eq!(second, &expected);
        let result = messages
            .iter()
            .find(|message| message["tool_call_id"] == "second")
            .context("the result answers the provider's call id")?;
        assert_eq!(result["content"], "result");
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
async fn a_refusal_is_visible_text_and_replays_as_the_answer() -> anyhow::Result<()> {
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
        json!({"role":"assistant","content":"I cannot do that."})
    );
    Ok(())
}
