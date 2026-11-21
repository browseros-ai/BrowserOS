//! Bedrock ConverseStream uses an SDK client scoped to the resolved turn. Indexed deltas
//! produce the same turn events as the HTTP wires and reconstruct the SDK message for replay.

use std::collections::BTreeMap;

use aws::error::{BuildError, ProvideErrorMetadata};
use aws::types::{
    ContentBlock, ContentBlockDelta, ContentBlockStart, ConversationRole, ConverseStreamOutput,
    ImageBlock, ImageFormat, ImageSource, InferenceConfiguration, Message, ReasoningContentBlock,
    ReasoningContentBlockDelta, ReasoningTextBlock, StopReason, SystemContentBlock, Tool,
    ToolConfiguration, ToolInputSchema, ToolResultBlock, ToolResultContentBlock, ToolSpecification,
    ToolUseBlock, ToolUseBlockStart,
};
use aws_sdk_bedrockruntime as aws;
use aws_smithy_types::{Blob, Document, Number, retry::RetryConfig};
use http::header::{HeaderMap, HeaderName};
use serde_json::{Value, json};

use super::{AssistantTurn, ModelMessage, NativeOutput, Reasoning, StepRequest, Usage};
use super::{errors, headers, tool_call, tool_input, tool_started};
use crate::store::{Credentials, ProviderConfig};
use crate::turn::{ChatError, EventSink, FinishReason, TurnEvent};

pub(super) struct Endpoint {
    client: aws::Client,
    headers: HeaderMap,
    secrets: HeaderMap,
}

