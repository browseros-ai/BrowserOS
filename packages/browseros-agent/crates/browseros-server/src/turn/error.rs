//! The extension's chat error envelope as a domain value.
//!
//! `model` and `agent` produce it; `turn` renders it into the `errorText` of the stream's `error`
//! chunk and `http::dto` renders it as a response body. Both renderings must agree, because the
//! extension's error card parses either one the same way.

use std::fmt;

/// The extension's error envelope. Produced by `model` and `agent`, rendered here into
/// `errorText` and by `http` as a response body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatError {
    pub code: ChatErrorCode,
    pub title: String,
    pub message: String,
    pub retryable: bool,
    pub provider: Option<String>,
    pub status_code: Option<u16>,
    pub docs_url: Option<String>,
    pub retry_after_seconds: Option<u64>,
    /// Upstream text with secrets scrubbed, at most 20,000 characters.
    pub details: Option<String>,
}

/// The eight codes the extension's error card knows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
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

impl ChatErrorCode {
    /// Every code, for tables and tests.
    pub const ALL: [ChatErrorCode; 8] = [
        ChatErrorCode::RateLimited,
        ChatErrorCode::AuthFailed,
        ChatErrorCode::ProviderConfig,
        ChatErrorCode::ContextLength,
        ChatErrorCode::ContentFilter,
        ChatErrorCode::ConnectionFailed,
        ChatErrorCode::ProviderUnavailable,
        ChatErrorCode::Unknown,
    ];

    /// The code as the envelope spells it (`rate_limited`, …).
    pub fn as_str(self) -> &'static str {
        match self {
            ChatErrorCode::RateLimited => "rate_limited",
            ChatErrorCode::AuthFailed => "auth_failed",
            ChatErrorCode::ProviderConfig => "provider_config",
            ChatErrorCode::ContextLength => "context_length",
            ChatErrorCode::ContentFilter => "content_filter",
            ChatErrorCode::ConnectionFailed => "connection_failed",
            ChatErrorCode::ProviderUnavailable => "provider_unavailable",
            ChatErrorCode::Unknown => "unknown",
        }
    }

    /// The inverse of `as_str`.
    pub fn parse(code: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|candidate| candidate.as_str() == code)
    }

    /// Whether "Try again" can help without the user changing anything.
    fn retryable_by_default(self) -> bool {
        matches!(
            self,
            ChatErrorCode::RateLimited
                | ChatErrorCode::ConnectionFailed
                | ChatErrorCode::ProviderUnavailable
                | ChatErrorCode::Unknown
        )
    }
}

impl ChatError {
    /// An error with only the required fields; `retryable` follows the code.
    pub fn new(code: ChatErrorCode, title: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code,
            title: title.into(),
            message: message.into(),
            retryable: code.retryable_by_default(),
            provider: None,
            status_code: None,
            docs_url: None,
            retry_after_seconds: None,
            details: None,
        }
    }

    /// What a body the skeleton has not filled in yet returns, so a caller sees a readable error
    /// card and never a panic.
    pub fn not_implemented(what: &str) -> Self {
        Self {
            retryable: false,
            ..Self::new(
                ChatErrorCode::Unknown,
                "Not implemented",
                format!("{what} is not implemented in this build of the server."),
            )
        }
    }
}

impl fmt::Display for ChatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code.as_str(), self.message)
    }
}

impl std::error::Error for ChatError {}
