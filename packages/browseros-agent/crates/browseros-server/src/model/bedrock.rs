//! Bedrock ConverseStream through `rig-bedrock`, on an AWS SDK client built from the row alone:
//! its key, secret, optional session token and region. There is no environment, profile or
//! instance-metadata lookup, and the SDK never retries (`Model::step` decides). Rig encodes the
//! conversation and decodes the event stream; the server keeps the credential scope, the header
//! rules and the error cards.

use aws::config::interceptors::BeforeTransmitInterceptorContextMut;
use aws::config::retry::RetryConfig;
use aws::config::{ConfigBag, Intercept, RuntimeComponents};
use aws::error::BoxError;
use aws_sdk_bedrockruntime as aws;
use http::header::{HeaderMap, HeaderName};
use rig_bedrock::client::BedrockRuntime;
use rig_core::DynModel;
use rig_core::error::{ErrorReport, ProviderError};
use rig_core::operation::Completion;
use serde_json::json;

use super::errors;
use crate::store::{Credentials, ProviderConfig};
use crate::turn::ChatError;

/// The row's model on a client scoped to the row. Also returns every credential and custom
/// header value, for scrubbing error text.
pub(super) fn model(
    provider: &ProviderConfig,
) -> Result<(DynModel<Completion>, HeaderMap), ChatError> {
    model_at(provider, None)
}

/// `endpoint` replaces the regional AWS endpoint, for a loopback fake in tests.
pub(super) fn model_at(
    provider: &ProviderConfig,
    endpoint: Option<&str>,
) -> Result<(DynModel<Completion>, HeaderMap), ChatError> {
    let (config, mut secrets) = config(&provider.credentials)?;
    let headers = super::headers(&provider.headers)?;
    for (name, value) in &headers {
        // The SDK signs these headers itself. A custom value must never change the signed
        // destination or replace the session credentials assigned to this turn.
        if name.as_str() == "authorization" || name.as_str().starts_with("x-amz-") {
            return Err(errors::config(
                "A custom header overrides AWS request signing.",
            ));
        }
        secrets.append(name.clone(), value.clone());
    }
    let mut config = config.to_builder();
    if let Some(endpoint) = endpoint {
        config = config.endpoint_url(endpoint);
    }
    if !headers.is_empty() {
        config = config.interceptor(Headers(headers));
    }
    let config = config.build();
    let model = BedrockRuntime::from(aws::Client::from_conf(config))
        .completion(&provider.model)
        .erase();
    Ok((model, secrets))
}

/// The row's custom headers on every request of its client. Rig builds the SDK call itself, so
/// they ride on the client's configuration, added before signing like the SDK's own.
#[derive(Debug)]
struct Headers(HeaderMap);

impl Intercept for Headers {
    fn name(&self) -> &'static str {
        "browseros::ProviderHeaders"
    }

    fn modify_before_signing(
        &self,
        context: &mut BeforeTransmitInterceptorContextMut<'_>,
        _runtime_components: &RuntimeComponents,
        _cfg: &mut ConfigBag,
    ) -> Result<(), BoxError> {
        let headers = context.request_mut().headers_mut();
        for (name, value) in &self.0 {
            if let Ok(value) = value.to_str() {
                headers.insert(name.as_str().to_owned(), value.to_owned());
            }
        }
        Ok(())
    }
}

fn config(credentials: &Credentials) -> Result<(aws::Config, HeaderMap), ChatError> {
    let Credentials::Aws {
        access_key_id,
        secret_access_key,
        session_token,
        region,
    } = credentials
    else {
        return Err(missing_credentials());
    };
    if [access_key_id.expose(), secret_access_key.expose(), region]
        .iter()
        .any(|value| value.trim().is_empty())
    {
        return Err(missing_credentials());
    }
    // Region is interpolated into the SDK endpoint. Constrain it to the documented ID shape.
    if !region
        .bytes()
        .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    {
        return Err(errors::config("Enter a valid AWS region."));
    }
    let token = session_token
        .as_ref()
        .map(|secret| secret.expose())
        .filter(|value| !value.trim().is_empty());
    let mut secrets = HeaderMap::new();
    for (name, value) in [
        ("aws-access-key-id", Some(access_key_id.expose())),
        ("aws-secret-access-key", Some(secret_access_key.expose())),
        ("aws-session-token", token),
    ] {
        if let Some(value) = value {
            super::set_header(&mut secrets, HeaderName::from_static(name), value, true)?;
        }
    }
    let credentials = aws::config::Credentials::new(
        access_key_id.expose(),
        secret_access_key.expose(),
        token.map(str::to_owned),
        None,
        "browseros-provider",
    );
    // Build directly from the row. There is no profile, environment or IMDS lookup, and the
    // model dispatcher alone decides if a failed operation is still safe to retry.
    let config = aws::Config::builder()
        .behavior_version_latest()
        .region(aws::config::Region::new(region.clone()))
        .credentials_provider(credentials)
        .retry_config(RetryConfig::disabled())
        .build();
    Ok((config, secrets))
}

