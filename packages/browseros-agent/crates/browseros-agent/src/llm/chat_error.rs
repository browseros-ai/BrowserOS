//! The `ChatError` envelope (`@browseros/shared/schemas/chat-error`): what a failed turn shows the
//! user. Its JSON is the `errorText` of `error`/`tool-*-error` chunks and the body of pre-stream
//! 500s. `to_chat_error` + `redact` port `apps/server/src/agent/chat-error.ts` (P5).

use super::{ModelError, ProviderKind};
use serde::{Deserialize, Serialize};

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

/// Classifier of `chat-error.ts:281-333`. S0 stub: everything is `unknown` with the raw text.
pub fn to_chat_error(error: &ModelError, provider: Option<ProviderKind>) -> ChatError {
    ChatError {
        code: ChatErrorCode::Unknown,
        title: "Something went wrong".to_owned(),
        message: redact(&error.to_string()),
        retryable: false,
        provider: provider.map(|kind| kind.as_str().to_owned()),
        status_code: None,
        docs_url: None,
        retry_after_seconds: None,
        details: None,
    }
}

/// `serializeChatError`: `{"error":{…}}`.
pub fn chat_error_text(error: &ChatError) -> String {
    serde_json::json!({ "error": error }).to_string()
}

/// Value-shaped secret scrubbing of `chat-error.ts:51-77`. S0 stub: the redaction list is P5's;
/// until then nothing is shown, so no unredacted upstream text can leak through the stub.
pub fn redact(text: &str) -> String {
    let _ = text;
    "[REDACTED]".to_owned()
}
