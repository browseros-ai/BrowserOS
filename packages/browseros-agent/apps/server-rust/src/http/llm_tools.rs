//! One-shot LLM calls used by the provider settings and scheduled-task editor. Provider
//! selection and secret ownership stay here; the shared agent crate owns model transports.
//! Unlike chat, each call substitutes a fresh UUID in custom headers (TS pipeline F11).

use crate::{AppState, db::ProviderRow, oauth::OAuthProvider};
use axum::{
    Json,
    extract::State,
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

pub async fn test_provider(
    State(state): State<AppState>,
    super::extract::ZodJson(body): super::extract::ZodJson<Value>,
) -> Response {
    if let Err(issues) = validate(&body, false) {
        return super::errors::zod_400(&issues);
    }
    let mut config = config_from_body(&body);
    if let Some(id) = nonempty_string(&body, "providerId") {
        match state.providers.get(id).await {
            Ok(Some(row)) => merge_stored_credentials(&mut config, &row),
            Ok(None) => {}
            Err(error) => return internal_error(&error.to_string()),
        }
    }

    // Elapsed time includes credential/model resolution, just like testProviderConnection.
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
                if let Some(surrogate) = units
                    .last()
                    .copied()
                    .filter(|unit| (0xd800..=0xdbff).contains(unit))
                {
                    // JS slice can split an emoji. Keep its unpaired UTF-16 unit escaped in
                    // JSON instead of replacing it with U+FFFD, which changes the TS preview.
                    let prefix = String::from_utf16_lossy(&units[..units.len() - 1]);
                    let prefix = serde_json::to_string(&format!(
                        "Connection successful. Response: \"{prefix}"
                    ))
                    .unwrap_or_default();
                    let prefix = prefix.strip_suffix('"').unwrap_or_default();
                    let body = format!(
                        r#"{{"success":true,"message":{prefix}\u{surrogate:04x}...\"","responseTime":{response_time}}}"#
                    );
                    return (
                        StatusCode::OK,
                        [(http::header::CONTENT_TYPE, "application/json")],
                        body,
                    )
                        .into_response();
                }
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

pub async fn refine_prompt(
    State(state): State<AppState>,
    super::extract::ZodJson(body): super::extract::ZodJson<Value>,
) -> Response {
    if let Err(issues) = validate(&body, true) {
        return super::errors::zod_400(&issues);
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
        Ok(text) if !js_trim(&text).is_empty() => {
            Json(json!({"success": true, "refined": js_trim(&text)})).into_response()
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
            return if is_llm(&row) {
                Ok(Some(row))
            } else {
                Err(failure(&format!(
                    "Provider {} is a coding agent and cannot refine a prompt",
                    row.id
                )))
            };
        }
        // The stores split LLM and ACP rows. An explicitly named coding agent must fail,
        // whereas a coding-agent default is allowed to fall back to the first LLM row.
        if let Ok(agent_id) = Uuid::parse_str(id)
            && agent_id.to_string() == id
            && state
                .agents
                .get(agent_id)
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
        .filter(is_llm)
    {
        return Ok(Some(row));
    }
    let rows = state
        .providers
        .list()
        .await
        .map_err(|e| internal_error(&e.to_string()))?;
    let row = rows
        .into_iter()
        .find(is_llm)
        .ok_or_else(|| failure(NO_LLM))?;
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

fn is_llm(row: &ProviderRow) -> bool {
    row.provider_type.parse::<ProviderKind>().is_ok()
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
    // TS creates AbortSignal.timeout only after resolving OAuth and creating the model.
    // Dropping generation closes the response body; cancellation also reaches adapters
    // that keep a stream task alive beyond that future.
    let cancel = CancellationToken::new();
    let result = tokio::time::timeout(
        timeout,
        llm::generate_text(model.as_ref(), system, prompt, cancel.clone()),
    )
    .await;
    cancel.cancel();
    result
        .map_err(|_| "The operation timed out.".to_owned())?
        .map_err(|error| error.to_string())
}

fn string<'a>(body: &'a Value, key: &str) -> Option<&'a str> {
    body.get(key).and_then(Value::as_str)
}

fn nonempty_string<'a>(body: &'a Value, key: &str) -> Option<&'a str> {
    string(body, key).filter(|value| !value.is_empty())
}