fn missing_credentials() -> ChatError {
    errors::config("AWS Bedrock requires an access key ID, secret access key, and region.")
}

/// A Bedrock failure as an error card. Rig reports a service exception by its type (and the HTTP
/// status when there was one, which a stream exception lacks), so the type picks the status the
/// card is chosen by.
pub(super) fn error(error: &ProviderError, secrets: &HeaderMap) -> ChatError {
    let message = match error {
        ProviderError::ProviderResponse(_) => None,
        ProviderError::Provider(message) => Some(message.as_str()),
        // A transport failure or an unreadable reply, as on the HTTP wires. Rig cannot tell a
        // refused connection from a timeout here: the SDK drops the cause.
        _ => return errors::provider(error, None, secrets),
    };
    let report = ErrorReport::from(error);
    let code = report.code.as_deref();
    let status = report.http_status.or_else(|| match code? {
        "AccessDeniedException" | "UnrecognizedClientException" | "ExpiredTokenException" => {
            Some(403)
        }
        "ThrottlingException" | "ServiceQuotaExceededException" => Some(429),
        "ServiceUnavailableException" | "InternalServerException" | "ModelStreamErrorException" => {
            Some(503)
        }
        "ResourceNotFoundException" => Some(404),
        _ => None,
    });
    let message = message.or_else(|| report.provider_response_body());
    let body = json!({"error": {"code": code, "message": message}}).to_string();
    // rig-bedrock keeps no response headers, so a delay comes only from the body's hints.
    errors::upstream(status, errors::retry_after(&report), &body, secrets)
}

#[cfg(test)]
mod tests {
    use anyhow::Context;
    use uuid::Uuid;

    use super::*;
    use crate::model::fake_provider::{FakeProvider, Reply};
    use crate::model::{Models, event_stream};
    use crate::store::{ProviderKind, Secret, Store};
    use crate::turn::ChatErrorCode;

    fn credentials(key: &str, token: Option<&str>, region: &str) -> Credentials {
        Credentials::Aws {
            access_key_id: Secret::new(key),
            secret_access_key: Secret::new(format!("secret-{key}")),
            session_token: token.map(Secret::new),
            region: region.into(),
        }
    }

