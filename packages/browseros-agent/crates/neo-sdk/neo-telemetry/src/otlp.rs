//! The push door: OTLP/HTTP exporters, the resource and the sampler, from [`OtlpConfig`].
//!
//! Owned by the OTLP work. This is the seam with the pipeline (`pipeline.rs`): its provider
//! builders call [`push`], [`resource`] and [`sampler`] and plug in whatever they return, so
//! neither side edits the other's file. `None` from [`push`] is push off; a `None` signal inside
//! [`Push`] is that signal's push off.
//!
//! Notes for the implementation: set endpoint, protocol, headers, timeout and compression on
//! every exporter explicitly, because opentelemetry-otlp falls back to reading
//! `OTEL_EXPORTER_OTLP_*` from the process environment for anything left unset (and
//! `Resource::builder` reads `OTEL_SERVICE_NAME` / `OTEL_RESOURCE_ATTRIBUTES` the same way;
//! `Resource::builder_empty` does not). The processor and the reader export from the SDK's own
//! threads, outside any runtime: their client is the [`Transport`], never the server's runtime.

use opentelemetry_otlp::MetricExporter;
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::metrics::PeriodicReader;
use opentelemetry_sdk::trace::{BatchSpanProcessor, Sampler};

use crate::transport::Transport;
use crate::{OtlpConfig, TelemetryError};

/// Everything push needs, owned together: the transport lives exactly as long as the processor
/// and the reader that send through it, so dropping the pipeline cannot leave one without the
/// other.
#[derive(Debug)]
#[expect(
    dead_code,
    reason = "the providers take the processor and the reader once they are built (pipeline.rs)"
)]
pub(crate) struct Push {
    /// The trace push: a bounded, non-blocking batch processor over an OTLP/HTTP span
    /// exporter; `None` when no traces endpoint is configured.
    pub(crate) spans: Option<BatchSpanProcessor>,
    /// The metric push: a periodic reader (cumulative temporality) over an OTLP/HTTP metric
    /// exporter; `None` when no metrics endpoint is configured.
    pub(crate) metrics: Option<PeriodicReader<MetricExporter>>,
    /// The exporters' I/O thread and client.
    pub(crate) transport: Transport,
}

/// The push door for `config`, or `None` when no OTLP endpoint is set (the default).
#[expect(
    dead_code,
    clippy::unnecessary_wraps,
    reason = "the providers plug it in once they are built (pipeline.rs)"
)]
pub(crate) fn push(_config: &OtlpConfig) -> Result<Option<Push>, TelemetryError> {
    Ok(None)
}

/// The resource both providers share: `service.name` (`OTEL_SERVICE_NAME`, default
/// `browseros-neo`), then `OTEL_RESOURCE_ATTRIBUTES`, applied last so it wins.
#[expect(
    dead_code,
    reason = "both providers take it once they are built (pipeline.rs)"
)]
pub(crate) fn resource(_config: &OtlpConfig) -> Resource {
    Resource::builder_empty()
        .with_service_name("browseros-neo")
        .build()
}

/// The tracer provider's sampler (`OTEL_TRACES_SAMPLER` / `_ARG`, default
/// `parentbased_always_on`).
#[expect(
    dead_code,
    reason = "the tracer provider takes it once it is built (pipeline.rs)"
)]
pub(crate) fn sampler(_config: &OtlpConfig) -> Sampler {
    Sampler::ParentBased(Box::new(Sampler::AlwaysOn))
}
