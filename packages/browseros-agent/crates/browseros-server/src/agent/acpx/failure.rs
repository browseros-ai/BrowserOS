//! Translates failures at the acpx process boundary into the chat's error vocabulary.
//! Adapter messages may contain upstream response bodies, so cards use local copy.

use serde_json::Value;

use super::Adapter;
use crate::turn::{ChatError, ChatErrorCode};

/// A classified launcher or service failure, before the runner builds a display-safe card.
#[derive(Debug)]
pub(super) enum Failure {
    Auth,
    MissingSession,
    RejectedModel(Option<String>),
    Service(ChatErrorCode, Option<u16>),
    Runtime(&'static str),
    Io(String),
}

impl Failure {
    pub(super) const PROMPT: Self = Self::Runtime(
        "The coding agent rejected the request. Check the agent settings, then retry.",
    );

    pub(super) fn io(error: impl std::fmt::Display) -> Self {
        Self::Io(error.to_string())
    }

    pub(super) fn card(self, adapter: Adapter, model: Option<&str>) -> ChatError {
        use ChatErrorCode::*;
        let name = adapter.name();
        let mut error = match self {
            Self::Auth => ChatError::new(
                AuthFailed,
                format!("{name} sign-in required"),
                format!("Sign in using the {name} CLI, then retry this turn."),
            ),
            Self::RejectedModel(reported) => {
                let model = reported.as_deref().or(model).map_or_else(
                    || "The selected model".into(),
                    |model| format!("Model \"{model}\""),
                );
                ChatError::new(
                    ProviderConfig,
                    "Model unavailable",
                    format!(
                        "{model} cannot be used with this {name} sign-in. Choose Agent default or another model in AI settings."
                    ),
                )
            }
            Self::Io(detail) => {
                tracing::debug!(%detail, "ACP process or state I/O failed");
                ChatError::new(
                    ConnectionFailed,
                    format!("{name} could not start"),
                    "Check that the coding agent launcher and its working directory are available.",
                )
            }
            Self::MissingSession => ChatError::new(
                ConnectionFailed,
                format!("{name} session missing"),
                "Retry to start the coding agent session again.",
            ),
            Self::Runtime(message) => ChatError::new(
                ProviderUnavailable,
                format!("{name} could not finish"),
                message,
            ),
            Self::Service(code, status) => {
                let message = match code {
                    AuthFailed => "Check your coding agent sign-in in AI settings.",
                    ProviderConfig => "Check your coding agent settings and retry.",
                    RateLimited => "The coding agent service is limiting requests or usage.",
                    ContextLength => "Shorten the prompt or start a new conversation.",
                    ContentFilter => "The coding agent service rejected the content.",
                    ConnectionFailed => "The connection to the coding agent service failed.",
                    _ => "The coding agent service could not complete the request.",
                };
                let mut error = ChatError::new(code, format!("{name} request failed"), message);
                error.status_code = status;
                error
            }
        };
        error.provider = Some(name.into());
        error
    }
}

pub(super) fn error_response(message: &Value, model: Option<&str>) -> Option<Failure> {
    let error = message.get("error")?;
    let code = error["code"].as_i64()?;
    let text = error["message"].as_str().unwrap_or_default();
    let data = &error["data"];
    if (code == -32000
        && text
            .to_ascii_lowercase()
            .contains("authentication required"))
        || matches!(
            data["errorKind"].as_str(),
            Some("authentication_failed" | "oauth_org_not_allowed")
        )
    {
        return Some(Failure::Auth);
    }
    if text.starts_with("No named session ") {
        return Some(Failure::MissingSession);
    }
    if data["errorKind"] == "model_not_found"
        || model.is_some_and(|model| selected_model_rejection(text, data, model))
    {
        return Some(Failure::RejectedModel(None));
    }
    use ChatErrorCode::*;
    let service = match (data["errorKind"].as_str(), data["codexErrorInfo"].as_str()) {
        (Some("rate_limit" | "billing_error" | "account_on_hold"), _)
        | (_, Some("usageLimitExceeded" | "rateLimitExceeded")) => RateLimited,
        (Some("max_output_tokens"), _) | (_, Some("contextWindowExceeded")) => ContextLength,
        (_, Some("cyberPolicy" | "misalignmentPolicyViolation")) => ContentFilter,
        (_, Some("unauthorized")) => AuthFailed,
        (Some("invalid_request"), _) | (_, Some("badRequest")) => ProviderConfig,
        _ => return Some(Failure::PROMPT),
    };
    Some(Failure::Service(service, None))
}

fn selected_model_rejection(text: &str, data: &Value, model: &str) -> bool {
    // acpx can refuse a model at the catalog check, or wrap Claude's model
    // control error inside a CLI error. Match the requested selector in both.
    let model_not_found = format!("Model '{model}' not found");
    text == model_not_found
        || (data["origin"] == "cli"
            && (data["details"] == model_not_found
                || ["apply --model", "replay saved model"]
                    .into_iter()
                    .any(|action| {
                        text.starts_with(&format!(
                        "Cannot {action} \"{model}\": the ACP agent did not advertise that model."
                    ))
                    })))
}

pub(super) fn codex_metadata_warning(text: &str) -> bool {
    text.starts_with("Warning: Model metadata for `")
        && text.ends_with(
            "` not found. Defaulting to fallback metadata; this can degrade performance and cause issues.\n\n",
        )
}

pub(super) fn codex_text_error(text: &str) -> Option<Failure> {
    // The pinned adapter appends two newlines to terminal service diagnostics.
    // Its legacy warnings and context-compaction notice use the same delimiter.
    let body = text.strip_suffix("\n\n")?;
    if body.trim().is_empty()
        || body.starts_with("Warning: ")
        || body.starts_with("Config warning: ")
        || body == "*Context compacted to fit the model's context window.*"
    {
        return None;
    }
    Some(codex_http_error(body).unwrap_or(Failure::PROMPT))
}

fn codex_http_error(body: &str) -> Option<Failure> {
    let body: Value = serde_json::from_str(body).ok()?;
    let status = u16::try_from(body["status"].as_u64()?).ok()?;
    let error = &body["error"];
    if body["type"] != "error" || !(400..=599).contains(&status) {
        return None;
    }
    let kind = error["type"].as_str()?;
    let message = error["message"].as_str()?;
    let code = error["code"].as_str().unwrap_or_default();
    if code == "model_not_found" {
        let model = message
            .strip_prefix("The '")
            .and_then(|message| message.split_once("' model"))
            .map(|(model, _)| model)
            .filter(|model| {
                !model.is_empty()
                    && model.len() <= 128
                    && model
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || b"-_.:/".contains(&byte))
            });
        return Some(Failure::RejectedModel(model.map(str::to_owned)));
    }
    let code = match (status, code, kind) {
        (401 | 403, _, _) => ChatErrorCode::AuthFailed,
        (429, _, _) | (_, "insufficient_quota", _) => ChatErrorCode::RateLimited,
        (413, _, _) | (_, "context_length_exceeded", _) => ChatErrorCode::ContextLength,
        (_, "content_filter" | "content_policy_violation", _) => ChatErrorCode::ContentFilter,
        (400 | 404, _, "invalid_request_error") => ChatErrorCode::ProviderConfig,
        _ => ChatErrorCode::ProviderUnavailable,
    };
    Some(Failure::Service(code, Some(status)))
}