fn js_trim(text: &str) -> &str {
    // ECMAScript trims BOM but not NEL (U+0085); Rust's str::trim does the reverse.
    text.trim_matches(|c| {
        matches!(c,
        '\u{0009}'..='\u{000d}' | ' ' | '\u{00a0}' | '\u{1680}' | '\u{2000}'..='\u{200a}'
        | '\u{2028}' | '\u{2029}' | '\u{202f}' | '\u{205f}' | '\u{3000}' | '\u{feff}')
    })
}

fn internal_error(message: &str) -> Response {
    super::errors::chat_envelope_500(&llm::to_chat_error(
        &llm::ModelError::Config(message.to_owned()),
        None,
    ))
}

// Issue order follows the Zod object's declared fields, rather than request key order. Unknown
// keys are ignored, and optional fields still reject null; serde Option<T> cannot model that.
fn validate(body: &Value, refine: bool) -> Result<(), Vec<super::extract::ZodIssue>> {
    let mut issues = Vec::new();
    if !body.is_object() {
        issues.push(invalid_type("object", Some(body), vec![]));
    } else {
        let providers: Vec<_> = ProviderKind::ALL.iter().map(|kind| kind.as_str()).collect();
        validate_enum(body, "provider", &providers, !refine, &mut issues);
        validate_string(body, "providerId", false, None, &mut issues);
        validate_string(
            body,
            "model",
            !refine,
            Some(if refine {
                "Too small: expected string to have >=1 characters"
            } else {
                "Model name is required"
            }),
            &mut issues,
        );
        for key in ["apiKey", "baseUrl"] {
            validate_string(body, key, false, None, &mut issues);
        }
        if let Some(headers) = body.get("headers") {
            validate_headers(headers, &mut issues);
        }
        for key in [
            "resourceName",
            "region",
            "accessKeyId",
            "secretAccessKey",
            "sessionToken",
        ] {
            validate_string(body, key, false, None, &mut issues);
        }
        validate_enum(
            body,
            "reasoningEffort",
            &["none", "low", "medium", "high", "xhigh", "max"],
            false,
            &mut issues,
        );
        validate_enum(
            body,
            "reasoningSummary",
            &["auto", "concise", "detailed"],
            false,
            &mut issues,
        );
        if refine {
            validate_string(
                body,
                "prompt",
                true,
                Some("Prompt cannot be empty"),
                &mut issues,
            );
            validate_string(
                body,
                "name",
                true,
                Some("Task name cannot be empty"),
                &mut issues,
            );
        }
        validate_string(body, "upstreamProvider", false, None, &mut issues);
    }
    if issues.is_empty() {
        Ok(())
    } else {
        Err(issues)
    }
}

fn invalid_type(
    expected: &str,
    value: Option<&Value>,
    path: Vec<Value>,
) -> super::extract::ZodIssue {
    let received = match value {
        None => "undefined",
        Some(Value::Null) => "null",
        Some(Value::Array(_)) => "array",
        Some(Value::Object(_)) => "object",
        Some(Value::Bool(_)) => "boolean",
        Some(Value::Number(_)) => "number",
        Some(Value::String(_)) => "string",
    };
    super::extract::ZodIssue {
        code: "invalid_type".to_owned(),
        path,
        message: format!("Invalid input: expected {expected}, received {received}"),
        attributes: json!({"expected":expected}),
    }
}

fn validate_string(
    body: &Value,
    key: &str,
    required: bool,
    minimum_message: Option<&str>,
    issues: &mut Vec<super::extract::ZodIssue>,
) {
    let value = body.get(key);
    if value.is_none() && !required {
        return;
    }
    match value.and_then(Value::as_str) {
        None => issues.push(invalid_type("string", value, vec![json!(key)])),
        Some("") if minimum_message.is_some() => issues.push(super::extract::ZodIssue {
            code: "too_small".to_owned(),
            path: vec![json!(key)],
            message: minimum_message.unwrap_or_default().to_owned(),
            attributes: json!({"origin":"string","minimum":1,"inclusive":true}),
        }),
        _ => {}
    }
}

