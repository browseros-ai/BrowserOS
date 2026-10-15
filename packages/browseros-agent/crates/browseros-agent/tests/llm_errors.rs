//! `llm::to_chat_error` / `redact` / `chat_error_text` against the TS classifier: every case of
//! `apps/server/tests/agent/chat-error.test.ts` (TS error classes mapped per the
//! `to_chat_error` docs), plus the rungs and quirks that file leaves out.

use browseros_agent::llm::{
    ChatError, ChatErrorCode, LlmConfigError, ModelError, ProviderKind, chat_error_text, redact,
    to_chat_error,
};
use serde_json::{Value, json};

/// `new APICallError({ message: 'upstream failed', ...overrides })`.
fn api(status: u16, message: &str) -> ModelError {
    ModelError::Api {
        status,
        message: message.to_owned(),
        response_body: None,
        response_headers: Vec::new(),
        data: None,
        retryable: matches!(status, 408 | 409 | 429) || status >= 500,
    }
}

fn with_body(error: ModelError, body: &str) -> ModelError {
    match error {
        ModelError::Api {
            status,
            message,
            response_headers,
            data,
            retryable,
            ..
        } => ModelError::Api {
            status,
            message,
            response_body: Some(body.to_owned()),
            response_headers,
            data,
            retryable,
        },
        other => other,
    }
}

fn classify(error: &ModelError) -> ChatError {
    to_chat_error(error, None)
}

#[test]
fn treats_a_429_as_retryable_rate_limiting() {
    let error = to_chat_error(&api(429, "slow down"), Some(ProviderKind::Anthropic));
    assert_eq!(error.code, ChatErrorCode::RateLimited);
    assert!(error.retryable);
    assert_eq!(error.provider.as_deref(), Some("anthropic"));
}

#[test]
fn reads_retry_after_off_the_response_headers() {
    let headers = |pairs: &[(&str, &str)]| ModelError::Api {
        status: 429,
        message: "upstream failed".to_owned(),
        response_body: None,
        response_headers: pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect(),
        data: None,
        retryable: true,
    };
    assert_eq!(
        classify(&headers(&[("retry-after", "30")])).retry_after_seconds,
        Some(30)
    );
    assert_eq!(
        classify(&headers(&[("retry-after", "1.2")])).retry_after_seconds,
        Some(2)
    );
    assert_eq!(
        classify(&headers(&[("retry-after-ms", "1500")])).retry_after_seconds,
        Some(2)
    );
    assert_eq!(
        classify(&headers(&[("retry-after", "0")])).retry_after_seconds,
        None
    );
    assert_eq!(
        classify(&headers(&[(
            "retry-after",
            "Wed, 21 Oct 2026 07:28:00 GMT"
        )]))
        .retry_after_seconds,
        None
    );
    // TS quirk: `retry-after` wins, but `retry-after-ms` being present divides it by 1000.
    assert_eq!(
        classify(&headers(&[
            ("retry-after", "30"),
            ("retry-after-ms", "30000")
        ]))
        .retry_after_seconds,
        Some(1)
    );
}

#[test]
fn classifies_401_and_403_as_auth_failure() {
    for status in [401, 403] {
        let error = classify(&api(status, "invalid api key"));
        assert_eq!(error.code, ChatErrorCode::AuthFailed, "{status}");
        assert_eq!(error.title, "Authentication failed");
        assert!(!error.retryable);
        assert_eq!(error.status_code, Some(status));
    }
}

#[test]
fn classifies_5xx_as_a_transient_provider_outage() {
    let error = classify(&api(503, "overloaded"));
    assert_eq!(error.code, ChatErrorCode::ProviderUnavailable);
    assert!(error.retryable);
}

#[test]
fn classifies_413_as_context_length_and_other_statuses_as_unknown() {
    let error = classify(&api(413, ""));
    assert_eq!(error.code, ChatErrorCode::ContextLength);
    assert_eq!(error.title, "Conversation too long");
    assert_eq!(
        error.message,
        "This conversation exceeded the model context window. Start a new chat or pick a model with a larger window."
    );

    let error = classify(&api(400, "bad request"));
    assert_eq!(error.code, ChatErrorCode::Unknown);
    assert_eq!(error.message, "bad request");
    assert!(!error.retryable);
    let error = classify(&api(409, "  "));
    assert_eq!(error.message, "The model request failed.");
    assert!(error.retryable, "409 is retryable in the AI SDK");
}

