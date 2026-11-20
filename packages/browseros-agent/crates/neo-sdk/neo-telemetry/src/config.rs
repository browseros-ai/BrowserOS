//! The standard `OTEL_*` settings: the OTLP/HTTP push door, the resource and the sampler.
//!
//! Owned by the OTLP work. [`OtlpConfig`]'s shape is open: give it whatever fields the
//! exporters, the resource and the sampler need (`OTEL_EXPORTER_OTLP_ENDPOINT` and the
//! signal-specific endpoints, `OTEL_EXPORTER_OTLP_PROTOCOL` as [`crate::OtlpProtocol`],
//! `OTEL_EXPORTER_OTLP_HEADERS`, `OTEL_SERVICE_NAME`, `OTEL_RESOURCE_ATTRIBUTES`,
//! `OTEL_TRACES_SAMPLER` / `_ARG`, `OTEL_METRIC_EXPORT_INTERVAL`, `OTEL_BSP_SCHEDULE_DELAY`), as
//! long as [`Default`] stays "push off" and `Debug` never prints a header value.
//! `TelemetryConfig::from_env` calls [`OtlpConfig::from_env`] and nothing else here.

use crate::TelemetryError;

/// The standard OpenTelemetry environment, parsed. [`Default`] is push off: no exporter starts.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OtlpConfig {}

impl OtlpConfig {
    /// Reads the `OTEL_*` variables through `lookup`. A malformed value is an error, which
    /// fails Neo's startup.
    pub fn from_env(_lookup: &dyn Fn(&str) -> Option<String>) -> Result<Self, TelemetryError> {
        Ok(Self::default())
    }
}
