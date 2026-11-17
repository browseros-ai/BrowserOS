//! The crate's one low-level error: something below the hub failed. That is the capture
//! connection, a CDP command on it, or a host's [`SessionTabs`](crate::SessionTabs) lookup.
//! Hosts map it into their own error type. claw-server keeps the message as an internal
//! error, so its logs read as they did before the extraction.

/// A failure below the hub, carrying only its message.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct CaptureFailure {
    message: String,
}

impl CaptureFailure {
    /// A failure with this message.
    #[must_use]
    pub fn new(message: String) -> Self {
        Self { message }
    }

    /// The message, as it was created.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }
}

/// The result type of the capture ports and the link.
pub type CaptureResult<T> = Result<T, CaptureFailure>;
