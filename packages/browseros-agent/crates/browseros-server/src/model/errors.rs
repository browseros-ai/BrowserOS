//! Converts transport and upstream failures into the panel's common envelope. Rig reports a
//! failure as a structured `ProviderError`; the cards, their wording and the retry rule are the
//! server's. Only scrubbed provider text leaves the model layer, including when a service
//! echoes its request headers.

use std::sync::LazyLock;
use std::time::{Duration, SystemTime};

use http::HeaderMap;
use regex::Regex;
use rig_core::error::{ErrorReport, ProviderError};
use rig_core::http_client;
use serde_json::Value;
use url::Url;

use crate::turn::{ChatError, ChatErrorCode};

pub(super) fn config(message: impl Into<String>) -> ChatError {
    ChatError::new(
        ChatErrorCode::ProviderConfig,
        "Check provider settings",
        message,
    )
}

pub(super) fn connect(url: &Url) -> ChatError {
    // A custom provider URL can carry credentials outside its host.
    let message = match url.host_str() {
        Some(host) => format!("Could not connect to the provider at {host}."),
        None => "Could not connect to the provider.".into(),
    };
    connection_error(message)
}

pub(super) fn disconnected() -> ChatError {
    connection_error("The connection to the provider closed before the response finished.")
}

fn connection_error(message: impl Into<String>) -> ChatError {
    let mut error = ChatError::new(
        ChatErrorCode::ConnectionFailed,
        "Could not connect",
        message,
    );
    error.docs_url = Some("https://docs.browseros.com/troubleshooting/connection-issues".into());
    error
}

pub(super) fn protocol(message: &'static str) -> ChatError {
    let mut error = ChatError::new(ChatErrorCode::Unknown, "Invalid model response", message);
    error.retryable = false;
    error
}

/// A Rig failure on an HTTP wire. `url` is the destination, for the "could not connect" card,
/// which names only its host.
pub(super) fn provider(error: &ProviderError, url: Option<&Url>, secrets: &HeaderMap) -> ChatError {
    match error {
        ProviderError::Http(transport) => match transport.as_ref() {
            http_client::Error::Transient(source)
                if source
                    .downcast_ref::<reqwest_rig::Error>()
                    .is_some_and(reqwest_rig::Error::is_connect) =>
            {
                url.map_or_else(disconnected, connect)
            }
            _ => disconnected(),
        },
        // The stream ended before the provider's terminal event, or went silent for Rig's idle
        // limit (five minutes).
        ProviderError::Truncated | ProviderError::Stalled(_) => disconnected(),
        ProviderError::ProviderResponse(_) | ProviderError::Relayed(_) => report(error, secrets),
        _ => protocol("The provider sent a response the server could not read."),
    }
}

/// A provider's own refusal, by its status and body: an HTTP error reply, or an error event in
/// the stream (whose numeric code Rig reads as the status).
pub(super) fn report(error: &ProviderError, secrets: &HeaderMap) -> ChatError {
    let report = ErrorReport::from(error);
    let retry_after = report
        .retry_after(SystemTime::now())
        .map(|delay| delay.as_millis().div_ceil(1_000))
        .and_then(|seconds| u64::try_from(seconds).ok());
    upstream(
        report.http_status,
        retry_after,
        report.provider_response_body().unwrap_or_default(),
        secrets,
    )
}

