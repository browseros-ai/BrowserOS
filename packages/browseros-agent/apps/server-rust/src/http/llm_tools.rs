//! One-shot LLM calls used by the provider settings and scheduled-task editor. Provider
//! selection and secret ownership stay here; the shared agent crate owns model transports.
//! Unlike chat, each call substitutes a fresh UUID in custom headers (TS pipeline F11).

use crate::{AppState, db::ProviderRow, oauth::OAuthProvider};
use axum::{
    Json,
    extract::{Request, State},
    response::{IntoResponse, Response},
};
use browseros_agent::llm::{self, LanguageModel, LlmConfig, ProviderKind, ScriptedModel};
use http::StatusCode;
use serde_json::{Value, json};
use std::{collections::BTreeMap, sync::Arc, time::Duration};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

const NO_LLM: &str = "Connect an LLM provider in AI settings to refine a prompt";
const TEST_PROMPT: &str = "Respond with exactly: 'ok'";
const TEST_TIMEOUT: Duration = Duration::from_secs(15);
const REFINE_TIMEOUT: Duration = Duration::from_secs(30);

pub async fn test_provider(State(state): State<AppState>, request: Request) -> Response {
    let body = match read_body(request).await {
        Ok(body) => body,
        Err(response) => return response,
    };
    if let Err(response) = validate(&body, false) {
        return response;
    }
    let mut config = config_from_body(&body);
    if let Some(id) = nonempty_string(&body, "providerId") {
        match state.providers.get(id).await {
            Ok(Some(row)) => merge_stored_credentials(&mut config, &row),
            Ok(None) => {}
            Err(error) => return internal_error(&error.to_string()),
        }
    }

    // The deadline covers OAuth resolution and the entire text stream, including a stalled
    // provider. Dropping the generation future closes the HTTP body; cancellation also reaches
    // adapters that keep a stream task alive beyond that future.
    let start = std::time::Instant::now();
    let result = generate(&state, config.clone(), None, TEST_PROMPT, TEST_TIMEOUT).await;
    let response_time = (start.elapsed().as_secs_f64() * 1000.0).round() as u64;
    let (success, message) = match result {
        Ok(text) if text.is_empty() => (
            true,
            "Connection successful. Provider responded.".to_owned(),
        ),
        Ok(text) => {
            let preview = if text.encode_utf16().count() > 100 {
                let units: Vec<_> = text.encode_utf16().take(100).collect();
                format!("{}...", String::from_utf16_lossy(&units))
            } else {
                text
            };
            (
                true,
                format!("Connection successful. Response: \"{preview}\""),
            )
        }
        Err(error) => (false, format!("[{}] {error}", config.provider.as_str())),
    };
    (
        if success {
            StatusCode::OK
        } else {
            StatusCode::BAD_REQUEST
        },
        Json(json!({"success": success, "message": message, "responseTime": response_time})),
    )
        .into_response()
}

pub async fn refine_prompt(State(state): State<AppState>, request: Request) -> Response {
    let body = match read_body(request).await {
        Ok(body) => body,
        Err(response) => return response,
    };
    if let Err(response) = validate(&body, true) {
        return response;
    }
    let row = match resolve_refine_provider(&state, &body).await {
        Ok(row) => row,
        Err(response) => return response,
    };
    let mut config = config_from_body(&body);
    if let Some(row) = row {
        // Stored nullable values overwrite inline values too. Otherwise a cleared key or URL
        // could silently resurrect a stale client copy and send credentials to the wrong host.
        if row.model_id.as_deref().is_none_or(str::is_empty) {
            return super::errors::success_false(StatusCode::BAD_REQUEST, NO_LLM);
        }
        config = config_from_row(&row, config.upstream_provider);
    }
    if config.model.is_empty() {
        return super::errors::success_false(StatusCode::BAD_REQUEST, NO_LLM);
    }
    let system = refinement_system_prompt(string(&body, "name").unwrap_or_default());
    match generate(
        &state,
        config,
        Some(&system),
        string(&body, "prompt").unwrap_or_default(),
        REFINE_TIMEOUT,
    )
    .await
    {
        Ok(text) if !text.trim().is_empty() => {
            Json(json!({"success": true, "refined": text.trim()})).into_response()
        }
        Ok(_) => super::errors::success_false(
            StatusCode::BAD_REQUEST,
            "Provider returned an empty response",
        ),
        Err(error) => super::errors::success_false(StatusCode::BAD_REQUEST, &error),
    }
}

