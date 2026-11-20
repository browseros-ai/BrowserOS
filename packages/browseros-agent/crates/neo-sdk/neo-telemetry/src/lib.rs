//! Neo's telemetry: bounded OpenTelemetry metrics and the `neo.run` / `neo.op` spans, read by the
//! host API's Prometheus scrape and, when configured, pushed over OTLP/HTTP.
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
//!                                   └ Recording (test-support): every call kept in memory
//! ```
//!
//! The crate depends on no browser, CDP, JavaScript engine or SQLite crate, and the server's
//! dispatch code reaches OpenTelemetry only through this facade. Every metric label comes from a
//! closed vocabulary ([`label`], and the server's own tables in [`Vocabulary`]), so no caller can
//! grow a series set.

mod backend;
mod config;
mod instruments;
pub mod label;
mod op_settled;
mod open_tab_counts;
mod otlp;
mod otlp_protocol;
mod pipeline;
mod privacy;
mod prometheus;
#[cfg(feature = "test-support")]
mod recording;
mod run_span;
mod run_telemetry;
mod site_detail;
mod telemetry;
mod telemetry_config;
mod telemetry_error;
mod trace_context;
mod tracing_otel;
mod transport;
mod vocabulary;

pub use config::OtlpConfig;
pub use op_settled::{OpSettled, OpSite};
pub use open_tab_counts::{OpenTabCounts, OpenTabsSource};
pub use otlp_protocol::OtlpProtocol;
#[cfg(feature = "test-support")]
pub use recording::{Recording, TelemetryCall};
pub use run_span::{RunFacts, RunSpan, RunVerdict};
pub use run_telemetry::RunTelemetry;
pub use site_detail::SiteDetail;
pub use telemetry::Telemetry;
pub use telemetry_config::TelemetryConfig;
pub use telemetry_error::TelemetryError;
pub use trace_context::TraceContext;
pub use vocabulary::Vocabulary;
