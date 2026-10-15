//! The `ChatError` envelope (`@browseros/shared/schemas/chat-error`): what a failed turn shows the
//! user. Its JSON is the `errorText` of `error`/`tool-*-error` chunks and the body of pre-stream
//! 500s. `to_chat_error` + `redact` are an exact port of `apps/server/src/agent/chat-error.ts`:
//! the same classification ladder, fallback sentences, redaction list and 20 000-unit cap.
//!
//! The server is a loopback process using the user's own credentials, so upstream text is shown
//! (classified, scrubbed and capped behind `details`) instead of masked.

use super::{ModelError, ProviderKind};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::LazyLock;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatError {
    pub code: ChatErrorCode,
    pub title: String,
    pub message: String,
    pub retryable: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status_code: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub docs_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_after_seconds: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChatErrorCode {
    RateLimited,
    AuthFailed,
    ProviderConfig,
    ContextLength,
    ContentFilter,
    ConnectionFailed,
    ProviderUnavailable,
    Unknown,
}

/// `details` carries the whole scrubbed upstream error so the card can show and copy it; bounded
/// so a pathological body cannot flood the wire. Measured in UTF-16 units, as TS `length`.
const DETAILS_MAX_LENGTH: usize = 20_000;

const CONNECTION_DOCS_URL: &str = "https://docs.browseros.com/troubleshooting/connection-issues";

const REDACTED: &str = "[REDACTED]";

/// Value-shaped secrets scrubbed out of upstream text before it is shown or copied, in TS order
/// (`SECRET_PATTERNS`). Bodies are arbitrary text, so these match on a value's shape rather than
/// its key. Skewed toward over-redaction: a missed token lands in a bug report, a false positive
/// only hides a value nobody needed. `(?-u:\b)` keeps JS's ASCII word boundary.
static SECRET_PATTERNS: LazyLock<Vec<(Regex, &'static str)>> = LazyLock::new(|| {
    let patterns: [(&str, &str); 13] = [
        // Provider API keys (OpenAI/Anthropic/Stripe sk-/pk-/rk-) and AWS access keys.
        (r"(?-u:\b)(?:sk|pk|rk)[-_][A-Za-z0-9_-]{12,}", REDACTED),
        (r"(?-u:\b)AKIA[0-9A-Z]{16}(?-u:\b)", REDACTED),
        // Bearer tokens and JWTs (three base64url segments).
        (r"(?i)(?-u:\b)Bearer\s+[A-Za-z0-9._-]{12,}", REDACTED),
        (
            r"(?-u:\b)eyJ[A-Za-z0-9_-]{6,}\.[A-Za-z0-9_-]{6,}\.[A-Za-z0-9_-]{6,}",
            REDACTED,
        ),
        // Vendor tokens: GitHub classic + fine-grained, GitLab, Google, Slack.
        (r"(?-u:\b)gh[pousr]_[A-Za-z0-9]{20,}", REDACTED),
        (r"(?-u:\b)github_pat_[A-Za-z0-9_]{22,}", REDACTED),
        (r"(?-u:\b)glpat-[A-Za-z0-9_-]{20}", REDACTED),
        (r"(?-u:\b)AIza[0-9A-Za-z_-]{35}", REDACTED),
        (r"(?-u:\b)ya29\.[0-9A-Za-z_-]{20,}", REDACTED),
        (r"(?-u:\b)xox[baprs]-[A-Za-z0-9-]{10,}", REDACTED),
        // PEM private key blocks.
        (
            r"(?s)-----BEGIN [A-Z ]*PRIVATE KEY-----.*?-----END [A-Z ]*PRIVATE KEY-----",
            REDACTED,
        ),
        // Credentials embedded in a URL (scheme://user:pass@host): keep the scheme and host.
        (
            r"(?i)([a-z][a-z0-9+.-]*://)[^\s:@/]+:[^\s:@/]+@",
            "${1}[REDACTED]@",
        ),
        // Sensitive JSON string values, matched on the raw text so no object parse is needed.
        (
            r#"(?i)("[^"]*(?:token|secret|password|passwd|api[_-]?key|authorization|credential)[^"]*"\s*:\s*)"[^"]*""#,
            r#"${1}"[REDACTED]""#,
        ),
    ];
    patterns
        .into_iter()
        .filter_map(|(pattern, replacement)| match Regex::new(pattern) {
            Ok(regex) => Some((regex, replacement)),
            Err(error) => {
                // Unreachable for these literals (covered by tests); never panic on a redaction path.
                tracing::error!(%error, pattern, "redaction pattern failed to compile");
                None
            }
        })
        .collect()
});

/// Value-shaped secret scrubbing of `chat-error.ts:51-77`, applied in order.
pub fn redact(text: &str) -> String {
    SECRET_PATTERNS
        .iter()
        .fold(text.to_owned(), |acc, (regex, replacement)| {
            regex.replace_all(&acc, *replacement).into_owned()
        })
}