    #[tokio::test]
    async fn sdk_credentials_and_region_are_scoped_to_each_provider() -> anyhow::Result<()> {
        let (east, _) = config(&credentials(
            "fixture-east",
            Some("east-token"),
            "us-east-1",
        ))?;
        let (west, _) = config(&credentials("fixture-west", None, "us-west-2"))?;
        assert_eq!(east.region().context("east region")?.as_ref(), "us-east-1");
        assert_eq!(west.region().context("west region")?.as_ref(), "us-west-2");
        assert_eq!(
            east.retry_config().context("retry config")?.max_attempts(),
            1
        );

        let refusal = || {
            Reply(
                http::StatusCode::BAD_REQUEST,
                HeaderMap::new(),
                String::new(),
            )
        };
        let fake = FakeProvider::new(vec![refusal(), refusal()]).await?;
        let east = aws::Client::from_conf(east.to_builder().endpoint_url(&fake.base).build());
        let west = aws::Client::from_conf(west.to_builder().endpoint_url(&fake.base).build());
        let (east_result, west_result) = tokio::join!(
            east.converse_stream().model_id("test-east").send(),
            west.converse_stream().model_id("test-west").send(),
        );
        assert!(
            east_result.is_err() && west_result.is_err(),
            "the local SDK endpoint returns 400"
        );
        {
            let calls = fake
                .calls
                .lock()
                .map_err(|_| anyhow::anyhow!("capture lock"))?;
            let mut seen = std::collections::BTreeSet::new();
            assert_eq!(calls.len(), 2);
            for call in calls.iter() {
                let headers = &call.headers;
                let authorization = headers["authorization"].to_str()?;
                if authorization.contains("Credential=fixture-east/") {
                    assert!(authorization.contains("/us-east-1/bedrock/aws4_request"));
                    assert_eq!(headers["x-amz-security-token"], "east-token");
                    seen.insert("east");
                } else {
                    assert!(authorization.contains("Credential=fixture-west/"));
                    assert!(authorization.contains("/us-west-2/bedrock/aws4_request"));
                    assert!(!headers.contains_key("x-amz-security-token"));
                    seen.insert("west");
                }
            }
            assert_eq!(seen, std::collections::BTreeSet::from(["east", "west"]));
        }

        let dir = tempfile::tempdir()?;
        let models = Models::new(Store::open(dir.path()).await?, None);
        let provider = ProviderConfig {
            id: "fixture".into(),
            kind: ProviderKind::Bedrock,
            model: "anthropic.claude-sonnet-4-5-v1:0".into(),
            base_url: None,
            headers: vec![],
            context_window: 8000,
            supports_images: false,
            reasoning_effort: None,
            reasoning_summary: None,
            resource_name: None,
            credentials: Credentials::None,
        };
        let Err(error) = models.resolve(provider, Uuid::new_v4()).await else {
            anyhow::bail!("missing credentials must fail before making a request");
        };
        assert_eq!(error.code, ChatErrorCode::ProviderConfig);
        assert!(
            error
                .message
                .contains("access key ID, secret access key, and region")
        );

        let provider = ProviderConfig {
            id: "fixture".into(),
            kind: ProviderKind::Bedrock,
            model: "anthropic.claude-sonnet-4-5-v1:0".into(),
            base_url: None,
            headers: vec![],
            context_window: 8000,
            supports_images: false,
            reasoning_effort: None,
            reasoning_summary: None,
            resource_name: None,
            credentials: credentials("fixture-key", None, "us-east-1"),
        };
        models.resolve(provider, Uuid::new_v4()).await?;
        Ok(())
    }

