//! Every shared JSON fixture must be accepted by its generated Rust wire model.
//! Inventory parity prevents a new fixture from silently escaping Rust coverage.
use serde::de::DeserializeOwned;
use server_api::models;
use std::collections::BTreeSet;
use std::error::Error;
use std::path::Path;

fn decode<T: DeserializeOwned>(source: &str) -> Result<(), Box<dyn Error>> {
    let _: T = serde_json::from_str(source)?;
    Ok(())
}

#[test]
fn all_contract_fixtures_deserialize() -> Result<(), Box<dyn Error>> {
    let manifest: Vec<serde_json::Value> = serde_json::from_str(include_str!(
        "../../../contracts/server-api/fixtures/manifest.json"
    ))?;
    let expected: BTreeSet<_> = manifest
        .iter()
        .filter_map(|item| item["file"].as_str())
        .collect();
    let mut covered = BTreeSet::new();
    macro_rules! fixture {
        ($name:literal, $model:ty) => {{
            decode::<$model>(include_str!(concat!(
                "../../../contracts/server-api/fixtures/",
                $name
            )))?;
            covered.insert($name);
        }};
    }
    fixture!("health.json", models::HealthResponse);
    fixture!("health-minimal.json", models::HealthResponse);
    fixture!("status.json", models::StatusResponse);
    fixture!("diagnostics.json", models::DiagnosticsResponse);
    fixture!("shutdown.json", models::ShutdownResponse);
    fixture!("success.json", models::SuccessResponse);
    fixture!("error-string.json", models::StringError);
    fixture!("error-message.json", models::MessageError);
    fixture!("error-failure.json", models::FailureMessage);
    fixture!("error-validation.json", models::ValidationError);
    fixture!("error-origin.json", models::HttpAgentErrorEnvelope);
    fixture!("error-chat.json", models::ChatErrorEnvelope);
    fixture!("error-probe-wrapper.json", models::AcpxProbeWrapperError);
    fixture!("provider-read.json", models::PublicProviderResponse);
    fixture!("provider-list.json", models::ProviderList);
    fixture!("provider-write.json", models::ProviderResponse);
    fixture!(
        "provider-default-empty.json",
        models::DefaultProviderResponse
    );
    fixture!("provider-default.json", models::DefaultProviderResponse);
    fixture!("provider-upsert.json", models::UpsertProviderRequest);
    fixture!(
        "provider-default-request.json",
        models::SetDefaultProviderRequest
    );
    fixture!("test-provider-request.json", models::TestProviderRequest);
    fixture!("test-provider-response.json", models::TestProviderResponse);
    fixture!("test-provider-failure.json", models::TestProviderBadRequest);
    fixture!("refine-request.json", models::RefinePromptRequest);
    fixture!("refine-response.json", models::RefinePromptResponse);
    fixture!("refine-failure.json", models::RefinePromptBadRequest);
    fixture!("conversation-list.json", models::ConversationList);
    fixture!("conversation-detail.json", models::ConversationResponse);
    fixture!(
        "conversation-import-request.json",
        models::ImportConversationRequest
    );
    fixture!(
        "conversation-import-legacy-array.json",
        models::ImportConversationRequest
    );
    fixture!(
        "conversation-imported.json",
        models::ImportConversationResponse
    );
    fixture!(
        "conversation-existing.json",
        models::ImportConversationResponse
    );
    fixture!("chat-minimal.json", models::ChatRequest);
    fixture!("chat-browseros.json", models::ChatRequest);
    fixture!("chat-legacy.json", models::ChatRequest);
    fixture!("chat-acp.json", models::ChatRequest);
    fixture!("chat-state.json", models::ConversationRunSnapshot);
    fixture!("chat-panels.json", models::ConversationPanelAssignments);
    fixture!("chat-stopped.json", models::StopChatResponse);
    fixture!("chat-deleted.json", models::DeleteChatResponse);
    fixture!("agent-list.json", models::AgentList);
    fixture!("agent-detail.json", models::AgentResponse);
    fixture!("agent-create.json", models::CreateAgentRequest);
    fixture!("agent-create-builtin.json", models::CreateAgentRequest);
    fixture!("agent-update.json", models::UpdateAgentRequest);
    fixture!("scheduled-job-list.json", models::ScheduledJobList);
    fixture!("scheduled-job-detail.json", models::ScheduledJobResponse);
    fixture!(
        "scheduled-job-upsert.json",
        models::UpsertScheduledJobRequest
    );
    fixture!("scheduled-run-list.json", models::ScheduledJobRunList);
    fixture!("scheduled-run-detail.json", models::ScheduledJobRunResponse);
    fixture!(
        "scheduled-run-upsert.json",
        models::UpsertScheduledJobRunRequest
    );
    fixture!("probe-request.json", models::AcpxProbeRequest);
    fixture!("probe-result.json", models::AcpxProbeResult);
    fixture!("probe-error-result.json", models::AcpxProbeResult);
    fixture!("klavis-list.json", models::ConnectorList);
    fixture!("klavis-oauth.json", models::ConnectorOAuthUrls);
    fixture!("klavis-integrations.json", models::UserIntegrationList);
    fixture!("klavis-add.json", models::AddConnectorResponse);
    fixture!("klavis-success.json", models::ConnectorSuccess);
    fixture!("klavis-request.json", models::ServerNameRequest);
    fixture!("klavis-api-key.json", models::SubmitApiKeyRequest);
    fixture!("oauth-device-code.json", models::OAuthDeviceCode);
    fixture!("oauth-status.json", models::OAuthStatus);
    fixture!("oauth-token.json", models::OAuthTokenRequest);
    fixture!("oauth-token-response.json", models::OAuthTokenResponse);
    fixture!("mcp-status.json", models::McpStatus);
    fixture!("mcp-tools.json", models::McpToolList);
    fixture!("mcp-agents.json", models::McpAgentList);
    fixture!("mcp-install-request.json", models::McpInstallRequest);
    fixture!("mcp-install-result.json", models::McpInstallResult);
    fixture!("chunk-start.json", models::UiMessageChunk);
    fixture!("chunk-start-step.json", models::UiMessageChunk);
    fixture!("chunk-reasoning-start.json", models::UiMessageChunk);
    fixture!("chunk-reasoning-delta.json", models::UiMessageChunk);
    fixture!("chunk-reasoning-end.json", models::UiMessageChunk);
    fixture!("chunk-text-start.json", models::UiMessageChunk);
    fixture!("chunk-text-delta.json", models::UiMessageChunk);
    fixture!("chunk-text-end.json", models::UiMessageChunk);
    fixture!("chunk-tool-input-start.json", models::UiMessageChunk);
    fixture!("chunk-tool-input-delta.json", models::UiMessageChunk);
    fixture!("chunk-tool-input-available.json", models::UiMessageChunk);
    fixture!("chunk-tool-input-error.json", models::UiMessageChunk);
    fixture!("chunk-tool-output-available.json", models::UiMessageChunk);
    fixture!("chunk-tool-output-error.json", models::UiMessageChunk);
    fixture!("chunk-error.json", models::UiMessageChunk);
    fixture!("chunk-finish-step.json", models::UiMessageChunk);
    fixture!("chunk-finish.json", models::UiMessageChunk);
    fixture!("chunk-abort.json", models::UiMessageChunk);
    fixture!("chunk-source-url.json", models::UiMessageChunk);
    fixture!("chunk-file.json", models::UiMessageChunk);
    assert_eq!(expected, covered);
    let directory =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../contracts/server-api/fixtures");
    let files: BTreeSet<_> = std::fs::read_dir(directory)?
        .map(|entry| entry.map(|entry| entry.file_name().to_string_lossy().into_owned()))
        .collect::<Result<_, _>>()?;
    let mut indexed: BTreeSet<_> = covered.into_iter().map(String::from).collect();
    indexed.insert("manifest.json".into());
    assert_eq!(files, indexed);
    Ok(())
}

