//! Neo: The optional upstream test runner's opaque transport into one QuickJS run. The servicing
//! script owns command decoding and native SDK object identity; this transport exposes no
//! browser operations and keeps no state on `Runtime` or its clones.

use futures_util::future::BoxFuture;

/// A transport owned by one [`crate::Runtime::run_with_test_host`] invocation. Only that run
/// receives `__neo_test_host`; page actions still use the SDK and the ordinary dispatcher.
///
/// Implementors own frame limits, queue bounds and backpressure. Receiving must return a
/// yielding future that is safe to drop. The runtime drops pending receives when the run
/// settles, times out or is cancelled; a receive must not spawn an unowned reader task.
pub trait TestHost: Send + Sync + 'static {
    /// Receives the next opaque frame, or `None` at EOF. JavaScript observes EOF as `null`.
    /// The method itself must return promptly, and its future must yield while it waits.
    ///
    /// # Errors
    ///
    /// Returns an opaque transport failure, which rejects the JavaScript receive promise.
    fn receive(&self) -> BoxFuture<'_, Result<Option<String>, TestHostError>>;

    /// Publishes one opaque frame synchronously. This must never block the realm thread,
    /// including on queue capacity or a receiver lock. Return an error on backpressure or
    /// transport failure; the runtime throws that error into the servicing script.
    ///
    /// Do not hold a receive-side lock while publishing. The host's independent I/O owner
    /// must be able to accept the reply while the servicing script waits for its next frame.
    ///
    /// # Errors
    ///
    /// Returns an opaque transport failure. The runtime does not interpret its message.
    fn reply(&self, frame: String) -> Result<(), TestHostError>;
}

/// An opaque transport failure, separate from browser calls and their recorded outcomes.
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct TestHostError(String);

impl TestHostError {
    /// Constructs an error without interpreting or classifying the host's message.
    #[must_use]
    pub fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}