#[test]
fn the_last_retry_attempt_is_what_gets_classified() {
    // TS unwraps RetryError to its last error; the Rust loop surfaces the last attempt itself.
    let error = to_chat_error(&api(429, "still limited"), Some(ProviderKind::OpenRouter));
    assert_eq!(error.code, ChatErrorCode::RateLimited);
    assert_eq!(error.status_code, Some(429));
}

#[test]
fn classifies_provider_construction_failures_as_configuration_errors() {
    let error = to_chat_error(
        &ModelError::from(LlmConfigError(
            "Anthropic provider requires apiKey".to_owned(),
        )),
        Some(ProviderKind::Anthropic),
    );
    assert_eq!(error.code, ChatErrorCode::ProviderConfig);
    assert_eq!(error.message, "Anthropic provider requires apiKey");
    assert!(!error.retryable);

    for message in [
        "Ollama provider requires baseUrl",
        "ChatGPT requires OAuth authentication",
        "Not authenticated with ChatGPT. Please login first.",
        "model is required for openai provider",
    ] {
        assert_eq!(
            classify(&ModelError::Other(message.to_owned())).code,
            ChatErrorCode::ProviderConfig,
            "{message}"
        );
    }
    // Not a configuration message: TS falls through to unknown.
    assert_eq!(
        classify(&ModelError::from(LlmConfigError(
            "Unknown provider: nope".to_owned()
        )))
        .code,
        ChatErrorCode::Unknown
    );
}

#[test]
fn classifies_a_missing_key_error_from_the_sdk_class() {
    let error = classify(&ModelError::Config("OpenAI API key is missing".to_owned()));
    assert_eq!(error.code, ChatErrorCode::ProviderConfig);
    assert_eq!(error.title, "Provider not configured");
    assert!(!error.retryable);
}

#[test]
fn classifies_local_connection_failures() {
    let error = classify(&ModelError::Connection("fetch failed".to_owned()));
    assert_eq!(error.code, ChatErrorCode::ConnectionFailed);
    assert!(error.retryable);
    assert_eq!(
        error.docs_url.as_deref(),
        Some("https://docs.browseros.com/troubleshooting/connection-issues")
    );
    assert_eq!(
        error.message,
        "Could not reach the model provider. Check your network connection and any local provider URL."
    );
    // A plain error saying "fetch failed" takes the same rung.
    assert_eq!(
        classify(&ModelError::Other("TypeError: fetch failed".to_owned())).code,
        ChatErrorCode::ConnectionFailed
    );
}

#[test]
fn message_rungs_cover_context_length_and_content_filter() {
    for message in [
        "maximum context length is 8192",
        "context_length_exceeded",
        "Too many tokens",
    ] {
        let error = classify(&ModelError::Other(message.to_owned()));
        assert_eq!(error.code, ChatErrorCode::ContextLength, "{message}");
        assert!(!error.retryable);
    }
    for message in [
        "blocked by content filter",
        "content_filter",
        "violates our content policy",
    ] {
        let error = classify(&ModelError::Other(message.to_owned()));
        assert_eq!(error.code, ChatErrorCode::ContentFilter, "{message}");
        assert_eq!(error.title, "Content was rejected");
    }
}

#[test]
fn invalid_prompt_is_a_context_length_card() {
    let error = classify(&ModelError::InvalidPrompt(
        "user message at index 2 has no content".to_owned(),
    ));
    assert_eq!(error.code, ChatErrorCode::ContextLength);
    assert_eq!(error.title, "Message could not be sent");
    assert!(!error.retryable);
}

#[test]
fn falls_back_to_unknown_while_still_carrying_the_real_message() {
    let error = classify(&ModelError::Other("something exotic broke".to_owned()));
    assert_eq!(error.code, ChatErrorCode::Unknown);
    assert_eq!(error.title, "Something went wrong");
    assert_eq!(error.message, "something exotic broke");
    assert!(error.retryable);
    assert_eq!(error.details.as_deref(), Some("something exotic broke"));
}

#[test]
fn redacts_key_shaped_tokens_out_of_the_user_facing_message() {
    let key = format!("sk-{}", "b".repeat(40));
    let error = classify(&api(401, &format!("Incorrect API key provided: {key}")));
    assert!(error.message.contains("[REDACTED]"));
    assert!(!error.message.contains("bbbbbbbbbb"));
}