fn validate_enum(
    body: &Value,
    key: &str,
    allowed: &[&str],
    required: bool,
    issues: &mut Vec<super::extract::ZodIssue>,
) {
    let value = body.get(key);
    if value.is_none() && !required {
        return;
    }
    if !value
        .and_then(Value::as_str)
        .is_some_and(|value| allowed.contains(&value))
    {
        let options = allowed
            .iter()
            .map(|value| format!("\"{value}\""))
            .collect::<Vec<_>>()
            .join("|");
        issues.push(super::extract::ZodIssue {
            code: "invalid_value".to_owned(),
            path: vec![json!(key)],
            message: format!("Invalid option: expected one of {options}"),
            attributes: json!({"values":allowed}),
        });
    }
}

fn validate_headers(value: &Value, issues: &mut Vec<super::extract::ZodIssue>) {
    use super::extract::ZodIssue;
    const NAME_PATTERN: &str = r"/^[!#$%&'*+.^_`|~0-9A-Za-z-]+(?![\s\S])/";
    const VALUE_PATTERN: &str = r"/^[\t\x20-\x7e\x80-\xff]*(?![\s\S])/";
    let Some(headers) = value.as_object() else {
        issues.push(invalid_type("record", Some(value), vec![json!("headers")]));
        return;
    };
    let mut aborted = false;
    for (name, value) in headers {
        let path = vec![json!("headers"), json!(name)];
        if name.is_empty()
            || !name
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"!#$%&'*+.^_`|~-".contains(&c))
        {
            aborted = true;
            issues.push(ZodIssue {
                code:"invalid_key".to_owned(), path, message:"Invalid key in record".to_owned(),
                attributes:json!({"origin":"record","issues":[{
                    "origin":"string","code":"invalid_format","format":"regex","pattern":NAME_PATTERN,
                    "path":[],"message":format!("Invalid string: must match pattern {NAME_PATTERN}")
                }]}),
            });
            // Zod does not validate the value when the record key already failed.
            continue;
        }
        match value.as_str() {
            None => {
                aborted = true;
                issues.push(invalid_type("string", Some(value), path));
            }
            Some(value)
                if !value.chars().all(|c| {
                    c == '\t' || (' '..='~').contains(&c) || ('\u{80}'..='\u{ff}').contains(&c)
                }) =>
            {
                issues.push(ZodIssue {
                    code: "invalid_format".to_owned(),
                    path,
                    message: format!("Invalid string: must match pattern {VALUE_PATTERN}"),
                    attributes: json!({"origin":"string","format":"regex","pattern":VALUE_PATTERN}),
                })
            }
            _ => {}
        }
    }
    // superRefine runs only when parsing did not abort. A regex failure is continuable;
    // an invalid key/type aborts and must not introduce spurious duplicate-name issues.
    if !aborted {
        let mut names = std::collections::BTreeSet::new();
        for name in headers.keys() {
            if !names.insert(name.to_ascii_lowercase()) {
                issues.push(ZodIssue::new(
                    "custom",
                    vec![json!("headers"), json!(name)],
                    "Duplicate header name",
                ));
            }
        }
    }
}

fn refinement_system_prompt(name: &str) -> String {
    format!(
        "You are helping a user write a prompt for a scheduled browser automation task called \"{name}\".\n\nThis prompt will be executed automatically on a recurring schedule by an AI agent that can fully control a browser — navigate sites, click, type, read content, and take screenshots.\n\nRewrite the user's rough prompt into a clear, natural instruction. Make it:\n- Specific about what to do and where (which websites, what pages, what to look for)\n- Clear about what result to return at the end (a summary, key data points, changes detected, etc.)\n- Complete enough to run unattended — the agent can't ask follow-up questions\n\nIf the user's prompt is too vague to fill in specifics, use natural placeholders like [your competitor's URL] that they can easily spot and replace.\n\nWrite it as a natural instruction — like telling a capable assistant what to do. Keep it concise. Return ONLY the rewritten prompt, nothing else."
    )
}
