//! Neo: a capture failure independent of the embedding host's HTTP and application errors.

/// Browser or transport failure, translated to the host's own error boundary by its adapter.
#[derive(Clone, Debug, thiserror::Error)]
#[error("{0}")]
pub struct CaptureFailure(String);

impl CaptureFailure {
    #[must_use]
    pub fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

pub type CaptureResult<T> = Result<T, CaptureFailure>;
