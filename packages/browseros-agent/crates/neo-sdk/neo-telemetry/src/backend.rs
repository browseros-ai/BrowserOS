//! What a [`crate::Telemetry`] handle records into.

use std::sync::Arc;

use crate::pipeline::Pipeline;

/// What a handle records into. Every facade call is one `match` on this, so the no-op costs a
/// branch: no allocation, no lock, no thread, no I/O.
#[derive(Clone, Debug)]
pub(crate) enum Backend {
    /// Telemetry off: the hosted embedding, and tests that do not look.
    Noop,
    /// The live OpenTelemetry pipeline: metrics, spans and their exporters.
    Otel(Arc<Pipeline>),
}
