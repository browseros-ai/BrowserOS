//! Converts transport and upstream failures into the panel's common envelope. Only scrubbed
//! provider text leaves the model layer, including when a service echoes its request headers.

use std::sync::LazyLock;
use std::time::{Duration, SystemTime};

use http::HeaderMap;
use regex::Regex;
use serde_json::Value;

use crate::turn::{ChatError, ChatErrorCode};

pub(super) fn config(message: impl Into<String>) -> ChatError {
    ChatError::new(
        ChatErrorCode::ProviderConfig,
        "Check provider settings",
        message,
    )
}

pub(super) fn disconnected() -> ChatError {
    let mut error = ChatError::new(
        ChatErrorCode::ConnectionFailed,
        "Could not connect",
        "The connection to the provider closed before the response finished.",
    );
    error.docs_url = Some("https://docs.browseros.com/troubleshooting/connection-issues".into());
    error
}

pub(super) fn protocol(message: &'static str) -> ChatError {
    let mut error = ChatError::new(ChatErrorCode::Unknown, "Invalid model response", message);
    error.retryable = false;
    error
}

pub(super) fn stream(event: &Value, secrets: &HeaderMap) -> ChatError {
    let error = event.get("error").unwrap_or(event);
    let status = error
        .get("code")
        .or_else(|| error.get("status"))
        .and_then(|value| {
            value
                .as_u64()
                .and_then(|n| u16::try_from(n).ok())
                .or_else(|| value.as_str()?.parse().ok())
        });
    upstream(status, None, &event.to_string(), secrets)
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
    let clue = format!(
        "{body} {}",
        parsed_raw
            .as_ref()
            .map(Value::to_string)
            .unwrap_or_default()
    )
    .to_ascii_lowercase();
    use ChatErrorCode::*;
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
        Some(404) => (
            ProviderConfig,
            "Check provider settings",
            "Check the model name and provider URL.",
        ),
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

pub(super) fn retry_after(headers: &HeaderMap) -> Option<u64> {
    let seconds = headers.get("retry-after").and_then(|value| {
        let value = value.to_str().ok()?;
        value.parse().ok().or_else(|| {
            // Parse the IMF-fixdate format without turning on any new Cargo features.
            retry_date(value).map(|date| {
                date.duration_since(SystemTime::now())
                    .unwrap_or_default()
                    .as_secs()
            })
        })
    });
    seconds.or_else(|| {
        let ms: u64 = headers.get("retry-after-ms")?.to_str().ok()?.parse().ok()?;
        Some(ms.div_ceil(1_000))
    })
}

fn retry_date(value: &str) -> Option<SystemTime> {
    aws_smithy_types::DateTime::from_str(value, aws_smithy_types::date_time::Format::HttpDate)
        .ok()
        .and_then(|value| SystemTime::try_from(value).ok())
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