async fn resolve_refine_provider(
    state: &AppState,
    body: &Value,
) -> Result<Option<ProviderRow>, Response> {
    let failure = |message: &str| super::errors::success_false(StatusCode::BAD_REQUEST, message);
    if let Some(id) = nonempty_string(body, "providerId") {
        let row = state
            .providers
            .get(id)
            .await
            .map_err(|e| internal_error(&e.to_string()))?;
        if let Some(row) = row {
            return Ok(Some(row));
        }
        // The stores split LLM and ACP rows. An explicitly named coding agent must fail,
        // whereas a coding-agent default is allowed to fall back to the first LLM row.
        if let Ok(id) = Uuid::parse_str(id)
            && state
                .agents
                .get(id)
                .await
                .map_err(|e| internal_error(&e.to_string()))?
                .is_some()
        {
            return Err(failure(&format!(
                "Provider {id} is a coding agent and cannot refine a prompt"
            )));
        }
        return Err(failure(&format!("Unknown provider {id}")));
    }
    if nonempty_string(body, "provider").is_some() && nonempty_string(body, "model").is_some() {
        return Ok(None);
    }
    if let Some(row) = state
        .providers
        .default()
        .await
        .map_err(|e| internal_error(&e.to_string()))?
    {
        return Ok(Some(row));
    }
    let rows = state
        .providers
        .list()
        .await
        .map_err(|e| internal_error(&e.to_string()))?;
    let row = rows.into_iter().next().ok_or_else(|| failure(NO_LLM))?;
    // TS lists public rows then re-reads credentials. Preserve that second lookup: deletion
    // between the two reads means no LLM, rather than using a row that no longer exists.
    state
        .providers
        .get(&row.id)
        .await
        .map_err(|e| internal_error(&e.to_string()))?
        .map(Some)
        .ok_or_else(|| failure(NO_LLM))
}

fn merge_stored_credentials(config: &mut LlmConfig, row: &ProviderRow) {
    if row.provider_type != config.provider.as_str() {
        return;
    }
    let mut reused = false;
    for (target, saved) in [
        (&mut config.api_key, &row.api_key),
        (&mut config.access_key_id, &row.access_key_id),
        (&mut config.secret_access_key, &row.secret_access_key),
        (&mut config.session_token, &row.session_token),
    ] {
        if target.as_deref().is_none_or(str::is_empty)
            && saved.as_deref().is_some_and(|value| !value.is_empty())
        {
            *target = saved.clone();
            reused = true;
        }
    }
    // A saved secret belongs to its saved destination. Pin all three endpoint selectors even
    // if a saved selector is absent, so an attacker cannot redirect one reused credential.
    if reused {
        config.base_url = row.base_url.clone();
        config.resource_name = row.resource_name.clone();
        config.region = row.region.clone();
    }
}

fn config_from_row(row: &ProviderRow, upstream_provider: Option<String>) -> LlmConfig {
    let mut config = config_from_body(&json!({
        "provider": row.provider_type, "model": row.model_id,
        "headers": row.headers,
    }));
    config.api_key = row.api_key.clone();
    config.base_url = row.base_url.clone();
    config.resource_name = row.resource_name.clone();
    config.region = row.region.clone();
    config.access_key_id = row.access_key_id.clone();
    config.secret_access_key = row.secret_access_key.clone();
    config.session_token = row.session_token.clone();
    config.upstream_provider = upstream_provider;
    config
}

