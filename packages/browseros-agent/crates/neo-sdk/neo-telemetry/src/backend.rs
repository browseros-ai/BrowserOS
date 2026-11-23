//! What a [`crate::Telemetry`] handle records into.

#[cfg(feature = "otel")]
use std::sync::Arc;

#[cfg(feature = "otel")]
use crate::pipeline::Pipeline;

/// What a handle records into. Every facade call is one `match` on this, so the no-op costs a
/// branch: no allocation, no lock, no thread, no I/O.
#[derive(Clone, Debug)]
pub(crate) enum Backend {
    /// Telemetry off: the hosted embedding, and tests that do not look.
    Noop,
    /// The live OpenTelemetry pipeline: metrics, spans and their exporters.
    #[cfg(feature = "otel")]
    Otel(Arc<Pipeline>),
}