    #[tokio::test]
    async fn signed_reasoning_and_tool_input_survive_the_follow_up() -> anyhow::Result<()> {
        use crate::model::route::{Route, Wire};
        use crate::model::{
            Backend, Effort, Image, Model, ModelMessage, Reasoning, StepRequest, ToolSpec, Usage,
        };
        use crate::turn::{FinishReason, TurnEvent};
        use serde_json::json;

        let (base, seen, server) = event_stream::serve(vec![
            vec![
                ("messageStart", json!({"role": "assistant"})),
                ("contentBlockDelta", json!({"contentBlockIndex": 0, "delta": {"reasoningContent": {"text": "thinking"}}})),
                ("contentBlockDelta", json!({"contentBlockIndex": 0, "delta": {"reasoningContent": {"signature": "opaque-signature"}}})),
                ("contentBlockStop", json!({"contentBlockIndex": 0})),
                ("contentBlockStart", json!({"contentBlockIndex": 1, "start": {"toolUse": {"toolUseId": "tool-1", "name": "run"}}})),
                ("contentBlockDelta", json!({"contentBlockIndex": 1, "delta": {"toolUse": {"input": "{\"code\":"}}})),
                ("contentBlockDelta", json!({"contentBlockIndex": 1, "delta": {"toolUse": {"input": "\"1+1\"}"}}})),
                ("contentBlockStop", json!({"contentBlockIndex": 1})),
                ("messageStop", json!({"stopReason": "tool_use"})),
                ("metadata", json!({"usage": {"inputTokens": 14, "outputTokens": 8, "totalTokens": 22}, "metrics": {"latencyMs": 1}})),
            ],
            vec![
                ("messageStart", json!({"role": "assistant"})),
                ("contentBlockDelta", json!({"contentBlockIndex": 0, "delta": {"text": "Two"}})),
                ("contentBlockStop", json!({"contentBlockIndex": 0})),
                ("messageStop", json!({"stopReason": "end_turn"})),
                ("metadata", json!({"usage": {"inputTokens": 30, "outputTokens": 1, "totalTokens": 31}, "metrics": {"latencyMs": 1}})),
            ],
        ])
        .await?;
        let provider = ProviderConfig {
            id: "fixture".into(),
            kind: ProviderKind::Bedrock,
            model: "anthropic.claude-sonnet-4-5-v1:0".into(),
            base_url: None,
            headers: vec![("x-conversation".into(), "fixture-conversation".into())],
            context_window: 8000,
            supports_images: true,
            reasoning_effort: Some("max".into()),
            reasoning_summary: None,
            resource_name: None,
            credentials: credentials("fixture-key", None, "us-east-1"),
        };
        let (rig, secrets) = model_at(&provider, Some(&base))?;
        let model = Model {
            route: Backend::Rig(Box::new(Route {
                model: rig,
                wire: Wire::Bedrock,
                url: None,
                secrets,
            })),
            name: provider.model.clone(),
            window: provider.context_window,
            supports_images: true,
            kind: Some(ProviderKind::Bedrock),
            reasoning: Reasoning {
                effort: provider.reasoning_effort.clone(),
                summary: None,
            },
        };
        let tools = [ToolSpec {
            name: "run".into(),
            description: "Evaluate code".into(),
            input_schema: json!({"type": "object"}),
        }];
        let question = || ModelMessage::User {
            text: "What is 1+1?".into(),
            images: vec![],
        };
        let mut events = Vec::new();
        let first = [question()];
        let turn = model
            .step(
                &StepRequest {
                    system: "Help",
                    messages: &first,
                    tools: &tools,
                    effort: Some(Effort::High),
                },
                &mut events,
            )
            .await?;
        assert_eq!(turn.finish(), FinishReason::ToolCalls);
        assert_eq!(
            turn.usage(),
            Some(Usage {
                input_tokens: 14,
                output_tokens: 8
            })
        );
        let call = turn.tool_calls()[0].clone();
        assert_eq!(call.input, json!({"code": "1+1"}));
        assert!(
            matches!(&events[..], [TurnEvent::Reasoning(text), TurnEvent::ToolStarted { id, .. }]
            if text == "thinking" && *id == call.id)
        );

        let follow_up = [
            question(),
            ModelMessage::Assistant(turn),
            ModelMessage::ToolResult {
                id: call.id.clone(),
                text: "2".into(),
            },
            ModelMessage::User {
                text: "Attached image from the tool result:".into(),
                images: vec![Image {
                    media_type: "image/png".into(),
                    bytes: vec![1, 2, 3],
                }],
            },
        ];
        let turn = model
            .step(
                &StepRequest {
                    system: "Help",
                    messages: &follow_up,
                    tools: &tools,
                    effort: Some(Effort::High),
                },
                &mut events,
            )
            .await?;
        assert_eq!(turn.finish(), FinishReason::Stop);
        assert_eq!(events.last(), Some(&TurnEvent::Text("Two".into())));
        server.abort();

        let seen = seen.lock().map_err(|_| anyhow::anyhow!("capture lock"))?;
        assert_eq!(seen.len(), 2);
        for (headers, body) in seen.iter() {
            assert_eq!(headers["x-conversation"], "fixture-conversation");
            let authorization = headers["authorization"].to_str()?;
            assert!(authorization.contains("Credential=fixture-key/"));
            assert!(
                authorization.contains("x-conversation"),
                "the custom header is signed"
            );
            assert_eq!(
                body["additionalModelRequestFields"],
                json!({"thinking": {"type": "enabled", "budget_tokens": 32000}})
            );
            assert_eq!(body["inferenceConfig"]["maxTokens"], 36_096);
        }
        let messages = seen[1].1["messages"].as_array().context("messages")?;
        assert_eq!(
            messages[1]["content"],
            json!([
                {"reasoningContent": {"reasoningText": {"text": "thinking", "signature": "opaque-signature"}}},
                {"toolUse": {"toolUseId": "tool-1", "name": "run", "input": {"code": "1+1"}}}
            ]),
            "signed reasoning and the call replay with the provider's own id"
        );
        let user = messages[2]["content"].as_array().context("user content")?;
        assert_eq!(user[0]["toolResult"]["toolUseId"], "tool-1");
        assert_eq!(user[1]["text"], "Attached image from the tool result:");
        assert_eq!(user[2]["image"]["format"], "png");
        Ok(())
    }

    #[test]
    fn a_throttle_keeps_the_delay_bedrock_asked_for() {
        let throttled = ProviderError::from_http_response(
            http::StatusCode::TOO_MANY_REQUESTS,
            r#"{"message":"Too many requests, please try again in 2s."}"#,
        );
        let card = error(&throttled, &HeaderMap::new());
        assert_eq!(card.code, ChatErrorCode::RateLimited);
        assert_eq!(card.retry_after_seconds, Some(2));
        let unreadable = ProviderError::Response("not a Converse reply".into());
        assert_eq!(
            error(&unreadable, &HeaderMap::new()).code,
            ChatErrorCode::Unknown,
            "an unreadable reply is not a dropped connection"
        );
    }
}