fn config_from_body(body: &Value) -> LlmConfig {
    let id = Uuid::new_v4().to_string();
    LlmConfig {
        // Validation guarantees a known inline provider. An absent provider is a placeholder
        // until refinement resolves its stored row; it must never reach the model factory.
        provider: string(body, "provider")
            .and_then(|p| p.parse().ok())
            .unwrap_or(ProviderKind::OpenAi),
        model: string(body, "model").unwrap_or_default().to_owned(),
        api_key: string(body, "apiKey").map(str::to_owned),
        base_url: string(body, "baseUrl").map(str::to_owned),
        headers: body
            .get("headers")
            .and_then(Value::as_object)
            .map(|headers| {
                headers
                    .iter()
                    .filter_map(|(name, value)| {
                        value
                            .as_str()
                            .map(|value| (name.clone(), value.replace("{{conversationId}}", &id)))
                    })
                    .collect::<BTreeMap<_, _>>()
            }),
        resource_name: string(body, "resourceName").map(str::to_owned),
        region: string(body, "region").map(str::to_owned),
        access_key_id: string(body, "accessKeyId").map(str::to_owned),
        secret_access_key: string(body, "secretAccessKey").map(str::to_owned),
        session_token: string(body, "sessionToken").map(str::to_owned),
        account_id: None,
        upstream_provider: string(body, "upstreamProvider").map(str::to_owned),
        conversation_id: id,
    }
}

async fn generate(
    state: &AppState,
    mut config: LlmConfig,
    system: Option<&str>,
    prompt: &str,
    timeout: Duration,
) -> Result<String, String> {
    let cancel = CancellationToken::new();
    let operation = async {
        let model: Arc<dyn LanguageModel> = if state.config.env.use_mock_llm {
            // Mock resolution precedes OAuth, matching TS: the compatibility harness needs
            // neither a vendor key nor a logged-in subscription to exercise these routes.
            Arc::new(ScriptedModel::mock())
        } else {
            if let Ok(provider) = config.provider.as_str().parse::<OAuthProvider>() {
                let credentials = state
                    .oauth
                    .credentials(provider)
                    .await
                    .map_err(|e| e.to_string())?;
                config.api_key = Some(credentials.access_token);
                if provider == OAuthProvider::ChatGptPro {
                    config.account_id = credentials.account_id;
                    config.upstream_provider = Some("openai".to_owned());
                }
            }
            llm::create_language_model(&config, &state.http).map_err(|e| e.to_string())?
        };
        llm::generate_text(model.as_ref(), system, prompt, cancel.clone())
            .await
            .map_err(|e| e.to_string())
    };
    let result = tokio::time::timeout(timeout, operation).await;
    cancel.cancel();
    result.unwrap_or_else(|_| Err("The operation was aborted due to timeout".to_owned()))
}

fn string<'a>(body: &'a Value, key: &str) -> Option<&'a str> {
    body.get(key).and_then(Value::as_str)
}

fn nonempty_string<'a>(body: &'a Value, key: &str) -> Option<&'a str> {
    string(body, key).filter(|value| !value.is_empty())
}