#[test]
fn redacts_key_shaped_tokens_out_of_details() {
    let key = format!("sk-{}", "a".repeat(40));
    let error = classify(&api(401, &format!("rejected key {key} for org")));
    let details = error.details.unwrap_or_default();
    assert!(details.contains("[REDACTED]"));
    assert!(!details.contains("aaaaaaaaaa"));
}

#[test]
fn carries_the_full_upstream_response_body_as_pretty_printed_details() {
    let body = json!({
        "error": {
            "code": "RATE_LIMITED",
            "message": "Provider returned error",
            "metadata": { "raw": "quota 0 of 100 for today" },
        }
    })
    .to_string();
    let error = to_chat_error(
        &with_body(api(429, "Provider returned error"), &body),
        Some(ProviderKind::OpenRouter),
    );
    let details = error.details.unwrap_or_default();
    assert!(details.contains("quota 0 of 100 for today"));
    assert!(details.contains('\n'));
    // `JSON.stringify(value, null, 2)`.
    assert!(
        details.starts_with("{\n  \"error\": {\n    \"code\": \"RATE_LIMITED\""),
        "{details}"
    );
}

#[test]
fn falls_back_to_structured_data_raw_when_there_is_no_response_body() {
    let error = ModelError::Api {
        status: 429,
        message: "Provider returned error".to_owned(),
        response_body: None,
        response_headers: Vec::new(),
        data: Some(Box::new(
            json!({ "code": "RATE_LIMITED", "raw": { "remaining": 0 } }),
        )),
        retryable: true,
    };
    let details = to_chat_error(&error, Some(ProviderKind::OpenRouter))
        .details
        .unwrap_or_default();
    assert_eq!(details, "{\n  \"remaining\": 0\n}");
}

#[test]
fn redacts_secrets_and_bounds_a_pathological_raw_body() {
    let secret = format!("sk-{}", "c".repeat(40));
    let body =
        json!({ "error": { "message": format!("boom {secret}"), "note": "x".repeat(30000) } })
            .to_string();
    let details = classify(&with_body(api(500, "upstream failed"), &body))
        .details
        .unwrap_or_default();
    assert!(details.contains("[REDACTED]"));
    assert!(!details.contains("cccccccccc"));
    assert!(details.encode_utf16().count() <= 20001);
    assert!(details.ends_with('…'));
}

#[test]
fn caps_details_in_utf16_units() {
    // 15 000 astral chars are 30 000 UTF-16 units: the cap lands at 10 000 chars.
    let error = classify(&ModelError::Other("😀".repeat(15_000)));
    let details = error.details.unwrap_or_default();
    assert_eq!(details.chars().count(), 10_001);
    assert!(details.ends_with('…'));
}

#[test]
fn surfaces_a_plain_error_message_as_copyable_details() {
    let error = to_chat_error(
        &ModelError::Other("Anthropic session expired. Please re-login.".to_owned()),
        Some(ProviderKind::Anthropic),
    );
    assert_eq!(error.code, ChatErrorCode::Unknown);
    assert!(
        error
            .details
            .unwrap_or_default()
            .contains("session expired")
    );
}

#[test]
fn redacts_a_jwt_sitting_inside_a_non_sensitive_field() {
    let jwt = [
        "eyJhbGciOiJIUzI1NiJ9",
        "eyJzdWIiOiIxMjM0NTY3ODkwIn0",
        "SflKxwRJSMeKKF2QT4fwpMeJf36POk6yJVadQssw5c",
    ]
    .join(".");
    let body = json!({ "error": { "message": format!("auth failed: {jwt}") } }).to_string();
    let details = classify(&with_body(api(500, "upstream failed"), &body))
        .details
        .unwrap_or_default();
    assert!(details.contains("[REDACTED]"));
    assert!(!details.contains("eyJhbGci"));
}

#[test]
fn redacts_a_sensitive_json_value_by_its_key_on_the_raw_text() {
    let body = json!({ "access_token": "ATsecret-value-9f8e7d6c5b4a" }).to_string();
    let details = classify(&with_body(api(401, "upstream failed"), &body))
        .details
        .unwrap_or_default();
    assert!(details.contains("[REDACTED]"));
    assert!(!details.contains("ATsecret-value"));
}