#[test]
fn opaque_values_and_nullable_columns_survive_round_trips() -> Result<(), Box<dyn Error>> {
    let message: models::UiMessage = serde_json::from_str(
        r#"{"id":"m1","role":"assistant","parts":[{"type":"text","text":"Hi"}]}"#,
    )?;
    assert_eq!(serde_json::to_value(message)?["parts"][0]["text"], "Hi");
    let headers: models::LlmHeaders = serde_json::from_str(r#"{"X-Contract":"fixture"}"#)?;
    assert_eq!(serde_json::to_value(headers)?["X-Contract"], "fixture");
    let response: models::DefaultProviderResponse = serde_json::from_str(r#"{"provider":null}"#)?;
    assert_eq!(
        serde_json::to_value(response)?["provider"],
        serde_json::Value::Null
    );
    let chunk: models::UiMessageChunk =
        serde_json::from_str(r#"{"type":"start-step","extension":{"value":1}}"#)?;
    assert_eq!(serde_json::to_value(chunk)?["extension"]["value"], 1);
    Ok(())
}

#[test]
fn request_nullability_and_strict_variants_are_preserved() -> Result<(), Box<dyn Error>> {
    let cleared: models::UpdateAgentRequest = serde_json::from_str(r#"{"modelId":null}"#)?;
    assert_eq!(
        serde_json::to_value(cleared)?,
        serde_json::json!({"modelId": null})
    );
    let omitted: models::UpdateAgentRequest = serde_json::from_str("{}")?;
    assert_eq!(serde_json::to_value(omitted)?, serde_json::json!({}));
    assert!(
        serde_json::from_str::<models::CreateAgentRequest>(
            r#"{"name":"A","type":"codex","extra":true}"#
        )
        .is_err()
    );
    assert!(
        serde_json::from_str::<models::CreateAgentRequest>(r#"{"name":"A","type":"custom"}"#)
            .is_err()
    );
    assert!(serde_json::from_str::<models::UiMessageChunk>(r#"{"type":"unknown"}"#).is_err());
    Ok(())
}
