//! Why telemetry could not start or flush.

use thiserror::Error;

/// Why telemetry could not start or flush. Never carries a secret: an exporter header's value
/// stays out of every message.
#[derive(Debug, Error)]
pub enum TelemetryError {
    /// A telemetry environment variable holds a value Neo cannot use. Startup fails on it.
    #[error("{name}: {reason}")]
    InvalidEnv {
        /// The variable, e.g. `CLAW_TELEMETRY_SITE_DETAIL`.
        name: &'static str,
        /// What is wrong with it, without echoing a secret value.
        reason: String,
    },
    /// The OpenTelemetry pipeline could not be built, or did not flush within its budget.
    #[error("telemetry {stage} failed: {reason}")]
    Pipeline {
        /// What failed: `start`, `flush`, `shutdown`.
        stage: &'static str,
        /// The SDK's or the exporter's own words.
        reason: String,
    },
}