#[test]
fn redacts_url_userinfo_while_keeping_the_host() {
    let password = "hunter2".repeat(2);
    let error = classify(&ModelError::Other(format!(
        "proxy failed at https://svc:{password}@proxy.internal:8080/v1"
    )));
    let details = error.details.unwrap_or_default();
    assert!(details.contains("[REDACTED]"));
    assert!(!details.contains(&password));
    assert!(details.contains("https://[REDACTED]@proxy.internal"));
}

#[test]
fn redacts_vendor_token_shapes() {
    let google = format!("AIza{}", "A".repeat(35));
    let slack = format!("xoxb-{}", "1234567890-abcdefghijklmnop");
    let github = format!("ghp_{}", "A1".repeat(12));
    let aws = format!("AKIA{}", "ABCDEFGHIJKLMNOP");
    let pem = "-----BEGIN RSA PRIVATE KEY-----\nMIIabc\n-----END RSA PRIVATE KEY-----";
    let text = format!(
        "google {google} slack {slack} gh {github} aws {aws} key {pem} Bearer abcdefghijklmnop"
    );
    let redacted = redact(&text);
    for secret in [
        google.as_str(),
        slack.as_str(),
        github.as_str(),
        aws.as_str(),
        "MIIabc",
        "abcdefghijklmnop",
    ] {
        assert!(!redacted.contains(secret), "{secret} leaked: {redacted}");
    }
    // Word boundaries are ASCII (JS `\b`): a key glued to a letter is not a match.
    assert_eq!(
        redact(&format!("xsk-{}", "a".repeat(20))),
        format!("xsk-{}", "a".repeat(20))
    );
}

#[test]
fn redacts_the_password_value_by_key_case_insensitively() {
    assert_eq!(
        redact(r#"{"Password": "p@ss", "user": "bob", "X-Api-Key" : "k"}"#),
        r#"{"Password": "[REDACTED]", "user": "bob", "X-Api-Key" : "[REDACTED]"}"#
    );
}

#[test]
fn chat_error_text_round_trips_through_the_envelope() {
    let text = chat_error_text(&to_chat_error(
        &api(401, "The provider rejected your credentials"),
        Some(ProviderKind::OpenRouter),
    ));
    let parsed: Value = serde_json::from_str(&text).unwrap_or_default();
    assert_eq!(parsed["error"]["code"], "auth_failed");
    assert_eq!(parsed["error"]["retryable"], false);
    assert_eq!(parsed["error"]["provider"], "openrouter");
    assert_eq!(parsed["error"]["statusCode"], 401);
    // Undefined fields are omitted, as `JSON.stringify` drops them.
    assert!(parsed["error"].get("docsUrl").is_none());
    assert!(parsed["error"].get("retryAfterSeconds").is_none());

    let text = chat_error_text(&classify(&ModelError::Other("boom".to_owned())));
    assert!(serde_json::from_str::<Value>(&text).is_ok());
    assert_eq!(
        serde_json::from_str::<Value>(&text).unwrap_or_default(),
        json!({ "error": {
            "code": "unknown",
            "title": "Something went wrong",
            "message": "boom",
            "retryable": true,
            "details": "boom",
        } })
    );
}

#[test]
fn retryability_and_delays_follow_the_ai_sdk() {
    use browseros_agent::llm::{MAX_RETRIES, retry_delay};
    use std::time::Duration;
    assert_eq!(MAX_RETRIES, 2);
    assert!(api(429, "x").is_retryable());
    assert!(!api(401, "x").is_retryable());
    assert!(ModelError::Connection("reset".to_owned()).is_retryable());
    assert!(!ModelError::Other("x".to_owned()).is_retryable());
    assert_eq!(retry_delay(&api(500, "x"), 0), Duration::from_secs(2));
    assert_eq!(retry_delay(&api(500, "x"), 1), Duration::from_secs(4));
    let hinted = |name: &str, value: &str| ModelError::Api {
        status: 429,
        message: String::new(),
        response_body: None,
        response_headers: vec![(name.to_owned(), value.to_owned())],
        data: None,
        retryable: true,
    };
    assert_eq!(
        retry_delay(&hinted("retry-after", "1"), 0),
        Duration::from_secs(1)
    );
    assert_eq!(
        retry_delay(&hinted("retry-after-ms", "250"), 1),
        Duration::from_millis(250)
    );
    // Outside 0-60 s the hint is ignored.
    assert_eq!(
        retry_delay(&hinted("retry-after", "120"), 0),
        Duration::from_secs(2)
    );
}