fn utf16_len(text: &str) -> usize {
    text.encode_utf16().count()
}

/// The longest prefix of `text` within `max` UTF-16 units. JS `slice` may split a surrogate pair;
/// a Rust string cannot hold half of one, so the cut lands on the char boundary before it.
fn utf16_prefix(text: &str, max: usize) -> &str {
    let mut units = 0;
    for (index, ch) in text.char_indices() {
        units += ch.len_utf16();
        if units > max {
            return &text[..index];
        }
    }
    text
}

fn to_details(text: Option<&str>) -> Option<String> {
    let text = text.filter(|text| !text.is_empty())?;
    let redacted = redact(text);
    let redacted = redacted.trim();
    if redacted.is_empty() {
        return None;
    }
    Some(if utf16_len(redacted) > DETAILS_MAX_LENGTH {
        format!("{}…", utf16_prefix(redacted, DETAILS_MAX_LENGTH))
    } else {
        redacted.to_owned()
    })
}

/// Upstream text reaches the user, so it gets the same scrub as `details`.
fn safe_message(text: &str, fallback: &str) -> String {
    let redacted = redact(text);
    let redacted = redacted.trim();
    if redacted.is_empty() {
        fallback.to_owned()
    } else {
        redacted.to_owned()
    }
}

/// `upstreamDetails`: the response body (pretty-printed when it is JSON), then `data.raw`, then
/// the message. OpenRouter-style gateways hide the real reason in `metadata.raw`.
fn upstream_details(
    message: &str,
    response_body: Option<&str>,
    data: Option<&Value>,
) -> Option<String> {
    if let Some(body) = response_body.filter(|body| !body.is_empty()) {
        return match serde_json::from_str::<Value>(body) {
            Ok(value) => to_details(serde_json::to_string_pretty(&value).ok().as_deref()),
            Err(_) => to_details(Some(body)),
        };
    }
    if let Some(raw) = data.and_then(|data| data.get("raw")) {
        return to_details(serde_json::to_string_pretty(raw).ok().as_deref());
    }
    to_details(Some(message))
}

/// JS `Number(raw)`: surrounding whitespace allowed, empty is 0, anything else non-numeric is NaN.
fn js_number(raw: &str) -> f64 {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return 0.0;
    }
    trimmed.parse::<f64>().unwrap_or(f64::NAN)
}

/// `retryAfterSeconds`, quirk included: `retry-after` wins when both headers are present, but the
/// value is still divided by 1000 whenever `retry-after-ms` exists.
fn retry_after_seconds(headers: &[(String, String)]) -> Option<u64> {
    let header = |name: &str| {
        headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    };
    // `??` falls through only on a missing header; an empty one then ends the lookup.
    let raw = header("retry-after")
        .or_else(|| header("retry-after-ms"))
        .filter(|value| !value.is_empty())?;
    let value = js_number(raw);
    if !value.is_finite() || value <= 0.0 {
        return None;
    }
    let has_ms = header("retry-after-ms").is_some_and(|value| !value.is_empty());
    let seconds = if has_ms {
        (value / 1000.0).ceil()
    } else {
        value.ceil()
    };
    // Finite and positive here; `as` saturates.
    Some(seconds as u64)
}

struct ApiCall<'a> {
    status: u16,
    message: &'a str,
    response_body: Option<&'a str>,
    response_headers: &'a [(String, String)],
    data: Option<&'a Value>,
    retryable: bool,
}

fn from_api_call_error(error: ApiCall<'_>, provider: Option<String>) -> ChatError {
    let base = |code, title: &str, message: String, retryable| ChatError {
        code,
        title: title.to_owned(),
        message,
        retryable,
        provider: provider.clone(),
        status_code: Some(error.status),
        docs_url: None,
        retry_after_seconds: None,
        details: upstream_details(error.message, error.response_body, error.data),
    };
    match error.status {
        401 | 403 => base(
            ChatErrorCode::AuthFailed,
            "Authentication failed",
            safe_message(
                error.message,
                "The provider rejected your credentials. Check your API key in AI settings.",
            ),
            false,
        ),
        429 => ChatError {
            retry_after_seconds: retry_after_seconds(error.response_headers),
            ..base(
                ChatErrorCode::RateLimited,
                "Rate limited",
                safe_message(
                    error.message,
                    "The provider is rate limiting requests. Wait a moment and try again.",
                ),
                true,
            )
        },
        413 => base(
            ChatErrorCode::ContextLength,
            "Conversation too long",
            safe_message(
                error.message,
                "This conversation exceeded the model context window. Start a new chat or pick a model with a larger window.",
            ),
            false,
        ),
        status if status >= 500 => base(
            ChatErrorCode::ProviderUnavailable,
            "Provider unavailable",
            safe_message(
                error.message,
                "The model provider returned a server error. This is usually temporary.",
            ),
            true,
        ),
        _ => base(
            ChatErrorCode::Unknown,
            "Something went wrong",
            safe_message(error.message, "The model request failed."),
            error.retryable,
        ),
    }
}

