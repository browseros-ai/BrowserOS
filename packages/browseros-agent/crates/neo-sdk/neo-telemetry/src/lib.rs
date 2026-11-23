//! Neo's telemetry: bounded OpenTelemetry metrics and its spans (`neo.run`, and one per op named
//! after the op), read by the host API's Prometheus scrape and, when configured, pushed over
//! OTLP/HTTP.
//!
//! ```text
//!  run service ─ start_run(RunFacts) ─▶ RunSpan ─┐ moved into the run's RunGuard, which
//!  janitor, private typing ─ internal_run ──────┘ finishes it once with the trace's verdict
//!  refused call ─ run_refused(ErrorLabel)
//!  RunCtx keeps RunSpan::ops() = RunTelemetry ─▶ op_settled(OpSettled) from the door's settle
//!  tab registry / tab watch ─ tab_opened, tab_closed, renderer_crashed, set_open_tabs
//!                                   │
//!                     Telemetry ────┤ Noop: one branch per hook, nothing else
//!                                   │ Otel: pipeline.rs ─ meter + tracer providers
//!                                   │   ├─ manual reader ─▶ prometheus.rs ─▶ render_prometheus (/v1/metrics)
//!                                   │   └─ otlp::push ─▶ OTLP/HTTP (protobuf | JSON) on its own thread
//!                                   └ in_memory (test-support): the same pipeline, spans kept for a Probe
//! ```
//!
//! Built without its default `otel` feature, the crate has no `Otel` backend: every handle is the
//! no-op, the scrape renders an empty exposition, and a configured OTLP push fails
//! [`Telemetry::new`]. The config reader still runs, so a bad variable fails the same way.
//!
//! The crate depends on no browser, CDP, JavaScript engine or SQLite crate, and the server's
//! dispatch code reaches OpenTelemetry only through this facade. Every metric label comes from a
//! closed vocabulary ([`label`], and the server's own tables in [`Vocabulary`]), so no caller can
//! grow a series set.

mod backend;
mod captcha_solve;
#[cfg_attr(
    not(feature = "otel"),
    expect(dead_code, reason = "without `otel` only the validation half is read")
)]
mod config;
#[cfg(feature = "otel")]
mod instruments;
pub mod label;
mod navigated;
mod op_settled;
mod open_tab_counts;
#[cfg(feature = "otel")]
mod otlp;
mod otlp_protocol;
#[cfg(feature = "otel")]
mod pipeline;
#[cfg(feature = "otel")]
mod privacy;
#[cfg(all(feature = "otel", feature = "test-support"))]
mod probe;
#[cfg(feature = "otel")]
mod prometheus;
mod run_span;
mod run_telemetry;
mod site_detail;
#[cfg(feature = "otel")]
mod spans;
mod telemetry;
mod telemetry_config;
mod telemetry_error;
mod trace_context;
#[cfg(feature = "otel")]
mod transport;
mod vocabulary;

pub use captcha_solve::{CaptchaSolve, CaptchaSpend};
pub use config::OtlpConfig;
pub use navigated::Navigated;
pub use op_settled::{OpCounting, OpSettled, OpSite};
pub use open_tab_counts::{OpenTabCounts, OpenTabsSource};
pub use otlp_protocol::OtlpProtocol;
#[cfg(all(feature = "otel", feature = "test-support"))]
pub use probe::{CapturedKind, CapturedSpan, Probe, SpanStatus, SpanValue};
pub use run_span::{RunFacts, RunSpan, RunVerdict};
pub use run_telemetry::RunTelemetry;
pub use site_detail::SiteDetail;
pub use telemetry::Telemetry;
pub use telemetry_config::TelemetryConfig;
pub use telemetry_error::TelemetryError;
pub use trace_context::TraceContext;
pub use vocabulary::Vocabulary;