async fn read_body(request: Request) -> Result<Value, Response> {
    let is_json = request
        .headers()
        .get(http::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .is_some_and(|value| {
            value == "application/json"
                || value.starts_with("application/") && value.ends_with("+json")
        });
    if !is_json {
        return Ok(json!({}));
    }
    let bytes = axum::body::to_bytes(request.into_body(), usize::MAX)
        .await
        .map_err(|_| internal_error("Malformed JSON in request body"))?;
    serde_json::from_slice(&bytes).map_err(|_| internal_error("Malformed JSON in request body"))
}

fn internal_error(message: &str) -> Response {
    super::errors::chat_envelope_500(&llm::to_chat_error(
        &llm::ModelError::Config(message.to_owned()),
        None,
    ))
}

fn validate(body: &Value, refine: bool) -> Result<(), Response> {
    use super::extract::ZodIssue;
    let mut issues = Vec::new();
    let mut issue = |key: &str, code: &str, message: &str| {
        issues.push(ZodIssue {
            code: code.to_owned(),
            path: if key.is_empty() {
                vec![]
            } else {
                vec![json!(key)]
            },
            message: message.to_owned(),
        })
    };
    if !body.is_object() {
        issue("", "invalid_type", "Invalid input: expected object");
    } else {
        for key in [
            "providerId",
            "model",
            "apiKey",
            "baseUrl",
            "resourceName",
            "region",
            "accessKeyId",
            "secretAccessKey",
            "sessionToken",
            "upstreamProvider",
            "prompt",
            "name",
        ] {
            if body.get(key).is_some_and(|value| !value.is_string()) {
                issue(key, "invalid_type", "Invalid input: expected string");
            }
        }
        if let Some(provider) = body.get("provider") {
            if provider
                .as_str()
                .and_then(|provider| provider.parse::<ProviderKind>().ok())
                .is_none()
            {
                issue(
                    "provider",
                    "invalid_value",
                    "Invalid option: expected a supported LLM provider",
                );
            }
        } else if !refine {
            issue(
                "provider",
                "invalid_value",
                "Invalid option: expected a supported LLM provider",
            );
        }
        if (!refine || body.get("model").is_some())
            && nonempty_string(body, "model").is_none()
            && body.get("model").is_none_or(Value::is_string)
        {
            issue(
                "model",
                if body.get("model").is_none() {
                    "invalid_type"
                } else {
                    "too_small"
                },
                if refine {
                    "Too small: expected string to have >=1 characters"
                } else {
                    "Model name is required"
                },
            );
        }
        if refine {
            for (key, message) in [
                ("prompt", "Prompt cannot be empty"),
                ("name", "Task name cannot be empty"),
            ] {
                if body.get(key).is_none() {
                    issue(key, "invalid_type", "Invalid input: expected string");
                } else if string(body, key) == Some("") {
                    issue(key, "too_small", message);
                }
            }
        }
        for (key, allowed) in [
            (
                "reasoningEffort",
                &["none", "low", "medium", "high", "xhigh", "max"][..],
            ),
            ("reasoningSummary", &["auto", "concise", "detailed"][..]),
        ] {
            if let Some(value) = body.get(key)
                && !value.as_str().is_some_and(|value| allowed.contains(&value))
            {
                issue(key, "invalid_value", "Invalid option");
            }
        }
        if let Some(headers) = body.get("headers") {
            match headers.as_object() {
                None => issue("headers", "invalid_type", "Invalid input: expected record"),
                Some(headers) => {
                    let mut names = std::collections::BTreeSet::new();
                    for (name, value) in headers {
                        if name.is_empty()
                            || !name.bytes().all(|c| {
                                c.is_ascii_alphanumeric() || b"!#$%&'*+.^_`|~-".contains(&c)
                            })
                        {
                            issue("headers", "invalid_format", "Invalid header name");
                        }
                        if !value.as_str().is_some_and(|value| {
                            value.chars().all(|c| {
                                c == '\t'
                                    || (' '..='~').contains(&c)
                                    || ('\u{80}'..='\u{ff}').contains(&c)
                            })
                        }) {
                            issue("headers", "invalid_format", "Invalid header value");
                        }
                        if !names.insert(name.to_ascii_lowercase()) {
                            issue("headers", "custom", "Duplicate header name");
                        }
                    }
                }
            }
        }
    }
    if issues.is_empty() {
        Ok(())
    } else {
        Err(super::errors::zod_400(&issues))
    }
}

fn refinement_system_prompt(name: &str) -> String {
    format!(
        "You are helping a user write a prompt for a scheduled browser automation task called \"{name}\".\n\nThis prompt will be executed automatically on a recurring schedule by an AI agent that can fully control a browser — navigate sites, click, type, read content, and take screenshots.\n\nRewrite the user's rough prompt into a clear, natural instruction. Make it:\n- Specific about what to do and where (which websites, what pages, what to look for)\n- Clear about what result to return at the end (a summary, key data points, changes detected, etc.)\n- Complete enough to run unattended — the agent can't ask follow-up questions\n\nIf the user's prompt is too vague to fill in specifics, use natural placeholders like [your competitor's URL] that they can easily spot and replace.\n\nWrite it as a natural instruction — like telling a capable assistant what to do. Keep it concise. Return ONLY the rewritten prompt, nothing else."
    )
}