pub(super) fn upstream(
    status: Option<u16>,
    retry_after: Option<u64>,
    body: &str,
    secrets: &HeaderMap,
) -> ChatError {
    let body_json = serde_json::from_str::<Value>(body).ok();
    let outer = body_json
        .as_ref()
        .and_then(|value| value.get("error").or(Some(value)));
    let raw = outer.and_then(|error| error.pointer("/metadata/raw"));
    let parsed_raw = raw
        .and_then(Value::as_str)
        .and_then(|text| serde_json::from_str::<Value>(text).ok());
    let inner = parsed_raw.as_ref().or(raw).or(outer);
    let inner = inner.map(|value| value.get("error").unwrap_or(value));
    let message = inner
        .and_then(error_message)
        .or_else(|| outer.and_then(error_message))
        .or_else(|| {
            if body_json.is_none() && !body.trim().is_empty() {
                Some(body)
            } else {
                None
            }
        });
    // Codex sends a named code. llama.cpp's router reports the same failure as a 400
    // with a model-specific message, so a generic 400 still keeps its normal classification.
    let model_not_found = [inner, outer]
        .into_iter()
        .flatten()
        .any(|error| error["code"] == "model_not_found")
        || (status == Some(400)
            && message
                .is_some_and(|text| text.starts_with("model '") && text.ends_with("' not found")));
    let clue = format!(
        "{body} {}",
        parsed_raw
            .as_ref()
            .map(Value::to_string)
            .unwrap_or_default()
    )
    .to_ascii_lowercase();
    use ChatErrorCode::*;
    let config_error = (
        ProviderConfig,
        "Check provider settings",
        "Check the model name and provider URL.",
    );
    let (code, title, fallback) = match status {
        Some(401 | 403) => (
            AuthFailed,
            "Authentication failed",
            "Check the provider credentials.",
        ),
        Some(429) => (
            RateLimited,
            "Rate limit reached",
            "The provider is rate limiting requests.",
        ),
        Some(413) => (
            ContextLength,
            "Context is too long",
            "The request exceeded the provider's input limit.",
        ),
        Some(500..=599) => (
            ProviderUnavailable,
            "Provider unavailable",
            "The provider could not complete the request.",
        ),
        _ if model_not_found => config_error,
        _ if [
            "context length",
            "context_length",
            "too many tokens",
            "maximum context",
            "prompt is too long",
        ]
        .iter()
        .any(|text| clue.contains(text)) =>
        {
            (
                ContextLength,
                "Context is too long",
                "The request exceeded the provider's input limit.",
            )
        }
        _ if [
            "content filter",
            "content_filter",
            "content policy",
            "content_policy",
            "guardrail_intervened",
        ]
        .iter()
        .any(|text| clue.contains(text)) =>
        {
            (
                ContentFilter,
                "Content filtered",
                "The provider blocked the request.",
            )
        }
        _ if [
            "authentication_error",
            "permission_error",
            "invalid_api_key",
        ]
        .iter()
        .any(|text| clue.contains(text)) =>
        {
            (
                AuthFailed,
                "Authentication failed",
                "Check the provider credentials.",
            )
        }
        _ if clue.contains("rate_limit") => (
            RateLimited,
            "Rate limit reached",
            "The provider is rate limiting requests.",
        ),
        _ if clue.contains("overloaded_error") || clue.contains("server_error") => (
            ProviderUnavailable,
            "Provider unavailable",
            "The provider could not complete the request.",
        ),
        Some(404) => config_error,
        _ => (
            Unknown,
            "Provider call failed",
            "The provider could not complete the request.",
        ),
    };
    let mut result = ChatError::new(
        code,
        title,
        message.map_or_else(|| fallback.into(), |text| scrub(text, secrets)),
    );
    if code == Unknown {
        result.retryable = false;
    }
    result.status_code = status;
    result.retry_after_seconds = retry_after;
    result.details = (!body.is_empty()).then(|| scrub(body, secrets));
    result
}

fn error_message(error: &Value) -> Option<&str> {
    error
        .as_str()
        .or_else(|| error.get("message")?.as_str())
        .filter(|s| !s.trim().is_empty())
}

// Providers sometimes return secrets observed upstream, in addition to the credentials we sent.
static CREDENTIALS: LazyLock<Option<Regex>> = LazyLock::new(|| {
    Regex::new(concat!(
        r#"(?ix) (?: bearer\s+[^\s"\\,]+ | \b(?:sk-|xox[baprs]-|AIza|ghp_|AKIA|ASIA)[a-z0-9_-]{8,} "#,
        r#"| \beyJ[a-z0-9_-]+\.[a-z0-9_-]+\.[a-z0-9_-]+ | (?:https?://)[^\s/@]+:[^\s/@]+@ "#,
        r#"| -----BEGIN[^\n]*PRIVATE\s+KEY-----[\s\S]*?-----END[^\n]*PRIVATE\s+KEY----- "#,
        r#"| (?:api[-_]?key|secret(?:_access_key)?|access[-_]?token|refresh[-_]?token|session[-_]?token|password|authorization) ["']?\s*[:=]\s*["']?[^\s"'\\,}]+ )"#,
    )).ok()
});

fn scrub(text: &str, secrets: &HeaderMap) -> String {
    let mut text = text.to_owned();
    for value in secrets.values().filter(|value| value.is_sensitive()) {
        if let Ok(raw) = value.to_str() {
            let raw = raw.strip_prefix("Bearer ").unwrap_or(raw);
            if !raw.is_empty() {
                text = text.replace(raw, "[redacted]");
                // A JSON error body can contain an escaped form of the same value.
                let quoted = Value::String(raw.into()).to_string();
                text = text.replace(quoted.trim_matches('"'), "[redacted]");
            }
        }
    }
    if let Some(regex) = CREDENTIALS.as_ref() {
        text = regex.replace_all(&text, "[redacted]").into_owned();
    }
    text.chars().take(20_000).collect()
}

pub(super) fn retry_delay(error: &ChatError, attempt: u32) -> Option<Duration> {
    match error.code {
        ChatErrorCode::RateLimited => error
            .retry_after_seconds
            .filter(|seconds| *seconds <= 5)
            .map(Duration::from_secs),
        ChatErrorCode::ConnectionFailed | ChatErrorCode::ProviderUnavailable => {
            Some(Duration::from_millis(250 * (1 << attempt)))
        }
        _ => None,
    }
}