pub(super) fn endpoint(provider: &ProviderConfig) -> Result<Endpoint, ChatError> {
    let (config, mut secrets) = config(&provider.credentials)?;
    let headers = headers(&provider.headers)?;
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
    Ok(Endpoint {
        client: aws::Client::from_conf(config),
        headers,
        secrets,
    })
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

fn build_error(_: BuildError) -> ChatError {
    errors::protocol("The Bedrock message was missing a required field.")
}

fn aws_request(
    client: &aws::Client,
    model: &str,
    request: &StepRequest<'_>,
    reasoning: &Reasoning,
) -> Result<aws::operation::converse_stream::builders::ConverseStreamFluentBuilder, ChatError> {
    let mut call = client
        .converse_stream()
        .model_id(model)
        .set_messages(Some(messages(request.messages)?));
    if !request.system.is_empty() {
        call = call.system(SystemContentBlock::Text(request.system.into()));
    }
    if !request.tools.is_empty() {
        let tools = request
            .tools
            .iter()
            .map(|tool| {
                ToolSpecification::builder()
                    .name(&tool.name)
                    .description(&tool.description)
                    .input_schema(ToolInputSchema::Json(document(&tool.input_schema)))
                    .build()
                    .map(Tool::ToolSpec)
                    .map_err(build_error)
            })
            .collect::<Result<Vec<_>, _>>()?;
        call = call.tool_config(
            ToolConfiguration::builder()
                .set_tools(Some(tools))
                .build()
                .map_err(build_error)?,
        );
    }
    if model.contains("anthropic")
        && let Some(effort) = reasoning.effort(
            request.effort,
            &["minimal", "low", "medium", "high", "xhigh", "max"],
        )
    {
        let budget = super::thinking_budget(effort);
        call = call.additional_model_request_fields(document(&json!({
            "thinking": {"type":"enabled","budget_tokens":budget}
        })));
        call = call.inference_config(
            InferenceConfiguration::builder()
                .max_tokens(budget + 4096)
                .build(),
        );
    }
    Ok(call)
}

fn messages(history: &[ModelMessage]) -> Result<Vec<Message>, ChatError> {
    let mut messages = Vec::new();
    let mut user = Vec::new();
    for message in history {
        match message {
            ModelMessage::User { text, images } => {
                if !text.is_empty() {
                    user.push(ContentBlock::Text(text.clone()));
                }
                for image in images {
                    let format = match image.media_type.as_str() {
                        "image/png" => ImageFormat::Png,
                        "image/jpeg" => ImageFormat::Jpeg,
                        "image/webp" => ImageFormat::Webp,
                        "image/gif" => ImageFormat::Gif,
                        _ => {
                            return Err(errors::config(
                                "Bedrock accepts PNG, JPEG, WebP, and GIF images.",
                            ));
                        }
                    };
                    user.push(ContentBlock::Image(
                        ImageBlock::builder()
                            .format(format)
                            .source(ImageSource::Bytes(Blob::new(image.bytes.clone())))
                            .build()
                            .map_err(build_error)?,
                    ));
                }
            }
            ModelMessage::ToolResult { id, text } => user.push(ContentBlock::ToolResult(
                ToolResultBlock::builder()
                    .tool_use_id(&id.0)
                    .content(ToolResultContentBlock::Text(text.clone()))
                    .build()
                    .map_err(build_error)?,
            )),
            ModelMessage::Assistant(turn) => {
                flush_user(&mut messages, &mut user)?;
                messages.push(match &turn.native {
                    NativeOutput::Bedrock(message) => message.as_ref().clone(),
                    _ => Message::builder()
                        .role(ConversationRole::Assistant)
                        .content(ContentBlock::Text(turn.text.clone()))
                        .build()
                        .map_err(build_error)?,
                });
            }
        }
    }
    flush_user(&mut messages, &mut user)?;
    Ok(messages)
}

fn flush_user(messages: &mut Vec<Message>, user: &mut Vec<ContentBlock>) -> Result<(), ChatError> {
    if !user.is_empty() {
        // Keep parallel results together when the agent inserted pictures between them.
        user.sort_by_key(|block| !matches!(block, ContentBlock::ToolResult(_)));
        messages.push(
            Message::builder()
                .role(ConversationRole::User)
                .set_content(Some(std::mem::take(user)))
                .build()
                .map_err(build_error)?,
        );
    }
    Ok(())
}

fn document(value: &Value) -> Document {
    match value {
        Value::Null => Document::Null,
        Value::Bool(value) => Document::Bool(*value),
        Value::String(value) => Document::String(value.clone()),
        Value::Array(values) => Document::Array(values.iter().map(document).collect()),
        Value::Object(values) => Document::Object(
            values
                .iter()
                .map(|(key, value)| (key.clone(), document(value)))
                .collect(),
        ),
        Value::Number(value) => Document::Number(
            value
                .as_u64()
                .map(Number::PosInt)
                .or_else(|| value.as_i64().map(Number::NegInt))
                .unwrap_or_else(|| Number::Float(value.as_f64().unwrap_or_default())),
        ),
    }
}

pub(super) async fn step(
    endpoint: &Endpoint,
    model: &str,
    request: &StepRequest<'_>,
    reasoning: &Reasoning,
    sink: &mut dyn EventSink,
) -> Result<AssistantTurn, ChatError> {
    let headers = endpoint.headers.clone();
    let mut response = aws_request(&endpoint.client, model, request, reasoning)?
        .customize()
        .mutate_request(move |request| {
            for (name, value) in &headers {
                if let Ok(value) = value.to_str() {
                    request
                        .headers_mut()
                        .insert(name.to_string(), value.to_owned());
                }
            }
        })
        .send()
        .await
        .map_err(|error| {
            let status = error
                .raw_response()
                .map(|response| response.status().as_u16());
            let retry = error
                .raw_response()
                .and_then(|response| response.headers().get("retry-after"))
                .and_then(|value| value.parse().ok());
            aws_error(error.as_service_error(), status, retry, &endpoint.secrets)
        })?;
    let mut output = Output::default();
    while let Some(event) = response
        .stream
        .recv()
        .await
        .map_err(|error| aws_error(error.as_service_error(), None, None, &endpoint.secrets))?
    {
        output.event(event, sink).await?;
    }
    output.complete()
}

fn aws_error(
    service: Option<&impl ProvideErrorMetadata>,
    status: Option<u16>,
    retry: Option<u64>,
    secrets: &HeaderMap,
) -> ChatError {
    let Some(service) = service else {
        return match status {
            Some(status) => errors::upstream(Some(status), retry, "", secrets),
            None => errors::disconnected(),
        };
    };
    let status = status.or_else(|| match service.code() {
        Some("AccessDeniedException" | "UnrecognizedClientException" | "ExpiredTokenException") => {
            Some(403)
        }
        Some("ThrottlingException" | "ServiceQuotaExceededException") => Some(429),
        Some(
            "ServiceUnavailableException" | "InternalServerException" | "ModelStreamErrorException",
        ) => Some(503),
        Some("ResourceNotFoundException") => Some(404),
        _ => None,
    });
    let body = json!({"error":{"code":service.code(),"message":service.message()}}).to_string();
    errors::upstream(status, retry, &body, secrets)
}

/// Smithy gives text blocks no start event. Tool blocks start with their identity; reasoning
/// blocks may deliver text and signature independently or a redacted byte string.
#[derive(Default)]
struct Block {
    text: String,
    thinking: String,
    signature: String,
    redacted: Option<Vec<u8>>,
    tool: Option<ToolUseBlockStart>,
    arguments: String,
}

#[derive(Default)]
struct Output {
    blocks: BTreeMap<i32, Block>,
    text: String,
    finish: Option<FinishReason>,
    usage: Option<Usage>,
}

impl Output {
    async fn event(
        &mut self,
        event: ConverseStreamOutput,
        sink: &mut dyn EventSink,
    ) -> Result<(), ChatError> {
        match event {
            ConverseStreamOutput::ContentBlockStart(event) => {
                if let Some(ContentBlockStart::ToolUse(tool)) = event.start {
                    let block = self.blocks.entry(event.content_block_index).or_default();
                    if block.tool.is_some() {
                        return Err(errors::protocol(
                            "Bedrock started the same tool block twice.",
                        ));
                    }
                    tool_started(sink, &tool.tool_use_id, &tool.name).await;
                    block.tool = Some(tool);
                }
            }
            ConverseStreamOutput::ContentBlockDelta(event) => {
                let block = self.blocks.entry(event.content_block_index).or_default();
                match event.delta {
                    Some(ContentBlockDelta::Text(text)) if !text.is_empty() => {
                        block.text.push_str(&text);
                        self.text.push_str(&text);
                        sink.emit(TurnEvent::Text(text)).await;
                    }
                    Some(ContentBlockDelta::ToolUse(tool)) => block.arguments.push_str(&tool.input),
                    Some(ContentBlockDelta::ReasoningContent(delta)) => match delta {
                        ReasoningContentBlockDelta::Text(text) if !text.is_empty() => {
                            block.thinking.push_str(&text);
                            sink.emit(TurnEvent::Reasoning(text)).await;
                        }
                        ReasoningContentBlockDelta::Signature(signature) => {
                            block.signature.push_str(&signature)
                        }
                        ReasoningContentBlockDelta::RedactedContent(blob) => block
                            .redacted
                            .get_or_insert_with(Vec::new)
                            .extend(blob.into_inner()),
                        _ => {}
                    },
                    _ => {}
                }
            }
            ConverseStreamOutput::MessageStop(event) => {
                self.finish = Some(match event.stop_reason {
                    StopReason::EndTurn | StopReason::StopSequence => FinishReason::Stop,
                    StopReason::ToolUse => FinishReason::ToolCalls,
                    StopReason::MaxTokens | StopReason::ModelContextWindowExceeded => {
                        FinishReason::Length
                    }
                    StopReason::ContentFiltered | StopReason::GuardrailIntervened => {
                        FinishReason::ContentFilter
                    }
                    StopReason::MalformedModelOutput | StopReason::MalformedToolUse => {
                        FinishReason::Error
                    }
                    _ => FinishReason::Other,
                });
            }
            ConverseStreamOutput::Metadata(event) => {
                if let Some(usage) = event.usage {
                    self.usage = u64::try_from(usage.input_tokens)
                        .ok()
                        .zip(u64::try_from(usage.output_tokens).ok())
                        .map(|(input_tokens, output_tokens)| Usage {
                            input_tokens,
                            output_tokens,
                        });
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn complete(self) -> Result<AssistantTurn, ChatError> {
        let finish = self.finish.ok_or_else(errors::disconnected)?;
        let mut content = Vec::new();
        let mut calls = Vec::new();
        for block in self.blocks.into_values() {
            if let Some(tool) = block.tool {
                if finish != FinishReason::ToolCalls {
                    continue;
                }
                let input = tool_input(&Value::String(block.arguments))?;
                calls.push(tool_call(
                    &json!(tool.tool_use_id),
                    &json!(tool.name),
                    input.clone(),
                )?);
                content.push(ContentBlock::ToolUse(
                    ToolUseBlock::builder()
                        .tool_use_id(tool.tool_use_id)
                        .name(tool.name)
                        .set_type(tool.r#type)
                        .input(document(&input))
                        .build()
                        .map_err(build_error)?,
                ));
            } else if let Some(redacted) = block.redacted {
                content.push(ContentBlock::ReasoningContent(
                    ReasoningContentBlock::RedactedContent(Blob::new(redacted)),
                ));
            } else if !block.thinking.is_empty() || !block.signature.is_empty() {
                let reasoning = ReasoningTextBlock::builder()
                    .text(block.thinking)
                    .set_signature((!block.signature.is_empty()).then_some(block.signature))
                    .build()
                    .map_err(build_error)?;
                content.push(ContentBlock::ReasoningContent(
                    ReasoningContentBlock::ReasoningText(reasoning),
                ));
            } else if !block.text.is_empty() {
                content.push(ContentBlock::Text(block.text));
            }
        }
        let message = Message::builder()
            .role(ConversationRole::Assistant)
            .set_content(Some(content))
            .build()
            .map_err(build_error)?;
        Ok(AssistantTurn {
            text: self.text,
            calls,
            finish,
            usage: self.usage,
            native: NativeOutput::Bedrock(Box::new(message)),
        })
    }
}

#[cfg(test)]
mod tests {
    use anyhow::Context;
    use aws::types::{
        ContentBlockDeltaEvent, ContentBlockStartEvent, ConverseStreamMetadataEvent,
        MessageStopEvent, TokenUsage, ToolUseBlockDelta,
    };
    use uuid::Uuid;

    use super::*;
    use crate::model::{Effort, Models, ToolSpec};
    use crate::store::{ProviderKind, Secret, Store};
    use crate::turn::{ChatErrorCode, ToolCallId};

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

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let url = format!("http://{}", listener.local_addr()?);
        let (send, mut captured) = tokio::sync::mpsc::channel(2);
        let app = axum::Router::new().fallback(async move |headers: HeaderMap| {
            let _ = send.send(headers).await;
            http::StatusCode::BAD_REQUEST
        });
        let server = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        let east = aws::Client::from_conf(east.to_builder().endpoint_url(&url).build());
        let west = aws::Client::from_conf(west.to_builder().endpoint_url(&url).build());
        let (east_result, west_result) = tokio::join!(
            east.converse_stream().model_id("test-east").send(),
            west.converse_stream().model_id("test-west").send(),
        );
        server.abort();
        assert!(
            east_result.is_err() && west_result.is_err(),
            "the local SDK endpoint returns 400"
        );
        let mut seen = std::collections::BTreeSet::new();
        for _ in 0..2 {
            let headers = captured.recv().await.context("signed request")?;
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
    async fn sdk_events_keep_signed_reasoning_and_tool_input_in_the_follow_up() -> anyhow::Result<()>
    {
        let mut output = Output::default();
        let mut visible = Vec::new();
        let delta = |index, delta| -> Result<_, BuildError> {
            Ok(ConverseStreamOutput::ContentBlockDelta(
                ContentBlockDeltaEvent::builder()
                    .content_block_index(index)
                    .delta(delta)
                    .build()?,
            ))
        };
        for event in [
            delta(
                0,
                ContentBlockDelta::ReasoningContent(ReasoningContentBlockDelta::Text(
                    "thinking".into(),
                )),
            )?,
            delta(
                0,
                ContentBlockDelta::ReasoningContent(ReasoningContentBlockDelta::Signature(
                    "opaque-signature".into(),
                )),
            )?,
            ConverseStreamOutput::ContentBlockStart(
                ContentBlockStartEvent::builder()
                    .content_block_index(1)
                    .start(ContentBlockStart::ToolUse(
                        ToolUseBlockStart::builder()
                            .tool_use_id("tool-1")
                            .name("run")
                            .build()?,
                    ))
                    .build()?,
            ),
            delta(
                1,
                ContentBlockDelta::ToolUse(
                    ToolUseBlockDelta::builder().input("{\"code\":").build()?,
                ),
            )?,
            delta(
                1,
                ContentBlockDelta::ToolUse(ToolUseBlockDelta::builder().input("\"1+1\"}").build()?),
            )?,
            ConverseStreamOutput::MessageStop(
                MessageStopEvent::builder()
                    .stop_reason(StopReason::ToolUse)
                    .build()?,
            ),
            ConverseStreamOutput::Metadata(
                ConverseStreamMetadataEvent::builder()
                    .usage(
                        TokenUsage::builder()
                            .input_tokens(14)
                            .output_tokens(8)
                            .total_tokens(22)
                            .build()?,
                    )
                    .build(),
            ),
        ] {
            output.event(event, &mut visible).await?;
        }
        let turn = output.complete()?;
        assert_eq!(turn.finish(), FinishReason::ToolCalls);
        assert_eq!(
            turn.usage(),
            Some(Usage {
                input_tokens: 14,
                output_tokens: 8
            })
        );
        assert_eq!(turn.tool_calls()[0].input, json!({"code":"1+1"}));
        assert!(
            matches!(&visible[..], [TurnEvent::Reasoning(text), TurnEvent::ToolStarted { id, .. }] if text == "thinking" && id.0 == "tool-1")
        );

        let (config, _) = config(&credentials("fixture-key", None, "us-east-1"))?;
        let client = aws::Client::from_conf(config);
        let messages = [
            ModelMessage::User {
                text: "What is 1+1?".into(),
                images: vec![],
            },
            ModelMessage::Assistant(turn),
            ModelMessage::ToolResult {
                id: ToolCallId("tool-1".into()),
                text: "2".into(),
            },
        ];
        let tools = [ToolSpec {
            name: "run".into(),
            description: "Evaluate code".into(),
            input_schema: json!({"type":"object"}),
        }];
        let request = StepRequest {
            system: "Help",
            messages: &messages,
            tools: &tools,
            effort: Some(Effort::High),
        };
        let reasoning = Reasoning {
            effort: Some("max".into()),
            summary: None,
        };
        let request = aws_request(
            &client,
            "anthropic.claude-sonnet-4-5-v1:0",
            &request,
            &reasoning,
        )?;
        let input = request.as_input();
        let messages = input.get_messages().as_ref().context("Converse input")?;
        assert!(
            matches!(messages[1].content(), [ContentBlock::ReasoningContent(ReasoningContentBlock::ReasoningText(thinking)), ContentBlock::ToolUse(tool)]
            if thinking.signature() == Some("opaque-signature") && tool.tool_use_id() == "tool-1")
        );
        assert!(
            matches!(messages[2].content(), [ContentBlock::ToolResult(result)] if result.tool_use_id() == "tool-1")
        );
        assert_eq!(
            input.get_additional_model_request_fields(),
            &Some(document(
                &json!({"thinking":{"type":"enabled","budget_tokens":32000}})
            ))
        );
        Ok(())
    }
}
