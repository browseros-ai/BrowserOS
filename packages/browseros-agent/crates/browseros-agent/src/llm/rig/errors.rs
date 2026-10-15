//! rig `ProviderError` → [`ModelError`], shaped like the AI SDK errors the TS classifier saw: an
//! HTTP failure becomes `Api` with the message the SDK's failed-response handler extracts
//! (`error.message` of the JSON body, else the status text) and the SDK's retryable statuses;
//! OpenRouter keeps `openrouter-fetch.ts`'s `[code] message (raw)` envelope and `data`.

use crate::llm::{ModelError, ProviderKind};
use rig_core::ProviderError;
use rig_core::http_client;
use serde_json::{Value, json};

/// `APICallError`'s default `isRetryable`: 408, 409, 429 and 5xx.
pub(crate) fn retryable_status(status: u16) -> bool {
    matches!(status, 408 | 409 | 429) || status >= 500
}

fn status_text(status: u16) -> String {
    http::StatusCode::from_u16(status)
        .ok()
        .and_then(|status| status.canonical_reason())
        .unwrap_or_default()
        .to_owned()
}

/// JS truthiness of a JSON field, for the `parsed.error?.code` checks.
fn truthy(value: Option<&Value>) -> Option<&Value> {
    value.filter(|value| match value {
        Value::Null => false,
        Value::Bool(value) => *value,
        Value::Number(number) => number.as_f64().is_some_and(|number| number != 0.0),
        Value::String(text) => !text.is_empty(),
        Value::Array(_) | Value::Object(_) => true,
    })
}

fn js_string(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

/// The message an AI SDK `createJsonErrorResponseHandler` extracts: `error.message` when the body
/// parses as `{error:{message}}`, else the status text.
fn sdk_message(status: u16, body: &str) -> String {
    serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|json| {
            json.pointer("/error/message")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .unwrap_or_else(|| status_text(status))
}

/// `createOpenRouterCompatibleFetch`: message and structured `data` for a failed response.
pub(crate) fn openrouter_envelope(status: u16, body: &str) -> (String, Option<Value>) {
    let mut message = format!("HTTP {status}: {}", status_text(status));
    let mut data = None;
    if let Ok(parsed) = serde_json::from_str::<Value>(body) {
        let error = parsed.get("error");
        let code = truthy(error.and_then(|error| error.get("code")));
        let raw = error.and_then(|error| error.pointer("/metadata/raw"));
        if let Some(code) = code {
            let mut fields = serde_json::Map::new();
            fields.insert("code".to_owned(), code.clone());
            if let Some(raw) = raw {
                fields.insert("raw".to_owned(), raw.clone());
            }
            data = Some(Value::Object(fields));
        }
        if let Some(text) = truthy(error.and_then(|error| error.get("message"))) {
            message = js_string(text);
            if let Some(code) = code {
                message = format!("[{}] {message}", js_string(code));
            }
            if let Some(raw) = truthy(raw) {
                message.push_str(&format!(" ({})", json!(raw)));
            }
        }
    }
    (message, data)
}

fn headers_of(headers: Option<&http::HeaderMap>) -> Vec<(String, String)> {
    headers
        .into_iter()
        .flatten()
        .filter_map(|(name, value)| {
            value
                .to_str()
                .ok()
                .map(|value| (name.as_str().to_owned(), value.to_owned()))
        })
        .collect()
}

fn api_error(
    provider: ProviderKind,
    status: u16,
    body: &str,
    headers: Option<&http::HeaderMap>,
) -> ModelError {
    let (message, data) = if provider == ProviderKind::OpenRouter {
        openrouter_envelope(status, body)
    } else {
        (sdk_message(status, body), None)
    };
    ModelError::Api {
        status,
        message,
        response_body: Some(body.to_owned()).filter(|body| !body.is_empty()),
        response_headers: headers_of(headers),
        data: data.map(Box::new),
        retryable: retryable_status(status),
    }
}

/// The message of a failure the provider reported in-band (an SSE `error` event): its
/// `error.message`, or the body itself.
fn in_band_message(body: &str) -> String {
    serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|json| {
            json.pointer("/error/message")
                .or_else(|| json.get("message"))
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .unwrap_or_else(|| body.to_owned())
}

pub(crate) fn model_error(error: ProviderError, provider: ProviderKind) -> ModelError {
    if let Some(response) = error.provider_response() {
        return match response.status.filter(|status| !status.is_success()) {
            Some(status) => api_error(
                provider,
                status.as_u16(),
                &response.body,
                response.headers.as_ref(),
            ),
            None => ModelError::Other(in_band_message(&response.body)),
        };
    }
    match &error {
        ProviderError::Http(http) => match http.as_ref() {
            http_client::Error::InvalidStatusCodeWithDetails {
                status,
                body,
                headers,
            } => api_error(provider, status.as_u16(), body, Some(headers)),
            http_client::Error::Instance(_) | http_client::Error::StreamEnded => {
                ModelError::Connection(error.to_string())
            }
            _ => ModelError::Other(error.to_string()),
        },
        // Rejected before sending: an empty turn, an unencodable part (AI SDK InvalidPromptError).
        ProviderError::Request(_) => ModelError::InvalidPrompt(error.to_string()),
        ProviderError::UnsupportedOption(_) => ModelError::Config(error.to_string()),
        ProviderError::Relayed(report) => match report.http_status {
            Some(status) => api_error(
                provider,
                status,
                report.provider_response_body().unwrap_or_default(),
                report.provider_response_headers(),
            ),
            None => ModelError::Other(report.message.clone()),
        },
        _ => ModelError::Other(error.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn openrouter_envelope_matches_openrouter_fetch() {
        let body = r#"{"error":{"code":429,"message":"Provider returned error","metadata":{"raw":"quota 0 of 100"}}}"#;
        let (message, data) = openrouter_envelope(429, body);
        assert_eq!(
            message,
            r#"[429] Provider returned error ("quota 0 of 100")"#
        );
        assert_eq!(data, Some(json!({"code": 429, "raw": "quota 0 of 100"})));

        let (message, data) = openrouter_envelope(502, "<html>bad gateway</html>");
        assert_eq!(message, "HTTP 502: Bad Gateway");
        assert_eq!(data, None);

        let (message, data) =
            openrouter_envelope(401, r#"{"error":{"message":"User not found.","code":401}}"#);
        assert_eq!(message, "[401] User not found.");
        assert_eq!(data, Some(json!({"code": 401})));
    }

    #[test]
    fn retryable_statuses_follow_the_ai_sdk() {
        for status in [408, 409, 429, 500, 503] {
            assert!(retryable_status(status), "{status}");
        }
        for status in [400, 401, 403, 404, 413, 425] {
            assert!(!retryable_status(status), "{status}");
        }
    }
}