fn plain(
    code: ChatErrorCode,
    title: &str,
    message: String,
    retryable: bool,
    provider: Option<String>,
    details: Option<String>,
) -> ChatError {
    ChatError {
        code,
        title: title.to_owned(),
        message,
        retryable,
        provider,
        status_code: None,
        docs_url: None,
        retry_after_seconds: None,
        details,
    }
}

fn connection_failed(message: &str, provider: Option<String>) -> ChatError {
    ChatError {
        docs_url: Some(CONNECTION_DOCS_URL.to_owned()),
        ..plain(
            ChatErrorCode::ConnectionFailed,
            "Connection failed",
            "Could not reach the model provider. Check your network connection and any local provider URL."
                .to_owned(),
            true,
            provider,
            to_details(Some(message)),
        )
    }
}

fn provider_config(message: &str, provider: Option<String>) -> ChatError {
    plain(
        ChatErrorCode::ProviderConfig,
        "Provider not configured",
        safe_message(message, "This provider is not fully configured."),
        false,
        provider,
        to_details(Some(message)),
    )
}

/// `fromMessage`: the last-resort rungs for errors with no structured evidence.
fn from_message(message: &str, provider: Option<String>) -> Option<ChatError> {
    let lower = message.to_lowercase();
    let any = |needles: &[&str]| needles.iter().any(|needle| lower.contains(needle));
    if any(&["failed to fetch", "fetch failed"]) {
        return Some(connection_failed(message, provider));
    }
    if any(&[
        "context length",
        "context_length_exceeded",
        "too many tokens",
        "maximum context",
    ]) {
        return Some(plain(
            ChatErrorCode::ContextLength,
            "Conversation too long",
            "This conversation exceeded the model context window. Start a new chat or pick a model with a larger window."
                .to_owned(),
            false,
            provider,
            to_details(Some(message)),
        ));
    }
    if any(&["content filter", "content_filter", "content policy"]) {
        return Some(plain(
            ChatErrorCode::ContentFilter,
            "Content was rejected",
            "The provider rejected the content being sent or generated. Try rephrasing and send it again."
                .to_owned(),
            false,
            provider,
            to_details(Some(message)),
        ));
    }
    if any(&[
        "requires apikey",
        "requires baseurl",
        "requires oauth",
        "not authenticated with",
        "is required for",
    ]) {
        return Some(provider_config(message, provider));
    }
    None
}

/// Classifier of `chat-error.ts:281-333`. Mapping of the TS error classes:
/// `APICallError` → [`ModelError::Api`]; `LoadAPIKeyError`/`NoSuchModelError` →
/// [`ModelError::Config`]; `InvalidPromptError` → [`ModelError::InvalidPrompt`]; a network failure
/// (TS matches `fetch failed`) → [`ModelError::Connection`]; any other thrown `Error` →
/// [`ModelError::Other`] (message rungs, then `unknown`). `RetryError` has no Rust counterpart:
/// the loop surfaces the last attempt's error itself.
pub fn to_chat_error(error: &ModelError, provider: Option<ProviderKind>) -> ChatError {
    let provider = provider.map(|kind| kind.as_str().to_owned());
    match error {
        ModelError::Api {
            status,
            message,
            response_body,
            response_headers,
            data,
            retryable,
        } => from_api_call_error(
            ApiCall {
                status: *status,
                message,
                response_body: response_body.as_deref(),
                response_headers,
                data: data.as_deref(),
                retryable: *retryable,
            },
            provider,
        ),
        ModelError::Config(message) => provider_config(message, provider),
        ModelError::InvalidPrompt(message) => plain(
            ChatErrorCode::ContextLength,
            "Message could not be sent",
            safe_message(message, "This message could not be sent."),
            false,
            provider,
            to_details(Some(message)),
        ),
        ModelError::Connection(message) => connection_failed(message, provider),
        ModelError::Other(_) | ModelError::Cancelled => {
            let message = error.to_string();
            if let Some(matched) = from_message(&message, provider.clone()) {
                return matched;
            }
            // The raw error is the only evidence for an unclassified failure: carry it scrubbed.
            plain(
                ChatErrorCode::Unknown,
                "Something went wrong",
                safe_message(&message, "An unexpected error occurred."),
                true,
                provider,
                to_details(Some(&message)),
            )
        }
    }
}

/// `serializeChatError`: `{"error":{…}}`.
pub fn chat_error_text(error: &ChatError) -> String {
    serde_json::json!({ "error": error }).to_string()
}
